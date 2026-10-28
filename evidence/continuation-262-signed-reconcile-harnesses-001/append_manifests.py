#!/usr/bin/env python3
"""Append this refinement and its source hashes to both global indexes."""
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


for path in (LOCAL, PREFIX):
    if path.exists() or path.is_symlink():
        raise SystemExit("refusing to overwrite local manifest or prefix receipt")
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"global manifest unavailable: {path}")
    data = path.read_bytes()
    if data and not data.endswith(b"\n"):
        raise SystemExit(f"global manifest is not newline-terminated: {path}")
    _, blanks, malformed = parse(data)
    if label == "worktree":
        worktree_before = {"line_count": len(data.splitlines(keepends=True)), "sha256": digest(data)}
        worktree_blanks, worktree_malformed = blanks, malformed
    else:
        project_before = {"line_count": len(data.splitlines(keepends=True)), "sha256": digest(data)}
        project_blanks, project_malformed = blanks, malformed
PREFIX.write_text(json.dumps({"worktree": worktree_before, "project": project_before}, indent=2, sort_keys=True) + "\n", encoding="utf-8")

source_rows = []
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    relative = Path(name)
    source = WORKTREE / relative
    if relative.is_absolute() or ".." in relative.parts or not source.is_file() or source.is_symlink():
        raise SystemExit(f"unsafe or missing source: {name}")
    if digest(source.read_bytes()) != value:
        raise SystemExit(f"stale source hash: {name}")
    source_rows.append((value, name))

files = sorted(path for path in ROOT.iterdir() if path.name != "SHA256SUMS")
if any(path.is_symlink() or not path.is_file() for path in files):
    raise SystemExit("unexpected evidence entry type")
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
worktree_rows.append((local_hash, f"{evidence_prefix}/SHA256SUMS"))
worktree_rows.extend(source_rows)
append(WORKTREE_INDEX, worktree_rows)
project_rows = [(value, f"{project_prefix}/{evidence_prefix}/{name}") for value, name in local_rows]
project_rows.append((local_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS"))
project_rows.extend((value, f"{project_prefix}/{name}") for value, name in source_rows)
append(PROJECT_INDEX, project_rows)
append(PROJECT_INDEX, [(digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS")])

for label, path, before in (("worktree", WORKTREE_INDEX, worktree_before), ("project", PROJECT_INDEX, project_before)):
    lines = path.read_bytes().splitlines(keepends=True)
    count = before["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != before["sha256"]:
        raise SystemExit(f"historical {label} prefix changed")

print(json.dumps({
    "status": "appended",
    "local_evidence_entries": len(local_rows),
    "source_hashes": len(source_rows),
    "worktree_rows_appended": len(worktree_rows),
    "project_rows_appended": len(project_rows) + 1,
    "historical_prefixes_preserved": True,
    "historical_blank_rows_preserved": {"worktree": worktree_blanks, "project": project_blanks},
    "historical_malformed_rows_preserved": {"worktree": worktree_malformed, "project": project_malformed},
}, sort_keys=True))
