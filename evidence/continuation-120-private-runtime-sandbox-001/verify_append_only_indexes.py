#!/usr/bin/env python3
"""Verify this receipt, linked admission evidence, and append-only index bindings."""

import hashlib
import json
from pathlib import Path
import stat


ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
EXTERNAL = WORKTREE / "evidence/mission-012-private-runtime-sandbox-001"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXCLUDED_NAMES = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha(path: Path) -> str:
    return sha_bytes(path.read_bytes())


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, sep, name = line.partition("  ")
        if sep:
            result[name] = digest
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if (
        len(lines) < expected["line_count"]
        or sha_bytes(b"".join(lines[: expected["line_count"]])) != expected["sha256"]
    ):
        raise SystemExit(f"{key} append-only prefix mismatch")


def actual_files(directory: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for path in directory.rglob("*"):
        if path.is_symlink():
            raise SystemExit(f"symlink in evidence tree: {path.relative_to(directory)}")
        if path.is_file() and path.name not in EXCLUDED_NAMES and path != directory / "SHA256SUMS":
            result[path.relative_to(directory).as_posix()] = sha(path)
    return result


for index, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(index, key)

local_actual = actual_files(ROOT)
if local_actual != latest(LOCAL):
    raise SystemExit("local evidence manifest is incomplete or stale")
external_actual = actual_files(EXTERNAL)
if external_actual != latest(EXTERNAL / "SHA256SUMS"):
    raise SystemExit("source admission evidence manifest is incomplete or stale")

work_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for directory, actual in ((ROOT, local_actual), (EXTERNAL, external_actual)):
    for rel, digest in actual.items():
        work_rel = (directory / rel).relative_to(WORKTREE).as_posix()
        project_rel = f".runtime/webhook-005-workspace/{work_rel}"
        if work_latest.get(work_rel) != digest or project_latest.get(project_rel) != digest:
            raise SystemExit(f"evidence hash mismatch: {work_rel}")
    manifest_rel = (directory / "SHA256SUMS").relative_to(WORKTREE).as_posix()
    digest = sha(directory / "SHA256SUMS")
    if (
        work_latest.get(manifest_rel) != digest
        or project_latest.get(f".runtime/webhook-005-workspace/{manifest_rel}") != digest
    ):
        raise SystemExit(f"evidence index hash mismatch: {manifest_rel}")

for rel in ("docs/architecture.md", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md"):
    digest = sha(WORKTREE / rel)
    project_rel = f".runtime/webhook-005-workspace/{rel}"
    if work_latest.get(rel) != digest or project_latest.get(project_rel) != digest:
        raise SystemExit(f"documentation hash mismatch: {rel}")
if project_latest.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORKTREE_INDEX):
    raise SystemExit("parent index does not bind current worktree evidence index")

linkage = json.loads((ROOT / "admission-linkage.json").read_text(encoding="utf-8"))
agent = json.loads((ROOT / "agent-run-summary.json").read_text(encoding="utf-8"))
admission_dir = WORKTREE / "evidence/mission-012-private-runtime-sandbox-001"
admission = json.loads((admission_dir / "result.json").read_text(encoding="utf-8"))
admission_dto = admission["mission"]["admission"]
delivery = admission["webhook"]["event"]["delivery_id"]
runtime = Path("/tmp/mission-012-private-runtime-sandbox-001/project/.runtime")
source_path = runtime / linkage["admission_source_file_basename"]
try:
    source_mode = source_path.lstat().st_mode
except OSError:
    source_mode = 0
source_is_regular = stat.S_ISREG(source_mode) and not source_path.is_symlink()
source_record = json.loads(source_path.read_text(encoding="utf-8")) if source_is_regular else {}
expected_labels = [
    "admission_source",
    "forgejo_read_token",
    "host_admission",
    "invalid_forgejo_read_token",
    "mission_journal",
    "webhook_inbox",
]
checks = agent.get("sandbox_checks", {})
if (
    linkage.get("delivery_id") != delivery
    or agent.get("admission_source_delivery_id") != delivery
    or linkage.get("requirement_sha256") != admission["mission"]["requirement_sha256"]
    or linkage.get("requirement_sha256") != admission_dto.get("requirement_sha256")
    or linkage.get("base_commit") != admission_dto.get("base_commit")
    or linkage.get("admission_source_sha256") != agent.get("admission_source_sha256")
    or linkage.get("webhook_inbox_sha256") != agent.get("webhook_inbox_sha256")
    or linkage.get("candidate_file_sha256") != agent.get("candidate_file_sha256")
    or linkage.get("host_binary_sha256") != agent.get("host_binary_sha256")
    or linkage.get("forgejo_binary_sha256") != admission["forgejo"]["binary_sha256"]
    or linkage.get("admission_result_sha256") != sha(admission_dir / "result.json")
    or linkage.get("admission_manifest_sha256") != sha(admission_dir / "SHA256SUMS")
    or source_path.name != Path(admission_dto["host_admission_source_file"]).name
    or not source_is_regular
    or sha(source_path) != agent.get("admission_source_sha256")
    or source_record.get("delivery_id") != delivery
    or source_record.get("requirement_sha256") != linkage.get("requirement_sha256")
    or sha(runtime / "host-admission.json") != admission["mission"]["admission_file_sha256"]
    or sha(runtime / "forgejo-webhook-inbox.jsonl") != agent.get("webhook_inbox_sha256")
    or checks.get("private_runtime_files_not_visible") != expected_labels
    or linkage.get("private_runtime_files_not_visible") != expected_labels
    or agent.get("model_invoked_during_run") is not False
    or agent.get("agent_adapter") != "local-test-adapter"
):
    raise SystemExit("admission-to-agent evidence linkage is inconsistent")

print(json.dumps({
    "local_evidence_files_verified": len(local_actual),
    "admission_evidence_files_verified": len(external_actual),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
    "parent_binds_current_worktree_index": True,
    "admission_to_agent_linkage_verified": True,
}, sort_keys=True))
