#!/usr/bin/env python3
"""Append this receipt and its admission source evidence to all local indexes."""

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXTERNAL = WORKTREE / "evidence/mission-012-private-runtime-sandbox-001"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
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
        raise SystemExit(f"{key} append-only prefix changed")


def regular_files(directory: Path) -> list[Path]:
    paths = sorted(path for path in directory.rglob("*") if path.is_file())
    for path in directory.rglob("*"):
        if path.is_symlink():
            raise SystemExit(f"symlink in evidence tree: {path.relative_to(directory)}")
    return [path for path in paths if path.name not in EXCLUDED_NAMES and path != directory / "SHA256SUMS"]


for index, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    if index.is_symlink() or not index.is_file():
        raise SystemExit(f"{key} manifest is missing or redirected")
    check_prefix(index, key)

local_latest = latest(LOCAL)
new_local_lines = []
for path in regular_files(ROOT):
    rel = path.relative_to(ROOT).as_posix()
    digest = sha(path)
    if local_latest.get(rel) != digest:
        new_local_lines.append(f"{digest}  {rel}\n")
if new_local_lines:
    with LOCAL.open("ab") as stream:
        stream.writelines(line.encode("ascii") for line in new_local_lines)
        stream.flush()

external_manifest = EXTERNAL / "SHA256SUMS"
external_actual = {
    path.relative_to(EXTERNAL).as_posix(): sha(path)
    for path in regular_files(EXTERNAL)
}
if external_actual != latest(external_manifest):
    raise SystemExit("source admission evidence manifest is incomplete or stale")

records: dict[str, tuple[str, str, str]] = {}
for rel, digest in latest(LOCAL).items():
    path = (ROOT / rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path) != digest:
        raise SystemExit(f"local evidence hash mismatch: {rel}")
    work_rel = path.relative_to(WORKTREE).as_posix()
    project_rel = (Path(".runtime/webhook-005-workspace") / work_rel).as_posix()
    records[work_rel] = (digest, work_rel, project_rel)

local_manifest_rel = LOCAL.relative_to(WORKTREE).as_posix()
records[local_manifest_rel] = (
    sha(LOCAL),
    local_manifest_rel,
    (Path(".runtime/webhook-005-workspace") / local_manifest_rel).as_posix(),
)

for rel, digest in sorted(external_actual.items()):
    path = EXTERNAL / rel
    work_rel = path.relative_to(WORKTREE).as_posix()
    project_rel = (Path(".runtime/webhook-005-workspace") / work_rel).as_posix()
    records[work_rel] = (digest, work_rel, project_rel)
external_manifest_rel = external_manifest.relative_to(WORKTREE).as_posix()
records[external_manifest_rel] = (
    sha(external_manifest),
    external_manifest_rel,
    (Path(".runtime/webhook-005-workspace") / external_manifest_rel).as_posix(),
)

for rel in ("docs/architecture.md", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md"):
    digest = sha(WORKTREE / rel)
    records[rel] = (digest, rel, f".runtime/webhook-005-workspace/{rel}")

work_latest = latest(WORKTREE_INDEX)
work_lines = [
    f"{digest}  {work_rel}\n"
    for digest, work_rel, _ in records.values()
    if work_latest.get(work_rel) != digest
]
if work_lines:
    with WORKTREE_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(work_lines)
        stream.flush()

project_records = [(digest, project_rel) for digest, _, project_rel in records.values()]
project_records.append(
    (sha(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")
)
project_latest = latest(PROJECT_INDEX)
project_lines = [
    f"{digest}  {project_rel}\n"
    for digest, project_rel in project_records
    if project_latest.get(project_rel) != digest
]
if project_lines:
    with PROJECT_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(project_lines)
        stream.flush()

actual_local = {path.relative_to(ROOT).as_posix() for path in regular_files(ROOT)}
if actual_local != set(latest(LOCAL)):
    raise SystemExit("local evidence manifest does not cover every regular evidence file")
work_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for digest, work_rel, project_rel in records.values():
    if work_latest.get(work_rel) != digest or project_latest.get(project_rel) != digest:
        raise SystemExit(f"global manifest mismatch: {work_rel}")
if project_latest.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORKTREE_INDEX):
    raise SystemExit("parent index does not bind current worktree evidence index")

print(json.dumps({
    "local_entries_appended": len(new_local_lines),
    "worktree_entries_appended": len(work_lines),
    "project_entries_appended": len(project_lines),
    "local_files_verified": len(actual_local),
    "external_admission_files_indexed": len(external_actual) + 1,
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
}, sort_keys=True))
