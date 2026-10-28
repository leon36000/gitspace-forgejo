#!/usr/bin/env python3
"""Verify CONTINUATION-280 local, worktree, project and source hashes."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and all(c in "0123456789abcdef" for c in value) and name:
            latest[name] = value
    return latest


local = parse(ROOT / "SHA256SUMS")
worktree = parse(WORKTREE / "evidence/SHA256SUMS")
project = parse(PROJECT / "evidence/SHA256SUMS")
for label, path in (
    ("worktree", WORKTREE / "evidence/SHA256SUMS"),
    ("project", PROJECT / "evidence/SHA256SUMS"),
):
    lines = path.read_bytes().splitlines(keepends=True)
    count = PREFIX[label]["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != PREFIX[label]["sha256"]:
        raise SystemExit(f"original {label} prefix changed")

worktree_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for name, expected in local.items():
    path = ROOT / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != expected:
        raise SystemExit(f"local file hash mismatch: {name}")
    if worktree.get(f"{worktree_prefix}/{name}") != expected:
        raise SystemExit(f"worktree manifest mismatch: {name}")
    if project.get(f"{project_prefix}/{worktree_prefix}/{name}") != expected:
        raise SystemExit(f"project manifest mismatch: {name}")

source_count = 0
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    expected, name = line.split("  ", 1)
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != expected:
        raise SystemExit(f"source hash mismatch: {name}")
    if worktree.get(name) != expected or project.get(f"{project_prefix}/{name}") != expected:
        raise SystemExit(f"source not recorded in both manifests: {name}")
    source_count += 1

if worktree.get(f"{worktree_prefix}/SHA256SUMS") != digest((ROOT / "SHA256SUMS").read_bytes()):
    raise SystemExit("worktree local manifest hash mismatch")
if project.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(
    (WORKTREE / "evidence/SHA256SUMS").read_bytes()
):
    raise SystemExit("project worktree manifest hash mismatch")
print(
    json.dumps(
        {
            "status": "verified",
            "local_entries": len(local),
            "source_entries": source_count,
            "historical_prefixes_preserved": True,
            "latest_hashes_verified": True,
        },
        sort_keys=True,
    )
)
