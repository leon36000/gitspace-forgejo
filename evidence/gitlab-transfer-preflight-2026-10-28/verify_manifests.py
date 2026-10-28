#!/usr/bin/env python3
"""Verify CONTINUATION-347 files and append-only manifest prefixes."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def entries(data: bytes) -> list[tuple[str, str]]:
    return [tuple(line.split(b"  ", 1)[i].decode() for i in (0, 1)) for line in data.splitlines()]


def verify_file(root: Path, value: str, name: str) -> None:
    path = root / name
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"manifest target unavailable or symlinked: {path}")
    if digest(path.read_bytes()) != value:
        raise SystemExit(f"manifest digest mismatch: {path}")


prefix = json.loads((ROOT / "manifest-prefix-before.json").read_text())
worktree_bytes = (WORKTREE / "evidence/SHA256SUMS").read_bytes()
project_bytes = (PROJECT / "evidence/SHA256SUMS").read_bytes()
worktree_lines = worktree_bytes.splitlines(keepends=True)
project_lines = project_bytes.splitlines(keepends=True)
for key, data, lines, index in (
    ("worktree", worktree_bytes, worktree_lines, WORKTREE / "evidence/SHA256SUMS"),
    ("project", project_bytes, project_lines, PROJECT / "evidence/SHA256SUMS"),
):
    before = prefix[key]
    prior = b"".join(lines[: before["line_count"]])
    if digest(prior) != before["sha256"]:
        raise SystemExit(f"append-only prefix changed: {index}")

local_index = entries((ROOT / "SHA256SUMS").read_bytes())
for value, name in local_index:
    verify_file(ROOT, value, name)
new_worktree = entries(b"".join(worktree_lines[prefix["worktree"]["line_count"] :]))
new_project = entries(b"".join(project_lines[prefix["project"]["line_count"] :]))
for value, name in new_worktree:
    verify_file(WORKTREE, value, name)
for value, name in new_project[:-1]:
    verify_file(PROJECT, value, name)
if (
    not new_project
    or new_project[-1][1]
    != ".runtime/webhook-005-workspace/evidence/SHA256SUMS"
):
    raise SystemExit("project index does not end with the worktree manifest receipt")
if new_project[-1][0] != digest((WORKTREE / "evidence/SHA256SUMS").read_bytes()):
    raise SystemExit("project index worktree manifest digest mismatch")
print(
    json.dumps(
        {
            "local_evidence_entries_verified": len(local_index),
            "worktree_append_entries_verified": len(new_worktree),
            "project_append_entries_verified": len(new_project),
            "worktree_prefix_preserved": True,
            "project_prefix_preserved": True,
        },
        sort_keys=True,
    )
)
