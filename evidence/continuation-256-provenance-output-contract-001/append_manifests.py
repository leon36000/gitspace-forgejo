#!/usr/bin/env python3
"""Append CONT-256 evidence and source hashes to existing indexes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
FILES = ("append_manifests.py", "verify_manifests.py", "commands.txt", "environment.json",
         "checks-summary.json", "red.log", "single-process.log", "batch-process.log",
         "local-run.md", "source-sha256.txt", "manifest-prefix-before.json")
LOCAL = ROOT / "SHA256SUMS"
PREFIX = ROOT / "manifest-prefix-before.json"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> tuple[dict[str, str], int, int]:
    latest: dict[str, str] = {}
    blanks = malformed = 0
    for row in path.read_bytes().splitlines():
        if not row.strip():
            blanks += 1
            continue
        try:
            value, name = row.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            malformed += 1
            continue
        if len(value) != 64 or any(c not in "0123456789abcdef" for c in value) or not name:
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
    raise SystemExit("local manifest or prefix already exists")
prefixes = {}
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"global manifest unavailable: {path}")
    data = path.read_bytes()
    if data and not data.endswith(b"\n"):
        raise SystemExit(f"global manifest is unterminated: {path}")
    prefixes[label] = {"count": len(data.splitlines(keepends=True)), "sha256": digest(data)}
PREFIX.write_text(json.dumps(prefixes, indent=2, sort_keys=True) + "\n", encoding="utf-8")

sources = []
for row in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = row.split("  ", 1)
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink() or digest(path.read_bytes()) != value:
        raise SystemExit(f"stale source hash: {name}")
    sources.append((value, name))
local_rows = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"evidence file unavailable: {name}")
    local_rows.append((digest(path.read_bytes()), name))
with LOCAL.open("xb") as stream:
    for value, name in local_rows:
        stream.write(f"{value}  {name}\n".encode("utf-8"))
    stream.flush()
    os.fsync(stream.fileno())

ep = ROOT.relative_to(WORKTREE).as_posix()
pp = WORKTREE.relative_to(PROJECT).as_posix()
local_hash = digest(LOCAL.read_bytes())
worktree_rows = [(v, f"{ep}/{n}") for v, n in local_rows]
worktree_rows += [(local_hash, f"{ep}/SHA256SUMS"), *sources]
project_rows = [(v, f"{pp}/{ep}/{n}") for v, n in local_rows]
project_rows += [(local_hash, f"{pp}/{ep}/SHA256SUMS")]
project_rows += [(v, f"{pp}/{n}") for v, n in sources]
append(WORKTREE_INDEX, worktree_rows)
project_rows.append((digest(WORKTREE_INDEX.read_bytes()), f"{pp}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_rows)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(prefixes[label]["count"])
    if digest(b"".join(lines[:count])) != prefixes[label]["sha256"]:
        raise SystemExit(f"original {label} prefix changed")
local, blanks, malformed = parse(LOCAL)
worktree, wb, wm = parse(WORKTREE_INDEX)
project, pb, pm = parse(PROJECT_INDEX)
if blanks or malformed:
    raise SystemExit("local evidence manifest malformed")
for value, name in local_rows:
    if local.get(name) != value or worktree.get(f"{ep}/{name}") != value:
        raise SystemExit(f"local/worktree mismatch: {name}")
    if project.get(f"{pp}/{ep}/{name}") != value:
        raise SystemExit(f"project mismatch: {name}")
for value, name in sources:
    if worktree.get(name) != value or project.get(f"{pp}/{name}") != value:
        raise SystemExit(f"source mismatch: {name}")
if worktree.get(f"{ep}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree local manifest hash mismatch")
if project.get(f"{pp}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project worktree manifest hash mismatch")
print(json.dumps({"status": "verified", "local_evidence_entries": len(local_rows),
                  "source_entries_verified": len(sources),
                  "worktree_rows_appended": len(worktree_rows),
                  "project_rows_appended": len(project_rows),
                  "original_prefixes_preserved": True,
                  "latest_hashes_verified": True,
                  "historical_malformed_rows_retained": wb + wm + pb + pm}, sort_keys=True))
