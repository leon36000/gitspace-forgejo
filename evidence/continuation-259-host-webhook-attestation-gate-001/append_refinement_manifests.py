#!/usr/bin/env python3
"""Append the final manifest-verification receipt without rewriting prior rows."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
NAMES = [
    "append_refinement_manifests.py",
    "commands.txt",
    "manifest-verification-final.json",
]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, rows: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in rows:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


for name in NAMES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"missing refinement evidence: {name}")

rows = [(digest((ROOT / name).read_bytes()), name) for name in NAMES]
append(LOCAL, rows)
local_hash = digest(LOCAL.read_bytes())
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_rows = [(value, f"{evidence_prefix}/{name}") for value, name in rows]
worktree_rows.append((local_hash, f"{evidence_prefix}/SHA256SUMS"))
append(WORKTREE_INDEX, worktree_rows)
project_rows = [(value, f"{project_prefix}/{evidence_prefix}/{name}") for value, name in rows]
project_rows.append((local_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS"))
append(PROJECT_INDEX, project_rows)
append(
    PROJECT_INDEX,
    [(digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS")],
)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    old = PREFIX[label]
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(old["line_count"])
    if len(lines) < count or digest(b"".join(lines[:count])) != old["sha256"]:
        raise SystemExit(f"historical {label} prefix changed")

print(json.dumps({"status": "refinement_appended", "files": len(rows), "historical_prefixes_preserved": True}, sort_keys=True))
