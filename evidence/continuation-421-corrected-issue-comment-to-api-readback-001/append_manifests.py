#!/usr/bin/env python3
"""Append CONTINUATION-421 evidence to the local and parent hash indexes."""

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
PREFIX = ROOT / "manifest-prefix-before.json"
CHECK = ROOT / "manifest-check.txt"
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
DOCS = (
    "docs/architecture.md",
    "docs/checkpoint.md",
    "docs/implementation-plan.md",
    "docs/refusal-evidence-matrix.md",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()


if any(path.exists() or path.is_symlink() for path in (LOCAL, PREFIX, CHECK)):
    raise SystemExit("refusing to overwrite a pre-existing CONTINUATION-421 artifact")
for index in (WORKTREE_INDEX, PROJECT_INDEX):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"append-only index unavailable: {index}")
    if index.read_bytes() and not index.read_bytes().endswith(b"\n"):
        raise SystemExit(f"append-only index has no final newline: {index}")

before = {}
prefix_bytes = {}
for key, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = index.read_bytes()
    before[key] = {"line_count": len(data.splitlines()), "sha256": digest(data)}
    prefix_bytes[key] = data
PREFIX.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

files = []
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence directory: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED:
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
for relative in DOCS:
    value = digest((WORKTREE / relative).read_bytes())
    worktree_entries.append((value, relative))
    project_entries.append((value, f".runtime/webhook-005-workspace/{relative}"))

append(WORKTREE_INDEX, worktree_entries)
append(PROJECT_INDEX, project_entries)
append(
    PROJECT_INDEX,
    [
        (
            digest(WORKTREE_INDEX.read_bytes()),
            ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
        )
    ],
)

for key, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = index.read_bytes()
    old_count = before[key]["line_count"]
    old_prefix = b"".join(data.splitlines(keepends=True)[:old_count])
    if digest(old_prefix) != before[key]["sha256"]:
        raise SystemExit(f"append-only prefix changed for {key}")
for value, name in local_entries:
    if digest((ROOT / name).read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
if digest(LOCAL.read_bytes()) != local_digest:
    raise SystemExit("local evidence index hash mismatch")

CHECK.write_text(
    "CONTINUATION-421 local evidence hashes: PASS\n"
    "worktree and project append-only prefixes: PASS\n"
    "current architecture/checkpoint/plan/matrix hashes linked: PASS\n"
    f"local_evidence_files={len(local_entries)}\n",
    encoding="utf-8",
)
print(
    json.dumps(
        {
            "local_evidence_files": len(local_entries),
            "appended_worktree_entries": len(worktree_entries),
            "appended_project_entries": len(project_entries) + 1,
            "append_only_prefixes": "verified",
        },
        sort_keys=True,
    )
)
