#!/usr/bin/env python3
"""Verify CONTINUATION-110 local evidence hashes and preserved index prefixes."""
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
        raise SystemExit(f"{key} manifest is shorter than its saved prefix")
    if sha_bytes(b"".join(lines[: expected["line_count"]])) != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix changed")


for index, key in ((WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(index, key)

local_entries = {}
for line in (ROOT / "SHA256SUMS").read_text(encoding="ascii").splitlines():
    digest, rel = line.split("  ", 1)
    path = (ROOT / rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path) != digest:
        raise SystemExit(f"local evidence hash mismatch: {rel}")
    local_entries[path.relative_to(WORKTREE).as_posix()] = digest

actual_files = {
    path.resolve().relative_to(WORKTREE).as_posix()
    for path in ROOT.iterdir()
    if path.is_file() and path.name != "SHA256SUMS"
}
if actual_files != set(local_entries):
    raise SystemExit("local manifest does not cover the complete evidence directory")

work_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for rel, digest in local_entries.items():
    if work_latest.get(rel) != digest:
        raise SystemExit(f"worktree evidence index mismatch: {rel}")
    project_rel = (Path(".runtime/webhook-005-workspace") / rel).as_posix()
    if project_latest.get(project_rel) != digest:
        raise SystemExit(f"project evidence index mismatch: {rel}")

local_manifest_rel = (ROOT / "SHA256SUMS").relative_to(WORKTREE).as_posix()
local_manifest_digest = sha(ROOT / "SHA256SUMS")
if work_latest.get(local_manifest_rel) != local_manifest_digest:
    raise SystemExit("worktree index does not hash the local evidence manifest")
local_manifest_project_rel = (
    Path(".runtime/webhook-005-workspace") / local_manifest_rel
).as_posix()
if project_latest.get(local_manifest_project_rel) != local_manifest_digest:
    raise SystemExit("project index does not hash the local evidence manifest")

for rel in ("docs/architecture.md", "docs/checkpoint.md"):
    digest = sha(WORKTREE / rel)
    if work_latest.get(rel) != digest:
        raise SystemExit(f"worktree documentation index mismatch: {rel}")
    project_rel = (Path(".runtime/webhook-005-workspace") / rel).as_posix()
    if project_latest.get(project_rel) != digest:
        raise SystemExit(f"project documentation index mismatch: {rel}")

worktree_rel = ".runtime/webhook-005-workspace/evidence/SHA256SUMS"
if project_latest.get(worktree_rel) != sha(WORKTREE_INDEX):
    raise SystemExit("project index does not hash the current worktree evidence index")

print(json.dumps({"local_entries_verified": len(local_entries),
                  "worktree_prefix_preserved": True,
                  "project_prefix_preserved": True,
                  "worktree_line_count": len(WORKTREE_INDEX.read_text().splitlines()),
                  "project_line_count": len(PROJECT_INDEX.read_text().splitlines()),
                  "global_indexes_current": True}, sort_keys=True))
