#!/usr/bin/env python3
"""Append CONT-254 evidence and current Rust source hashes to the indexes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
FILES = (
    "append_manifests.py", "verify_manifests.py", "replay_checks.py",
    "commands.txt", "environment.json", "checks-summary.json",
    "all-targets.log", "local-run.md", "source-sha256.txt",
    "manifest-prefix-before.json",
)
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


def append(path: Path, values: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in values:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


if LOCAL.exists() or LOCAL.is_symlink() or PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("local manifest or prefix receipt already exists")
for path in (WORKTREE_INDEX, PROJECT_INDEX):
    if not path.is_file() or path.is_symlink() or (path.read_bytes() and not path.read_bytes().endswith(b"\n")):
        raise SystemExit(f"global manifest unavailable or unterminated: {path}")
before = {}
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes()
    before[label] = {"count": len(data.splitlines(keepends=True)), "sha256": digest(data)}
PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

source_values = []
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = line.split("  ", 1)
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink() or digest(path.read_bytes()) != value:
        raise SystemExit(f"stale source hash: {name}")
    source_values.append((value, name))
local_values = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"evidence file unavailable: {name}")
    local_values.append((digest(path.read_bytes()), name))
with LOCAL.open("xb") as stream:
    for value, name in local_values:
        stream.write(f"{value}  {name}\n".encode("utf-8"))
    stream.flush()
    os.fsync(stream.fileno())

evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
local_hash = digest(LOCAL.read_bytes())
worktree_values = [(v, f"{evidence_prefix}/{n}") for v, n in local_values]
worktree_values += [(local_hash, f"{evidence_prefix}/SHA256SUMS"), *source_values]
project_values = [(v, f"{project_prefix}/{evidence_prefix}/{n}") for v, n in local_values]
project_values += [(local_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS")]
project_values += [(v, f"{project_prefix}/{n}") for v, n in source_values]
append(WORKTREE_INDEX, worktree_values)
project_values.append((digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_values)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(before[label]["count"])
    if digest(b"".join(lines[:count])) != before[label]["sha256"]:
        raise SystemExit(f"original {label} manifest prefix changed")
local, blanks, malformed = parsed(LOCAL)
worktree, wb, wm = parsed(WORKTREE_INDEX)
project, pb, pm = parsed(PROJECT_INDEX)
if blanks or malformed:
    raise SystemExit("local evidence manifest contains malformed rows")
for value, name in local_values:
    if local.get(name) != value or worktree.get(f"{evidence_prefix}/{name}") != value:
        raise SystemExit(f"local/worktree entry mismatch: {name}")
    if project.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"project entry mismatch: {name}")
for value, name in source_values:
    if worktree.get(name) != value or project.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"source index mismatch: {name}")
if worktree.get(f"{evidence_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree local-manifest hash mismatch")
if project.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project worktree-manifest hash mismatch")
print(json.dumps({"status": "verified", "local_evidence_entries": len(local_values),
                  "source_entries_verified": len(source_values),
                  "worktree_rows_appended": len(worktree_values),
                  "project_rows_appended": len(project_values),
                  "original_prefixes_preserved": True,
                  "latest_hashes_verified": True,
                  "historical_malformed_rows_retained": wb + wm + pb + pm}, sort_keys=True))
