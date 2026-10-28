#!/usr/bin/env python3
"""Append current evidence/source digests while preserving all manifest history."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL_INDEX = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX_PATH = ROOT / "manifest-prefix-before.json"
EVIDENCE_PREFIX = ROOT.relative_to(WORKTREE).as_posix()
WORKTREE_PREFIX = WORKTREE.relative_to(PROJECT).as_posix()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and all(c in "0123456789abcdef" for c in value) and name:
            latest[name] = value
    return latest


def append_rows(path: Path, rows: list[str]) -> None:
    if not rows:
        return
    original = path.read_bytes()
    if original and not original.endswith(b"\n"):
        raise SystemExit(f"{path} has no final newline; preserving it and aborting")
    fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    with os.fdopen(fd, "ab") as stream:
        stream.write(("\n".join(rows) + "\n").encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())


def preserve_prefix(label: str, path: Path, expected: dict[str, object]) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(expected["line_count"])
    if len(lines) < count or digest(b"".join(lines[:count])) != expected["sha256"]:
        raise SystemExit(f"historical {label} prefix changed")


for path in (LOCAL_INDEX, WORKTREE_INDEX, PROJECT_INDEX, PREFIX_PATH):
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"missing or unsafe manifest input: {path}")
prefix = json.loads(PREFIX_PATH.read_text(encoding="utf-8"))
sources = parse(ROOT / "source-sha256.txt")
if not sources:
    raise SystemExit("source digest list is empty")
for name, expected in sources.items():
    relative = PurePosixPath(name)
    path = WORKTREE.joinpath(*relative.parts)
    if relative.is_absolute() or ".." in relative.parts or path.is_symlink() or not path.is_file():
        raise SystemExit(f"missing or unsafe product source: {name}")
    if digest(path.read_bytes()) != expected:
        raise SystemExit(f"product source digest mismatch: {name}")

local_before = parse(LOCAL_INDEX)
local_rows: dict[str, str] = {}
for path in sorted(ROOT.rglob("*"), key=lambda item: item.relative_to(ROOT).as_posix()):
    if path.is_symlink():
        raise SystemExit(f"unexpected symlink: {path.relative_to(ROOT)}")
    if path == LOCAL_INDEX:
        continue
    if path.is_dir():
        continue
    if not path.is_file():
        raise SystemExit(f"unexpected evidence entry: {path.relative_to(ROOT)}")
    name = path.relative_to(ROOT).as_posix()
    local_rows[name] = digest(path.read_bytes())
append_rows(
    LOCAL_INDEX,
    [f"{value}  {name}" for name, value in sorted(local_rows.items()) if local_before.get(name) != value],
)

local_latest = parse(LOCAL_INDEX)
worktree_before = parse(WORKTREE_INDEX)
worktree_rows = {f"{EVIDENCE_PREFIX}/{name}": value for name, value in local_latest.items()}
worktree_rows[f"{EVIDENCE_PREFIX}/SHA256SUMS"] = digest(LOCAL_INDEX.read_bytes())
worktree_rows.update(sources)
append_rows(
    WORKTREE_INDEX,
    [f"{value}  {name}" for name, value in sorted(worktree_rows.items()) if worktree_before.get(name) != value],
)

project_before = parse(PROJECT_INDEX)
project_rows = {f"{WORKTREE_PREFIX}/{name}": value for name, value in worktree_rows.items()}
project_rows[f"{WORKTREE_PREFIX}/evidence/SHA256SUMS"] = digest(WORKTREE_INDEX.read_bytes())
append_rows(
    PROJECT_INDEX,
    [f"{value}  {name}" for name, value in sorted(project_rows.items()) if project_before.get(name) != value],
)

preserve_prefix("worktree", WORKTREE_INDEX, prefix["worktree"])
preserve_prefix("project", PROJECT_INDEX, prefix["project"])
local_latest = parse(LOCAL_INDEX)
worktree_latest = parse(WORKTREE_INDEX)
project_latest = parse(PROJECT_INDEX)
for name, expected in local_latest.items():
    if digest((ROOT / name).read_bytes()) != expected:
        raise SystemExit(f"local evidence digest mismatch: {name}")
    if worktree_latest.get(f"{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"worktree manifest missing latest evidence: {name}")
    if project_latest.get(f"{WORKTREE_PREFIX}/{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"project manifest missing latest evidence: {name}")
for name, expected in sources.items():
    if worktree_latest.get(name) != expected or project_latest.get(f"{WORKTREE_PREFIX}/{name}") != expected:
        raise SystemExit(f"global source digest mismatch: {name}")
if worktree_latest.get(f"{EVIDENCE_PREFIX}/SHA256SUMS") != digest(LOCAL_INDEX.read_bytes()):
    raise SystemExit("worktree manifest does not cover local index")
if project_latest.get(f"{WORKTREE_PREFIX}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project manifest does not cover worktree index")

print(json.dumps({
    "status": "verified",
    "local_paths": len(local_latest),
    "product_sources": len(sources),
    "historical_prefixes_preserved": True,
}, sort_keys=True))
