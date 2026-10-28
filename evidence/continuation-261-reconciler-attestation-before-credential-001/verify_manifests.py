#!/usr/bin/env python3
"""Verify this evidence lot, source hashes, and append-only manifest prefixes."""
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


if not LOCAL.is_file() or LOCAL.is_symlink():
    raise SystemExit("local SHA256SUMS missing or unsafe")
local, _, malformed = parse(LOCAL)
if malformed:
    raise SystemExit("local manifest contains malformed rows")
files = sorted(path for path in ROOT.iterdir() if path.is_file() and path.name != "SHA256SUMS")
if set(local) != {path.name for path in files}:
    raise SystemExit("local manifest file set does not match evidence directory")
for path in files:
    if path.is_symlink() or local[path.name] != digest(path.read_bytes()):
        raise SystemExit(f"local evidence hash mismatch: {path.name}")

worktree, worktree_blanks, worktree_malformed = parse(WORKTREE_INDEX)
project, project_blanks, project_malformed = parse(PROJECT_INDEX)
local_hash = digest(LOCAL.read_bytes())
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for path in files:
    expected = local[path.name]
    if worktree.get(f"{evidence_prefix}/{path.name}") != expected:
        raise SystemExit(f"worktree manifest mismatch: {path.name}")
    if project.get(f"{project_prefix}/{evidence_prefix}/{path.name}") != expected:
        raise SystemExit(f"project manifest mismatch: {path.name}")
if worktree.get(f"{evidence_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree manifest does not bind local SHA256SUMS")
if project.get(f"{project_prefix}/{evidence_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("project manifest does not bind local SHA256SUMS")

source_rows = 0
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    expected, name = raw.split("  ", 1)
    source_path = Path(name)
    source = WORKTREE / source_path
    if source_path.is_absolute() or ".." in source_path.parts or not source.is_file() or source.is_symlink():
        raise SystemExit(f"unsafe or unavailable source path: {name}")
    if digest(source.read_bytes()) != expected:
        raise SystemExit(f"stale local source hash: {name}")
    if worktree.get(name) != expected or project.get(f"{project_prefix}/{name}") != expected:
        raise SystemExit(f"source hash missing from a global index: {name}")
    source_rows += 1

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    receipt = PREFIX[label]
    rows = path.read_bytes().splitlines(keepends=True)
    prefix = b"".join(rows[:receipt["line_count"]])
    if digest(prefix) != receipt["sha256"]:
        raise SystemExit(f"historical {label} manifest prefix changed")
if project.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind the current worktree index")

print(json.dumps({
    "status": "verified",
    "local_evidence_entries": len(files),
    "source_hashes": source_rows,
    "historical_prefixes_preserved": True,
    "historical_malformed_rows_preserved": {
        "worktree": worktree_malformed,
        "project": project_malformed,
    },
    "historical_blank_rows_preserved": {
        "worktree": worktree_blanks,
        "project": project_blanks,
    },
}, sort_keys=True))
