#!/usr/bin/env python3
"""Append refreshed CONT-226 receipts without changing prior manifest bytes."""

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
PREFIX = ROOT / "revision-002-prefix-before.json"
DOCS = ("docs/architecture.md", "docs/checkpoint.md")
REFRESHED_FILES = (
    "append_evidence_revision.py",
    "revision-002-prefix-before.json",
    "forgejo-v17-action-source-review.md",
    "local-run.md",
    "commands.txt",
    "workflow-run-process.log",
    "workflow-job-process.log",
    "source-sha256.txt",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()


if PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("revision prefix receipt already exists")
for path in (LOCAL, WORKTREE_INDEX, PROJECT_INDEX):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"append-only manifest is unavailable: {path}")

before = {
    "local": {"line_count": len(LOCAL.read_bytes().splitlines()), "sha256": digest(LOCAL.read_bytes())},
    "worktree": {
        "line_count": len(WORKTREE_INDEX.read_bytes().splitlines()),
        "sha256": digest(WORKTREE_INDEX.read_bytes()),
    },
    "project": {
        "line_count": len(PROJECT_INDEX.read_bytes().splitlines()),
        "sha256": digest(PROJECT_INDEX.read_bytes()),
    },
}
PREFIX.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

local_entries = []
for name in REFRESHED_FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"refreshed evidence file is unavailable: {name}")
    local_entries.append((digest(path.read_bytes()), name))
append(LOCAL, local_entries)
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

for key, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = index.read_bytes().splitlines(keepends=True)
    prefix = b"".join(lines[: before[key]["line_count"]])
    if digest(prefix) != before[key]["sha256"]:
        raise SystemExit(f"{key} index prefix changed")

summary = {
    "appended_local_entries": len(local_entries),
    "appended_worktree_entries": len(worktree_entries),
    "appended_project_entries": len(project_entries) + 1,
    "previous_prefixes_preserved": True,
}
(ROOT / "revision-002-manifest-check.txt").write_text(
    json.dumps(summary, sort_keys=True, indent=2) + "\n", encoding="utf-8"
)
print(json.dumps(summary, sort_keys=True))
