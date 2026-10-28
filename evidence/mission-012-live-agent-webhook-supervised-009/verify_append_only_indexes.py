#!/usr/bin/env python3
"""Verify local evidence coverage, current hashes and append-only index prefixes."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text())


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha(path: Path) -> str:
    return sha_bytes(path.read_bytes())


def latest(path: Path) -> dict[str, str]:
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, separator, rel = line.partition("  ")
        if separator:
            result[rel] = digest
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(lines) < expected["line_count"]:
        raise SystemExit(f"{key} index is shorter than the saved prefix")
    if sha_bytes(b"".join(lines[: expected["line_count"]])) != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix changed")


for index, key in ((WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(index, key)

local_index = ROOT / "SHA256SUMS"
check_prefix(local_index, "local")
local_entries = {}
local_all_lines = local_index.read_text(encoding="ascii").splitlines()
if any("  " not in line for line in local_all_lines):
    raise SystemExit("local manifest contains a malformed line")
for rel, digest in latest(local_index).items():
    path = (ROOT / rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path) != digest:
        raise SystemExit(f"local evidence checksum mismatch: {rel}")
    local_entries[path.relative_to(WORKTREE).as_posix()] = digest

actual = {
    path.resolve().relative_to(WORKTREE).as_posix()
    for path in ROOT.iterdir() if path.is_file() and path.name != "SHA256SUMS"
}
if actual != set(local_entries):
    raise SystemExit("local manifest does not cover the complete evidence directory")

work_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for rel, digest in local_entries.items():
    if work_latest.get(rel) != digest:
        raise SystemExit(f"worktree index mismatch: {rel}")
    project_rel = (Path(".runtime/webhook-005-workspace") / rel).as_posix()
    if project_latest.get(project_rel) != digest:
        raise SystemExit(f"project index mismatch: {rel}")

local_work_rel = local_index.relative_to(WORKTREE).as_posix()
local_project_rel = (Path(".runtime/webhook-005-workspace") / local_work_rel).as_posix()
local_digest = sha(local_index)
if work_latest.get(local_work_rel) != local_digest or project_latest.get(local_project_rel) != local_digest:
    raise SystemExit("a global index does not hash the local evidence manifest")

for rel in ("docs/architecture.md", "docs/checkpoint.md"):
    digest = sha(WORKTREE / rel)
    project_rel = (Path(".runtime/webhook-005-workspace") / rel).as_posix()
    if work_latest.get(rel) != digest or project_latest.get(project_rel) != digest:
        raise SystemExit(f"documentation index mismatch: {rel}")

worktree_project_rel = ".runtime/webhook-005-workspace/evidence/SHA256SUMS"
if project_latest.get(worktree_project_rel) != sha(WORKTREE_INDEX):
    raise SystemExit("project index does not hash the current worktree index")

print(json.dumps({
    "local_evidence_entries_verified": len(local_entries),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
    "worktree_lines": len(WORKTREE_INDEX.read_text().splitlines()),
    "project_lines": len(PROJECT_INDEX.read_text().splitlines()),
}, sort_keys=True))
