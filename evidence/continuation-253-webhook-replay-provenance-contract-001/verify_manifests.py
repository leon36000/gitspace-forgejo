#!/usr/bin/env python3
"""Read-only verification of CONT-253 append-only evidence indexes."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = ROOT / "manifest-prefix-before.json"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parsed(path: Path) -> tuple[dict[str, str], int, int]:
    latest: dict[str, str] = {}
    blanks = malformed = 0
    for raw in path.read_bytes().splitlines():
        if not raw.strip():
            blanks += 1
            continue
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            malformed += 1
            continue
        if len(value) != 64 or any(c not in "0123456789abcdef" for c in value) or not name:
            malformed += 1
            continue
        latest[name] = value
    return latest, blanks, malformed


before = json.loads(PREFIX.read_text(encoding="utf-8"))
indexes = (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX))
for label, path in indexes:
    data = path.read_bytes()
    lines = data.splitlines(keepends=True)
    count = before[label]["count"]
    if digest(b"".join(lines[:count])) != before[label]["sha256"]:
        raise SystemExit(f"original {label} prefix changed")

local, local_blanks, local_bad = parsed(LOCAL)
worktree, worktree_blanks, worktree_bad = parsed(WORKTREE_INDEX)
project, project_blanks, project_bad = parsed(PROJECT_INDEX)
if local_blanks or local_bad:
    raise SystemExit("local evidence index contains malformed rows")
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for name, value in local.items():
    path = ROOT / name
    if not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    if worktree.get(f"{evidence_prefix}/{name}") != value:
        raise SystemExit(f"worktree evidence hash mismatch: {name}")
    if project.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"project evidence hash mismatch: {name}")

source_count = 0
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = line.split("  ", 1)
    path = WORKTREE / name
    if not path.is_file() or digest(path.read_bytes()) != value:
        raise SystemExit(f"source file hash mismatch: {name}")
    if worktree.get(name) != value or project.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"source manifest hash mismatch: {name}")
    source_count += 1

local_manifest_hash = digest(LOCAL.read_bytes())
if worktree.get(f"{evidence_prefix}/SHA256SUMS") != local_manifest_hash:
    raise SystemExit("worktree index does not match local evidence manifest")
if project.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not match current worktree index")

print(json.dumps({
    "status": "verified",
    "local_evidence_entries": len(local),
    "source_entries_verified": source_count,
    "original_prefixes_preserved": True,
    "latest_hashes_verified": True,
    "historical_blank_or_malformed_rows_retained": worktree_blanks + worktree_bad + project_blanks + project_bad,
}, sort_keys=True))
