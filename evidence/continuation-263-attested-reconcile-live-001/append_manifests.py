#!/usr/bin/env python3
"""Append audit files to the runner's existing local manifest and global indexes."""
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


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(data: bytes) -> tuple[dict[str, str], int, int]:
    latest: dict[str, str] = {}
    blanks = malformed = 0
    for raw in data.splitlines():
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


def append(path: Path, rows: list[tuple[str, str]]) -> None:
    if not rows:
        return
    with path.open("ab") as stream:
        for value, name in rows:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


if PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("refusing to overwrite manifest-prefix-before.json")
for path in (LOCAL, WORKTREE_INDEX, PROJECT_INDEX):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"manifest unavailable or unsafe: {path}")
    if path.read_bytes() and not path.read_bytes().endswith(b"\n"):
        raise SystemExit(f"manifest is not newline-terminated: {path}")

before = {}
counts = {}
for label, path in (("local", LOCAL), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes()
    _, blanks, malformed = parse(data)
    before[label] = {"line_count": len(data.splitlines(keepends=True)), "sha256": digest(data)}
    counts[label] = {"blank": blanks, "malformed": malformed}
PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

source_rows = []
seen_sources: set[str] = set()
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    relative = Path(name)
    source = WORKTREE / relative
    if relative.is_absolute() or ".." in relative.parts or not source.is_file() or source.is_symlink() or name in seen_sources:
        raise SystemExit(f"unsafe, duplicate, or missing source: {name}")
    if digest(source.read_bytes()) != value:
        raise SystemExit(f"source hash differs from runner provenance: {name}")
    seen_sources.add(name)
    source_rows.append((value, name))

files = sorted(path for path in ROOT.iterdir() if path.name != "SHA256SUMS")
if any(path.is_symlink() or not path.is_file() for path in files):
    raise SystemExit("unexpected evidence entry type")
local_before, _, local_malformed = parse(LOCAL.read_bytes())
if local_malformed:
    raise SystemExit("local SHA256SUMS contains malformed rows")
evidence_rows = [(digest(path.read_bytes()), path.name) for path in files]
local_updates = [(value, name) for value, name in evidence_rows if local_before.get(name) != value]
append(LOCAL, local_updates)
local_hash = digest(LOCAL.read_bytes())

worktree_before, _, _ = parse(WORKTREE_INDEX.read_bytes())
project_before, _, _ = parse(PROJECT_INDEX.read_bytes())
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_rows = [
    (value, f"{evidence_prefix}/{name}")
    for value, name in evidence_rows
    if worktree_before.get(f"{evidence_prefix}/{name}") != value
]
worktree_local_name = f"{evidence_prefix}/SHA256SUMS"
if worktree_before.get(worktree_local_name) != local_hash:
    worktree_rows.append((local_hash, worktree_local_name))
for value, name in source_rows:
    if worktree_before.get(name) != value:
        worktree_rows.append((value, name))
append(WORKTREE_INDEX, worktree_rows)

project_rows = [
    (value, f"{project_prefix}/{evidence_prefix}/{name}")
    for value, name in evidence_rows
    if project_before.get(f"{project_prefix}/{evidence_prefix}/{name}") != value
]
project_local_name = f"{project_prefix}/{evidence_prefix}/SHA256SUMS"
if project_before.get(project_local_name) != local_hash:
    project_rows.append((local_hash, project_local_name))
for value, name in source_rows:
    project_name = f"{project_prefix}/{name}"
    if project_before.get(project_name) != value:
        project_rows.append((value, project_name))
project_index_name = f"{project_prefix}/evidence/SHA256SUMS"
append(PROJECT_INDEX, project_rows)
append(PROJECT_INDEX, [(digest(WORKTREE_INDEX.read_bytes()), project_index_name)])

for label, path in (("local", LOCAL), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    receipt = before[label]
    lines = path.read_bytes().splitlines(keepends=True)
    count = receipt["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != receipt["sha256"]:
        raise SystemExit(f"historical {label} prefix changed")

local_latest, _, _ = parse(LOCAL.read_bytes())
worktree_latest, _, _ = parse(WORKTREE_INDEX.read_bytes())
project_latest, _, _ = parse(PROJECT_INDEX.read_bytes())
for value, name in evidence_rows:
    if local_latest.get(name) != value:
        raise SystemExit(f"local manifest failed readback: {name}")
    if worktree_latest.get(f"{evidence_prefix}/{name}") != value:
        raise SystemExit(f"worktree index failed readback: {name}")
    if project_latest.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"project index failed readback: {name}")
for value, name in source_rows:
    if worktree_latest.get(name) != value or project_latest.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"source index failed readback: {name}")
if worktree_latest.get(worktree_local_name) != digest(LOCAL.read_bytes()):
    raise SystemExit("worktree index does not bind local SHA256SUMS")
if project_latest.get(project_local_name) != digest(LOCAL.read_bytes()):
    raise SystemExit("project index does not bind local SHA256SUMS")
if project_latest.get(project_index_name) != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind current worktree index")

print(json.dumps({
    "status": "appended_and_verified",
    "evidence_files": len(evidence_rows),
    "source_hashes": len(source_rows),
    "local_rows_appended": len(local_updates),
    "worktree_rows_appended": len(worktree_rows),
    "project_rows_appended": len(project_rows) + 1,
    "historical_prefixes_preserved": True,
    "historical_blank_and_malformed_rows_preserved": counts,
}, sort_keys=True))
