#!/usr/bin/env python3
"""Append CONTINUATION-357 evidence to local and parent indexes."""

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
EXCLUDED_NAMES = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
DOCS = (
    "docs/checkpoint.md",
    "docs/refusal-evidence-matrix.md",
    "docs/architecture.md",
    "docs/source-inventory.md",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()


if LOCAL.exists() or LOCAL.is_symlink():
    raise SystemExit("local evidence index already exists")
for index in (WORKTREE_INDEX, PROJECT_INDEX):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"append-only index unavailable: {index}")

prefix = {}
for key, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = index.read_bytes()
    prefix[key] = {"line_count": len(data.splitlines()), "sha256": digest(data)}
(ROOT / "manifest-prefix-before.json").write_text(
    json.dumps(prefix, sort_keys=True, indent=2) + "\n", encoding="utf-8"
)

files = []
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence directory: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED_NAMES:
        files.append(path)
files.sort(key=lambda path: path.relative_to(ROOT).as_posix())
local_entries = [(digest(path.read_bytes()), path.relative_to(ROOT).as_posix()) for path in files]
LOCAL.write_text("".join(f"{value}  {name}\n" for value, name in local_entries), encoding="utf-8")
local_digest = digest(LOCAL.read_bytes())

worktree_entries = [(value, f"evidence/{ROOT.name}/{name}") for value, name in local_entries]
worktree_entries.append((local_digest, f"evidence/{ROOT.name}/SHA256SUMS"))
project_entries = [
    (value, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}")
    for value, name in local_entries
]
project_entries.append(
    (local_digest, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS")
)
for relative in DOCS:
    value = digest((WORKTREE / relative).read_bytes())
    worktree_entries.append((value, relative))
    project_entries.append((value, f".runtime/webhook-005-workspace/{relative}"))

append(WORKTREE_INDEX, worktree_entries)
append(PROJECT_INDEX, project_entries)
append(
    PROJECT_INDEX,
    [(digest(WORKTREE_INDEX.read_bytes()), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")],
)
print(
    json.dumps(
        {
            "local_evidence_files": len(local_entries),
            "appended_worktree_entries": len(worktree_entries),
            "appended_project_entries": len(project_entries) + 1,
        },
        sort_keys=True,
    )
)
