#!/usr/bin/env python3
"""Rebuild this local manifest after the append command's output was redirected into it."""

from __future__ import annotations

import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXCLUDED_NAMES = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


files = []
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence directory: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED_NAMES:
        files.append(path)
files.sort(key=lambda path: path.relative_to(ROOT).as_posix())
local_entries = [
    (digest(path.read_bytes()), path.relative_to(ROOT).as_posix()) for path in files
]
LOCAL.write_text(
    "".join(f"{value}  {name}\n" for value, name in local_entries), encoding="utf-8"
)
local_digest = digest(LOCAL.read_bytes())
worktree_entries = [
    (value, f"evidence/{ROOT.name}/{name}") for value, name in local_entries
]
worktree_entries.append((local_digest, f"evidence/{ROOT.name}/SHA256SUMS"))
project_entries = [
    (value, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}")
    for value, name in local_entries
]
project_entries.append(
    (local_digest, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS")
)
with WORKTREE_INDEX.open("ab") as stream:
    for value, name in worktree_entries:
        stream.write(f"{value}  {name}\n".encode())
project_entries.append((digest(WORKTREE_INDEX.read_bytes()), ".runtime/webhook-005-workspace/evidence/SHA256SUMS"))
with PROJECT_INDEX.open("ab") as stream:
    for value, name in project_entries:
        stream.write(f"{value}  {name}\n".encode())
print(f"rebuilt {ROOT.name}: {len(local_entries)} files; outer indexes appended")
