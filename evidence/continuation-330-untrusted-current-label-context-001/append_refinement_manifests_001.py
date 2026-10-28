#!/usr/bin/env python3
"""Append final CONTINUATION-330 hashes after the Python consumer and edge-case tests."""

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
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
DOCS = ("docs/architecture.md", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md", "docs/source-inventory.md")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()


prefix = {}
for key, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    raw = index.read_bytes()
    prefix[key] = {"line_count": len(raw.splitlines()), "sha256": digest(raw)}
(ROOT / "manifest-prefix-after-001.json").write_text(
    json.dumps(prefix, sort_keys=True, indent=2) + "\n", encoding="utf-8"
)

files = []
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence directory: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED:
        files.append(path)
files.sort(key=lambda path: path.relative_to(ROOT).as_posix())
local = [(digest(path.read_bytes()), path.relative_to(ROOT).as_posix()) for path in files]
LOCAL.write_text("".join(f"{value}  {name}\n" for value, name in local), encoding="utf-8")

work = [(value, f"evidence/{ROOT.name}/{name}") for value, name in local]
work.append((digest(LOCAL.read_bytes()), f"evidence/{ROOT.name}/SHA256SUMS"))
project = [
    (value, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}")
    for value, name in local
]
project.append(
    (digest(LOCAL.read_bytes()), f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS")
)
for relative in DOCS:
    value = digest((WORKTREE / relative).read_bytes())
    work.append((value, relative))
    project.append((value, f".runtime/webhook-005-workspace/{relative}"))
append(WORKTREE_INDEX, work)
append(PROJECT_INDEX, project)
append(
    PROJECT_INDEX,
    [(digest(WORKTREE_INDEX.read_bytes()), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")],
)
print(json.dumps({"refreshed_local_files": len(local), "append_only": True}, sort_keys=True))
