#!/usr/bin/env python3
"""Read-only verifier for the CONT-257 append-only evidence indexes."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for row in path.read_bytes().splitlines():
        if not row.strip():
            continue
        value, name = row.decode("utf-8").split("  ", 1)
        latest[name] = value
    return latest


local = parse(ROOT / "SHA256SUMS")
worktree = parse(WORKTREE / "evidence/SHA256SUMS")
project = parse(PROJECT / "evidence/SHA256SUMS")
prefixes = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
for label, path in (("worktree", WORKTREE / "evidence/SHA256SUMS"),
                    ("project", PROJECT / "evidence/SHA256SUMS")):
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(prefixes[label]["count"])
    if digest(b"".join(lines[:count])) != prefixes[label]["sha256"]:
        raise SystemExit(f"original {label} prefix changed")
ep = ROOT.relative_to(WORKTREE).as_posix()
pp = WORKTREE.relative_to(PROJECT).as_posix()
for name, value in local.items():
    path = ROOT / name
    if not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    if worktree.get(f"{ep}/{name}") != value or project.get(f"{pp}/{ep}/{name}") != value:
        raise SystemExit(f"global evidence hash mismatch: {name}")
source_count = 0
for row in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = row.split("  ", 1)
    path = WORKTREE / name
    if not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"source hash mismatch: {name}")
    if worktree.get(name) != value or project.get(f"{pp}/{name}") != value:
        raise SystemExit(f"source index mismatch: {name}")
    source_count += 1
local_hash = digest((ROOT / "SHA256SUMS").read_bytes())
if worktree.get(f"{ep}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree local-manifest hash mismatch")
if project.get(f"{pp}/evidence/SHA256SUMS") != digest((WORKTREE / "evidence/SHA256SUMS").read_bytes()):
    raise SystemExit("project worktree-manifest hash mismatch")
print(json.dumps({
    "status": "verified",
    "local_evidence_entries": len(local),
    "source_entries_verified": source_count,
    "original_prefixes_preserved": True,
    "latest_hashes_verified": True,
}, sort_keys=True))
