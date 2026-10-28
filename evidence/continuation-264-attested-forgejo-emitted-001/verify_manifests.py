#!/usr/bin/env python3
"""Verify this live-run evidence, source hashes, and preserved manifest prefixes."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> tuple[dict[str, str], int, int]:
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
        if len(value) != 64 or any(ch not in "0123456789abcdef" for ch in value) or not name:
            malformed += 1
            continue
        latest[name] = value
    return latest, blanks, malformed


local, _, local_bad = parse(LOCAL)
if local_bad:
    raise SystemExit("local manifest contains malformed rows")
files = sorted(path for path in ROOT.iterdir() if path.is_file() and path.name != "SHA256SUMS")
if set(local) != {path.name for path in files}:
    raise SystemExit("local manifest file set does not match evidence directory")
for path in files:
    if path.is_symlink() or local[path.name] != digest(path.read_bytes()):
        raise SystemExit(f"local evidence hash mismatch: {path.name}")
worktree, worktree_blanks, worktree_bad = parse(WORKTREE_INDEX)
project, project_blanks, project_bad = parse(PROJECT_INDEX)
local_hash = digest(LOCAL.read_bytes())
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for path in files:
    value = local[path.name]
    if worktree.get(f"{evidence_prefix}/{path.name}") != value:
        raise SystemExit(f"worktree evidence hash mismatch: {path.name}")
    if project.get(f"{project_prefix}/{evidence_prefix}/{path.name}") != value:
        raise SystemExit(f"project evidence hash mismatch: {path.name}")
if worktree.get(f"{evidence_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree index does not bind local SHA256SUMS")
if project.get(f"{project_prefix}/{evidence_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("project index does not bind local SHA256SUMS")
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    source = WORKTREE / name
    if source.is_symlink() or not source.is_file() or digest(source.read_bytes()) != value:
        raise SystemExit(f"source hash mismatch: {name}")
    if worktree.get(name) != value or project.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"global source index mismatch: {name}")
for label, path in (("local", LOCAL), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    receipt = PREFIX[label]
    lines = path.read_bytes().splitlines(keepends=True)
    count = receipt["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != receipt["sha256"]:
        raise SystemExit(f"historical {label} prefix changed")
if project.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind current worktree index")
print(json.dumps({
    "status": "verified",
    "evidence_files": len(files),
    "source_hashes": len((ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines()),
    "historical_prefixes_preserved": True,
    "historical_blank_and_malformed_rows_preserved": {
        "worktree": {"blank": worktree_blanks, "malformed": worktree_bad},
        "project": {"blank": project_blanks, "malformed": project_bad},
    },
}, sort_keys=True))
