#!/usr/bin/env python3
"""Append updated evidence/source hashes while preserving the original prefixes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EVIDENCE_PREFIX = ROOT.relative_to(WORKTREE).as_posix()
PROJECT_PREFIX = WORKTREE.relative_to(PROJECT).as_posix()


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


for path in (LOCAL, WORKTREE_INDEX, PROJECT_INDEX):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"manifest unavailable or unsafe: {path}")
    data = path.read_bytes()
    if data and not data.endswith(b"\n"):
        raise SystemExit(f"manifest is not newline-terminated: {path}")

old_local, _, bad_local = parse(LOCAL.read_bytes())
if bad_local:
    raise SystemExit("local evidence manifest contains malformed rows")
worktree_latest, _, _ = parse(WORKTREE_INDEX.read_bytes())
project_latest, _, _ = parse(PROJECT_INDEX.read_bytes())

source_rows: list[tuple[str, str]] = []
seen_sources: set[str] = set()
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    source_path = Path(name)
    source = WORKTREE / source_path
    if (source_path.is_absolute() or ".." in source_path.parts or not source.is_file()
            or source.is_symlink() or name in seen_sources):
        raise SystemExit(f"unsafe, duplicate, or missing source path: {name}")
    if digest(source.read_bytes()) != value:
        raise SystemExit(f"stale source hash: {name}")
    seen_sources.add(name)
    source_rows.append((value, name))

entries = list(ROOT.iterdir())
if any(path.is_symlink() for path in entries):
    raise SystemExit("symlink in evidence directory")
if any(path.is_dir() and path.name != "__pycache__" for path in entries):
    raise SystemExit("unexpected evidence subdirectory")
files = sorted(path for path in entries if path.is_file() and path.name != "SHA256SUMS")
evidence_rows = [(digest(path.read_bytes()), path.name) for path in files]
local_updates = [(value, name) for value, name in evidence_rows if old_local.get(name) != value]
if not local_updates:
    raise SystemExit("no new evidence rows to append")
append(LOCAL, local_updates)
local_hash = digest(LOCAL.read_bytes())

worktree_updates = [
    (value, f"{EVIDENCE_PREFIX}/{name}")
    for value, name in evidence_rows
    if worktree_latest.get(f"{EVIDENCE_PREFIX}/{name}") != value
]
worktree_local_name = f"{EVIDENCE_PREFIX}/SHA256SUMS"
if worktree_latest.get(worktree_local_name) != local_hash:
    worktree_updates.append((local_hash, worktree_local_name))
for value, name in source_rows:
    if worktree_latest.get(name) != value:
        worktree_updates.append((value, name))
append(WORKTREE_INDEX, worktree_updates)

project_updates = [
    (value, f"{PROJECT_PREFIX}/{EVIDENCE_PREFIX}/{name}")
    for value, name in evidence_rows
    if project_latest.get(f"{PROJECT_PREFIX}/{EVIDENCE_PREFIX}/{name}") != value
]
project_local_name = f"{PROJECT_PREFIX}/{EVIDENCE_PREFIX}/SHA256SUMS"
if project_latest.get(project_local_name) != local_hash:
    project_updates.append((local_hash, project_local_name))
for value, name in source_rows:
    project_name = f"{PROJECT_PREFIX}/{name}"
    if project_latest.get(project_name) != value:
        project_updates.append((value, project_name))
project_index_name = f"{PROJECT_PREFIX}/evidence/SHA256SUMS"
worktree_hash = digest(WORKTREE_INDEX.read_bytes())
if project_latest.get(project_index_name) != worktree_hash:
    project_updates.append((worktree_hash, project_index_name))
append(PROJECT_INDEX, project_updates)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    before = PREFIX[label]
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(before["line_count"])
    if len(lines) < count or digest(b"".join(lines[:count])) != before["sha256"]:
        raise SystemExit(f"historical {label} prefix changed")

worktree_latest, worktree_blanks, worktree_bad = parse(WORKTREE_INDEX.read_bytes())
project_latest, project_blanks, project_bad = parse(PROJECT_INDEX.read_bytes())
for value, name in evidence_rows:
    if worktree_latest.get(f"{EVIDENCE_PREFIX}/{name}") != value:
        raise SystemExit(f"worktree manifest failed readback: {name}")
    if project_latest.get(f"{PROJECT_PREFIX}/{EVIDENCE_PREFIX}/{name}") != value:
        raise SystemExit(f"project manifest failed readback: {name}")
for value, name in source_rows:
    if worktree_latest.get(name) != value or project_latest.get(f"{PROJECT_PREFIX}/{name}") != value:
        raise SystemExit(f"source hash failed manifest readback: {name}")
if worktree_latest.get(worktree_local_name) != digest(LOCAL.read_bytes()):
    raise SystemExit("worktree manifest does not bind updated local manifest")
if project_latest.get(project_local_name) != digest(LOCAL.read_bytes()):
    raise SystemExit("project manifest does not bind updated local manifest")
if project_latest.get(project_index_name) != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project manifest does not bind current worktree manifest")

print(json.dumps({
    "status": "refinement_appended_and_verified",
    "source_hashes": len(source_rows),
    "local_evidence_rows_appended": len(local_updates),
    "worktree_rows_appended": len(worktree_updates),
    "project_rows_appended": len(project_updates),
    "historical_prefixes_preserved": True,
    "historical_blank_rows_preserved": {"worktree": worktree_blanks, "project": project_blanks},
    "historical_malformed_rows_preserved": {"worktree": worktree_bad, "project": project_bad},
}, sort_keys=True))
