#!/usr/bin/env python3
"""Append CONTINUATION-250 receipts while preserving existing manifest prefixes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = ROOT / "manifest-prefix-before.json"
FILES = (
    "append_manifests.py",
    "check_text_whitespace.py",
    "manifest-prefix-before.json",
    "local-run.md",
    "commands.txt",
    "environment.txt",
    "source-review.txt",
    "source-sha256.txt",
    "result.json",
    "forgejo-live-egress-gate.json",
    "agent-source-binding.json",
    "acceptance-baseline.log",
    "acceptance-corrected.log",
    "postmerge-acceptance.log",
    "mcp-egress-host-observation-tests.log",
    "sandbox-adapter-tests.log",
    "pycompile.log",
    "diff-check.log",
    "text-whitespace-check.log",
    "final-diff-check.log",
)
SOURCES = (
    "scripts/run_mission_012_stdio_mcp_egress_gate_e2e.py",
    "scripts/run_forgejo_live_pr_merge_e2e.py",
    "tests/test_forgejo_live_pr_merge_e2e.py",
    "docs/checkpoint.md",
    "docs/refusal-evidence-matrix.md",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode().split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and name:
            result[name] = value
    return result


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode())
        stream.flush()
        os.fsync(stream.fileno())


for index in (WORKTREE_INDEX, PROJECT_INDEX):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"global manifest unavailable: {index}")
if LOCAL.exists() and (LOCAL.is_symlink() or LOCAL.stat().st_size):
    raise SystemExit("local manifest is nonempty or symlinked")
if PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("prefix receipt already exists")
LOCAL.touch(exist_ok=True)
before = {
    "local": {"count": 0, "sha256": digest(b"")},
    "worktree": {
        "count": len(WORKTREE_INDEX.read_bytes().splitlines()),
        "sha256": digest(WORKTREE_INDEX.read_bytes()),
    },
    "project": {
        "count": len(PROJECT_INDEX.read_bytes().splitlines()),
        "sha256": digest(PROJECT_INDEX.read_bytes()),
    },
}
PREFIX.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

local_entries: list[tuple[str, str]] = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"local evidence unavailable: {name}")
    local_entries.append((digest(path.read_bytes()), name))
for name in SOURCES:
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"source unavailable: {name}")

append(LOCAL, local_entries)
local_hash = digest(LOCAL.read_bytes())
worktree_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_entries = [(value, f"{worktree_prefix}/{name}") for value, name in local_entries]
worktree_entries.append((local_hash, f"{worktree_prefix}/SHA256SUMS"))
project_entries = [
    (value, f"{project_prefix}/{worktree_prefix}/{name}") for value, name in local_entries
]
project_entries.append((local_hash, f"{project_prefix}/{worktree_prefix}/SHA256SUMS"))
for name in SOURCES:
    value = digest((WORKTREE / name).read_bytes())
    worktree_entries.append((value, name))
    project_entries.append((value, f"{project_prefix}/{name}"))
append(WORKTREE_INDEX, worktree_entries)
project_entries.append(
    (digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS")
)
append(PROJECT_INDEX, project_entries)

for label, path in (("local", LOCAL), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = path.read_bytes().splitlines(keepends=True)
    prior = before[label]
    if digest(b"".join(lines[: prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"{label} manifest prefix changed")
for path, entries in ((WORKTREE_INDEX, worktree_entries), (PROJECT_INDEX, project_entries)):
    current = latest(path)
    for value, name in entries:
        if current.get(name) != value:
            raise SystemExit(f"new latest manifest hash mismatch: {name}")
print(json.dumps({
    "local_entries_appended": len(local_entries),
    "worktree_entries_appended": len(worktree_entries),
    "project_entries_appended": len(project_entries),
    "all_existing_prefixes_preserved": True,
    "new_latest_entries_verified": True,
}, sort_keys=True))
