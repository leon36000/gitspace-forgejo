#!/usr/bin/env python3
"""Append CONT-252 evidence/source hashes and verify existing manifest prefixes."""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = ROOT / "manifest-prefix-before.json"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
FILES = (
    "append_manifests.py",
    "commands.txt",
    "dependency-license-recheck.json",
    "environment.json",
    "focused-tests.log",
    "sandbox-adapter-suite.log",
    "cargo-license-guard.json",
    "cargo-license-tests.log",
    "python-syntax.log",
    "rustfmt.log",
    "diff-check.log",
    "source-sha256.txt",
    "local-run.md",
)

def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()

def entries(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_bytes().splitlines():
        value, name = line.decode().split("  ", 1)
        result[name] = value
    return result

def append(path: Path, values: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in values:
            stream.write(f"{value}  {name}\n".encode())
        stream.flush()
        os.fsync(stream.fileno())

if LOCAL.exists() or PREFIX.exists():
    raise SystemExit("CONT-252 evidence indexes already exist; refusing to overwrite")
for index in (WORKTREE_INDEX, PROJECT_INDEX):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"global manifest unavailable: {index}")

before = {}
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    content = path.read_bytes()
    before[label] = {"count": len(content.splitlines()), "sha256": digest(content)}
PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

local_values: list[tuple[str, str]] = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"evidence file unavailable: {name}")
    local_values.append((digest(path.read_bytes()), name))
source_values: list[tuple[str, str]] = []
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = line.split("  ", 1)
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink() or digest(path.read_bytes()) != value:
        raise SystemExit(f"source hash is stale or unavailable: {name}")
    source_values.append((value, name))

with LOCAL.open("xb") as stream:
    for value, name in local_values:
        stream.write(f"{value}  {name}\n".encode())
    stream.flush()
    os.fsync(stream.fileno())
local_index_hash = digest(LOCAL.read_bytes())
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_values = [(value, f"{evidence_prefix}/{name}") for value, name in local_values]
worktree_values.append((local_index_hash, f"{evidence_prefix}/SHA256SUMS"))
worktree_values.extend(source_values)
project_values = [(value, f"{project_prefix}/{evidence_prefix}/{name}") for value, name in local_values]
project_values.append((local_index_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS"))
project_values.extend((value, f"{project_prefix}/{name}") for value, name in source_values)
append(WORKTREE_INDEX, worktree_values)
project_values.append((digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_values)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    content = path.read_bytes()
    prior = before[label]
    lines = content.splitlines(keepends=True)
    if digest(b"".join(lines[:prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"original {label} manifest prefix changed")
for index, values in ((LOCAL, local_values), (WORKTREE_INDEX, worktree_values), (PROJECT_INDEX, project_values)):
    latest = entries(index)
    if any(latest.get(name) != value for value, name in values):
        raise SystemExit(f"latest entries do not verify in {index}")
print(json.dumps({
    "status": "verified",
    "local_entries": len(local_values),
    "worktree_entries_appended": len(worktree_values),
    "project_entries_appended": len(project_values),
    "original_prefixes_preserved": True,
    "latest_entries_verified": True,
}, sort_keys=True))
