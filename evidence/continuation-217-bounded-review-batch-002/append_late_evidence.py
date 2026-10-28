#!/usr/bin/env python3
"""Append late-created CONTINUATION-217 receipts without rewriting indexes."""

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


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, separator, name = line.partition("  ")
        if separator:
            result[name] = value
    return result


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()


if not LOCAL.is_file() or not WORKTREE_INDEX.is_file() or not PROJECT_INDEX.is_file():
    raise SystemExit("append-only evidence indexes must already exist")

files = []
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence directory: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED_NAMES:
        files.append(path)
files.sort(key=lambda path: path.relative_to(ROOT).as_posix())
local_previous = latest(LOCAL)
changed = [
    (digest(path.read_bytes()), path.relative_to(ROOT).as_posix())
    for path in files
    if local_previous.get(path.relative_to(ROOT).as_posix()) != digest(path.read_bytes())
]
if not changed:
    raise SystemExit("no new or changed evidence to append")

append(LOCAL, changed)
local_digest = digest(LOCAL.read_bytes())
worktree_entries = [
    (value, f"evidence/{ROOT.name}/{name}") for value, name in changed
]
worktree_entries.append((local_digest, f"evidence/{ROOT.name}/SHA256SUMS"))
project_entries = [
    (value, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}")
    for value, name in changed
]
project_entries.append((
    local_digest,
    f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS",
))
append(WORKTREE_INDEX, worktree_entries)
append(PROJECT_INDEX, project_entries)
append(PROJECT_INDEX, [(
    digest(WORKTREE_INDEX.read_bytes()),
    ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
)])
print(json.dumps({"appended_files": len(changed), "local_manifest_rebound": True}, sort_keys=True))
