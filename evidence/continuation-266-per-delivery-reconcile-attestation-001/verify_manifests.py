#!/usr/bin/env python3
"""Verify this lot's checksums and append-only index prefix preservation."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
WORKTREE_PREFIX = WORKTREE.relative_to(PROJECT).as_posix()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and all(ch in "0123456789abcdef" for ch in value) and name:
            latest[name] = value
    return latest


def verify_prefix(label: str, path: Path) -> None:
    prior = PREFIX[label]
    lines = path.read_bytes().splitlines(keepends=True)
    count = prior["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != prior["sha256"]:
        raise SystemExit(f"historical {label} manifest prefix changed")


local = parse(ROOT / "SHA256SUMS")
files = sorted(path for path in ROOT.iterdir() if path.is_file() and path.name != "SHA256SUMS")
if set(local) != {path.name for path in files}:
    raise SystemExit("local checksum manifest does not cover the exact evidence file set")
for path in files:
    if path.is_symlink() or local[path.name] != digest(path.read_bytes()):
        raise SystemExit(f"local checksum mismatch: {path.name}")

worktree = parse(WORKTREE_INDEX)
project = parse(PROJECT_INDEX)
for path in [*files, ROOT / "SHA256SUMS"]:
    name = path.relative_to(WORKTREE).as_posix()
    value = digest(path.read_bytes())
    if worktree.get(name) != value or project.get(f"{WORKTREE_PREFIX}/{name}") != value:
        raise SystemExit(f"global evidence index mismatch: {name}")
source_count = 0
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"source checksum mismatch: {name}")
    if worktree.get(name) != value or project.get(f"{WORKTREE_PREFIX}/{name}") != value:
        raise SystemExit(f"global source index mismatch: {name}")
    source_count += 1

verify_prefix("local", ROOT / "SHA256SUMS")
verify_prefix("worktree", WORKTREE_INDEX)
verify_prefix("project", PROJECT_INDEX)
if project.get(f"{WORKTREE_PREFIX}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind the current worktree evidence index")
print(json.dumps({
    "status": "verified",
    "local_evidence_files": len(files),
    "source_hashes": source_count,
    "historical_prefixes_preserved": True,
}, sort_keys=True))
