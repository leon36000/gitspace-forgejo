#!/usr/bin/env python3
"""Append this evidence lot and its source hashes to both evidence indexes."""
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
    with path.open("ab") as stream:
        for value, name in rows:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


if LOCAL.exists() or LOCAL.is_symlink() or PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("refusing to overwrite a local manifest or prefix receipt")
historical_malformed_rows = {}
historical_blank_rows = {}
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"global manifest unavailable: {path}")
    data = path.read_bytes()
    if data and not data.endswith(b"\n"):
        raise SystemExit(f"global manifest is not newline-terminated: {path}")
    _, blanks, malformed = parse(data)
    historical_malformed_rows[label] = malformed
    historical_blank_rows[label] = blanks

before = {}
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes()
    before[label] = {"line_count": len(data.splitlines(keepends=True)), "sha256": digest(data)}
PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

source_rows = []
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    source_path = Path(name)
    source = WORKTREE / source_path
    if source_path.is_absolute() or ".." in source_path.parts or not source.is_file() or source.is_symlink():
        raise SystemExit(f"unsafe or unavailable source path: {name}")
    if digest(source.read_bytes()) != value:
        raise SystemExit(f"stale source hash: {name}")
    source_rows.append((value, name))

entries = list(ROOT.iterdir())
if any(path.is_symlink() or not path.is_file() for path in entries):
    raise SystemExit("unexpected evidence entry type")
files = sorted(path for path in entries if path.name != "SHA256SUMS")
local_rows = [(digest(path.read_bytes()), path.name) for path in files]
with LOCAL.open("xb") as stream:
    for value, name in local_rows:
        stream.write(f"{value}  {name}\n".encode("utf-8"))
    stream.flush()
    os.fsync(stream.fileno())

local_hash = digest(LOCAL.read_bytes())
evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_rows = [(value, f"{evidence_prefix}/{name}") for value, name in local_rows]
worktree_rows += [(local_hash, f"{evidence_prefix}/SHA256SUMS"), *source_rows]
project_rows = [(value, f"{project_prefix}/{evidence_prefix}/{name}") for value, name in local_rows]
project_rows += [(local_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS")]
project_rows += [(value, f"{project_prefix}/{name}") for value, name in source_rows]
append(WORKTREE_INDEX, worktree_rows)
project_rows.append((digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_rows)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    count = before[label]["line_count"]
    prefix = b"".join(path.read_bytes().splitlines(keepends=True)[:count])
    if digest(prefix) != before[label]["sha256"]:
        raise SystemExit(f"historical {label} manifest prefix changed")

print(json.dumps({
    "status": "appended",
    "local_evidence_entries": len(local_rows),
    "source_hashes": len(source_rows),
    "worktree_rows_appended": len(worktree_rows),
    "project_rows_appended": len(project_rows),
    "historical_prefixes_preserved": True,
    "historical_malformed_rows_preserved": historical_malformed_rows,
    "historical_blank_rows_preserved": historical_blank_rows,
}, sort_keys=True))
