#!/usr/bin/env python3
"""Append the final CONTINUATION-219 checkpoint revision without rewriting indexes."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
SCRIPT_RELATIVE = Path("evidence") / ROOT.name / "append_checkpoint_late_binding.py"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, separator, name = line.partition("  ")
        if separator:
            values[name] = value
    return values


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))


relative = SCRIPT_RELATIVE.as_posix()
if latest(LOCAL).get("append_checkpoint_late_binding.py"):
    raise SystemExit("late binding already appended")
if any(index.is_symlink() or not index.is_file() for index in (LOCAL, WORKTREE_INDEX, PROJECT_INDEX)):
    raise SystemExit("one or more append-only indexes are unavailable")

script_hash = digest((ROOT / "append_checkpoint_late_binding.py").read_bytes())
append(LOCAL, [(script_hash, "append_checkpoint_late_binding.py")])
local_hash = digest(LOCAL.read_bytes())
checkpoint_hash = digest((WORKTREE / "docs/checkpoint.md").read_bytes())

worktree_entries = [
    (script_hash, relative),
    (local_hash, f"evidence/{ROOT.name}/SHA256SUMS"),
    (checkpoint_hash, "docs/checkpoint.md"),
]
project_entries = [
    (script_hash, f".runtime/webhook-005-workspace/{relative}"),
    (local_hash, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS"),
    (checkpoint_hash, ".runtime/webhook-005-workspace/docs/checkpoint.md"),
]
append(WORKTREE_INDEX, worktree_entries)
append(PROJECT_INDEX, project_entries)
append(PROJECT_INDEX, [(
    digest(WORKTREE_INDEX.read_bytes()),
    ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
)])
print(json.dumps({
    "appended_local_entry": True,
    "appended_worktree_entries": len(worktree_entries),
    "appended_project_entries": len(project_entries) + 1,
    "checkpoint_sha256": checkpoint_hash,
}, sort_keys=True))
