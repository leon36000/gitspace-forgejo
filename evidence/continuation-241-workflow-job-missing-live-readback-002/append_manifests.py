#!/usr/bin/env python3
"""Append this report bundle and verify global append-only prefixes."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL_INDEX = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX_RECEIPT = ROOT / "manifest-prefix-before.json"
FILES = (
    "append_manifests.py",
    "manifest-prefix-before.json",
    "local-run.md",
    "commands.txt",
    "source-review.txt",
    "tool-versions.txt",
    "source-sha256.txt",
    "cargo-metadata-license.json",
    "cargo-license-guard.log",
    "all-targets-no-run.log",
    "all-targets-no-run.exit",
    "rustfmt.log",
    "diff-check.log",
    "python-ast-parse.log",
    "run-manifest-check.log",
    "rustfmt-final.log",
    "diff-check-final.log",
    "source-doc-whitespace.log",
    "provenance-script-hash-check.log",
    "verification-exit-codes.txt",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


def latest_entries(path: Path) -> dict[str, str]:
    entries: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and name:
            entries[name] = value
    return entries


for name, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"{name} append-only index unavailable: {index}")
if LOCAL_INDEX.exists() and LOCAL_INDEX.stat().st_size:
    raise SystemExit("local index is nonempty; refusing to rewrite or duplicate it")
LOCAL_INDEX.touch(exist_ok=True)

current = {
    "local": {
        "line_count": len(LOCAL_INDEX.read_bytes().splitlines()),
        "sha256": digest(LOCAL_INDEX.read_bytes()),
    },
    "worktree": {
        "line_count": len(WORKTREE_INDEX.read_bytes().splitlines()),
        "sha256": digest(WORKTREE_INDEX.read_bytes()),
    },
    "project": {
        "line_count": len(PROJECT_INDEX.read_bytes().splitlines()),
        "sha256": digest(PROJECT_INDEX.read_bytes()),
    },
}
if PREFIX_RECEIPT.is_symlink():
    raise SystemExit("prefix receipt is a symlink; refusing to append")
if PREFIX_RECEIPT.exists():
    before = json.loads(PREFIX_RECEIPT.read_text(encoding="utf-8"))
    if before != current or before["local"]["line_count"] != 0:
        raise SystemExit("prefix receipt does not match the untouched indexes")
else:
    before = current
    PREFIX_RECEIPT.write_text(
        json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )

local_entries = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"evidence file unavailable: {name}")
    local_entries.append((digest(path.read_bytes()), name))
append(LOCAL_INDEX, local_entries)
local_digest = digest(LOCAL_INDEX.read_bytes())

worktree_entries = [
    (value, f"evidence/{ROOT.name}/{name}") for value, name in local_entries
]
worktree_entries.append((local_digest, f"evidence/{ROOT.name}/SHA256SUMS"))
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
project_entries = [
    (value, f"{project_prefix}/evidence/{ROOT.name}/{name}")
    for value, name in local_entries
]
project_entries.append(
    (local_digest, f"{project_prefix}/evidence/{ROOT.name}/SHA256SUMS")
)
checkpoint_hash = digest((WORKTREE / "docs/checkpoint.md").read_bytes())
worktree_entries.append((checkpoint_hash, "docs/checkpoint.md"))
project_entries.append((checkpoint_hash, f"{project_prefix}/docs/checkpoint.md"))
append(WORKTREE_INDEX, worktree_entries)
project_entries.append(
    (digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS")
)
append(PROJECT_INDEX, project_entries)

for key, index in (
    ("local", LOCAL_INDEX),
    ("worktree", WORKTREE_INDEX),
    ("project", PROJECT_INDEX),
):
    lines = index.read_bytes().splitlines(keepends=True)
    prefix = b"".join(lines[: before[key]["line_count"]])
    if digest(prefix) != before[key]["sha256"]:
        raise SystemExit(f"{key} index prefix changed")

expected_worktree = {name: value for value, name in worktree_entries}
expected_project = {name: value for value, name in project_entries}
for key, index, expected in (
    ("worktree", WORKTREE_INDEX, expected_worktree),
    ("project", PROJECT_INDEX, expected_project),
):
    latest = latest_entries(index)
    for name, value in expected.items():
        if latest.get(name) != value:
            raise SystemExit(f"{key} latest entry mismatch for {name}")

print(
    json.dumps(
        {
            "local_entries_appended": len(local_entries),
            "worktree_entries_appended": len(worktree_entries),
            "project_entries_appended": len(project_entries),
            "local_worktree_project_prefixes_preserved": True,
            "latest_global_entries_match": True,
        },
        sort_keys=True,
    )
)
