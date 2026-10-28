#!/usr/bin/env python3
"""Verify CONTINUATION-109 evidence and append-only manifest prefixes."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text())
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"

def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        digest, separator, rel = line.partition("  ")
        if separator:
            result[rel] = digest
    return result

def prefix_ok(path: Path, key: str) -> bool:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    return (
        len(lines) >= expected["line_count"]
        and hashlib.sha256(b"".join(lines[: expected["line_count"]])).hexdigest()
        == expected["sha256"]
    )

for path, key in ((WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    if not prefix_ok(path, key):
        raise SystemExit(f"{key} append-only prefix changed")
work_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
files = sorted(path for path in ROOT.rglob("*") if path.is_file() and path.name != "SHA256SUMS")
for path in files:
    rel = path.relative_to(WORKTREE).as_posix()
    project_rel = (Path(".runtime/webhook-005-workspace") / rel).as_posix()
    if work_latest.get(rel) != sha(path) or project_latest.get(project_rel) != sha(path):
        raise SystemExit(f"evidence index mismatch: {rel}")
for rel in ("docs/architecture.md", "docs/checkpoint.md"):
    path = WORKTREE / rel
    project_rel = (Path(".runtime/webhook-005-workspace") / rel).as_posix()
    if work_latest.get(rel) != sha(path) or project_latest.get(project_rel) != sha(path):
        raise SystemExit(f"documentation index mismatch: {rel}")
if project_latest.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORKTREE_INDEX):
    raise SystemExit("project index does not track current worktree index")
local_count = 0
for line in (ROOT / "SHA256SUMS").read_text(encoding="ascii").splitlines():
    expected, rel = line.split("  ", 1)
    path = (ROOT / rel).resolve(strict=True)
    if ROOT.resolve() not in path.parents or sha(path) != expected:
        raise SystemExit(f"local manifest mismatch: {rel}")
    local_count += 1
print(json.dumps({"worktree_prefix_preserved": True, "project_prefix_preserved": True,
                  "local_entries_verified": local_count, "evidence_files_verified": len(files),
                  "documentation_files_verified": 2, "global_indexes_current": True}, sort_keys=True))
