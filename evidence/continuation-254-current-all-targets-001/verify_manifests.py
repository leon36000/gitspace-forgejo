#!/usr/bin/env python3
"""Read-only verification of the CONT-254 evidence indexes and source hashes."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]


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


local, lb, lm = parsed(ROOT / "SHA256SUMS")
worktree, wb, wm = parsed(WORKTREE / "evidence/SHA256SUMS")
project, pb, pm = parsed(PROJECT / "evidence/SHA256SUMS")
before = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
for label, path in (("worktree", WORKTREE / "evidence/SHA256SUMS"),
                    ("project", PROJECT / "evidence/SHA256SUMS")):
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(before[label]["count"])
    if digest(b"".join(lines[:count])) != before[label]["sha256"]:
        raise SystemExit(f"original {label} prefix changed")
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for name, value in local.items():
    if digest((ROOT / name).read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    if worktree.get(f"{evidence_prefix}/{name}") != value:
        raise SystemExit(f"worktree evidence hash mismatch: {name}")
    if project.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"project evidence hash mismatch: {name}")
source_count = 0
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = line.split("  ", 1)
    if digest((WORKTREE / name).read_bytes()) != value:
        raise SystemExit(f"source hash mismatch: {name}")
    if worktree.get(name) != value or project.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"source manifest mismatch: {name}")
    source_count += 1
local_hash = digest((ROOT / "SHA256SUMS").read_bytes())
if worktree.get(f"{evidence_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree local manifest mismatch")
if project.get(f"{project_prefix}/evidence/SHA256SUMS") != digest((WORKTREE / "evidence/SHA256SUMS").read_bytes()):
    raise SystemExit("project worktree manifest mismatch")
print(json.dumps({"status": "verified", "local_evidence_entries": len(local),
                  "source_entries_verified": source_count,
                  "original_prefixes_preserved": True,
                  "latest_hashes_verified": True,
                  "historical_malformed_rows_retained": wb + wm + pb + pm}, sort_keys=True))
