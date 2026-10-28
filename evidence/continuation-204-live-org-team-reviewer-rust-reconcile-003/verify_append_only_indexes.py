#!/usr/bin/env python3
"""Verify this receipt, its local hashes, and append-only index bindings."""

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
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


for index, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(index, key)

actual: dict[str, str] = {}
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence tree: {path.relative_to(ROOT)}")
    if path.is_file() and path != LOCAL and path.name not in EXCLUDED_NAMES:
        actual[path.relative_to(ROOT).as_posix()] = sha(path)
if actual != latest(LOCAL):
    raise SystemExit("local evidence manifest is incomplete or stale")

work_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for rel, digest in actual.items():
    work_rel = (ROOT / rel).relative_to(WORKTREE).as_posix()
    project_rel = (Path(".runtime/webhook-005-workspace") / work_rel).as_posix()
    if work_latest.get(work_rel) != digest or project_latest.get(project_rel) != digest:
        raise SystemExit(f"evidence hash mismatch: {rel}")
local_rel = LOCAL.relative_to(WORKTREE).as_posix()
if (
    work_latest.get(local_rel) != sha(LOCAL)
    or project_latest.get(f".runtime/webhook-005-workspace/{local_rel}") != sha(LOCAL)
):
    raise SystemExit("global indexes do not bind current local receipt manifest")

for rel in (
    "docs/architecture.md",
    "docs/checkpoint.md",
    "docs/refusal-evidence-matrix.md",
    "docs/source-inventory.md",
    "docs/implementation-plan.md",
):
    digest = sha(WORKTREE / rel)
    project_rel = f".runtime/webhook-005-workspace/{rel}"
    if work_latest.get(rel) != digest or project_latest.get(project_rel) != digest:
        raise SystemExit(f"documentation hash mismatch: {rel}")
if project_latest.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORKTREE_INDEX):
    raise SystemExit("parent index does not bind current worktree evidence index")

print(json.dumps({
    "local_evidence_files_verified": len(actual),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
    "parent_binds_current_worktree_index": True,
}, sort_keys=True))
