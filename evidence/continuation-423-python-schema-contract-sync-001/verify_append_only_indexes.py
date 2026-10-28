#!/usr/bin/env python3
"""Verify local evidence hashes and append-only parent index links."""

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXCLUDED = {"manifest-check.txt", "global-manifest-check.txt"}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_entries(path: Path) -> list[tuple[str, str]]:
    result = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            value, name = line.split("  ", 1)
            result.append((value, name))
    return result


if not LOCAL.is_file() or LOCAL.is_symlink():
    raise SystemExit("local evidence manifest missing or unsafe")
local_entries = read_entries(LOCAL)
for value, name in local_entries:
    path = ROOT / name
    if not path.is_file() or path.is_symlink() or digest(path.read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
indexed = {name for _, name in local_entries}
expected = {
    path.relative_to(ROOT).as_posix()
    for path in ROOT.rglob("*")
    if path.is_file() and path.name not in EXCLUDED and path != LOCAL
}
if indexed != expected:
    raise SystemExit(
        f"local evidence coverage mismatch: missing={sorted(expected-indexed)} extra={sorted(indexed-expected)}"
    )

prefix = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
for key, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = path.read_bytes().splitlines(keepends=True)
    count = prefix[key]["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != prefix[key]["sha256"]:
        raise SystemExit(f"{key} index prefix changed")

work_entries = read_entries(WORKTREE_INDEX)
project_entries = read_entries(PROJECT_INDEX)
work_names = {name: value for value, name in work_entries}
project_names = {name: value for value, name in project_entries}
for value, name in local_entries:
    work_name = f"evidence/{ROOT.name}/{name}"
    project_name = f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}"
    if work_names.get(work_name) != value or project_names.get(project_name) != value:
        raise SystemExit(f"parent index missing/mismatched: {name}")
local_hash = digest(LOCAL.read_bytes())
if work_names.get(f"evidence/{ROOT.name}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree index does not bind local manifest")
if project_names.get(f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS") != local_hash:
    raise SystemExit("project index does not bind local manifest")
for relative in (
    "docs/architecture.md",
    "docs/checkpoint.md",
    "docs/implementation-plan.md",
    "docs/refusal-evidence-matrix.md",
):
    value = digest((WORKTREE / relative).read_bytes())
    if work_names.get(relative) != value or project_names.get(
        f".runtime/webhook-005-workspace/{relative}"
    ) != value:
        raise SystemExit(f"parent index missing/mismatched document: {relative}")
if project_entries[-1] != (
    digest(WORKTREE_INDEX.read_bytes()),
    ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
):
    raise SystemExit("project index does not bind the current worktree index")
print(
    json.dumps(
        {
            "local_evidence_files_verified": len(local_entries),
            "parent_binds_current_worktree_index": True,
            "project_prefix_preserved": True,
            "worktree_prefix_preserved": True,
        },
        sort_keys=True,
    )
)
