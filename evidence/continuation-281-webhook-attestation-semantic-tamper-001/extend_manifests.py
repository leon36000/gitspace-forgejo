#!/usr/bin/env python3
"""Append later CONTINUATION-281 evidence without rewriting prior rows."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL_INDEX = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
LOCAL_PREFIX_PATH = ROOT / "extend-prefix-before.json"
WORKTREE_PREFIX = WORKTREE.relative_to(PROJECT).as_posix()
EVIDENCE_PREFIX = ROOT.relative_to(WORKTREE).as_posix()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and all(char in "0123456789abcdef" for char in value) and name:
            latest[name] = value
    return latest


def write_exclusive(path: Path, data: bytes) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def append_rows(path: Path, rows: list[str]) -> None:
    if not rows:
        return
    original = path.read_bytes()
    if original and not original.endswith(b"\n"):
        raise SystemExit(f"{path} has no final newline; refusing to alter prior rows")
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "ab") as stream:
        stream.write(("\n".join(rows) + "\n").encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())


def preserve_prefix(label: str, path: Path, expected: dict[str, object]) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    count = int(expected["line_count"])
    if len(lines) < count or digest(b"".join(lines[:count])) != expected["sha256"]:
        raise SystemExit(f"historical {label} manifest prefix changed")


if not LOCAL_PREFIX_PATH.exists():
    old = LOCAL_INDEX.read_bytes()
    write_exclusive(
        LOCAL_PREFIX_PATH,
        (
            json.dumps(
                {"line_count": len(old.splitlines()), "sha256": digest(old)},
                indent=2,
                sort_keys=True,
            )
            + "\n"
        ).encode("utf-8"),
    )
local_prefix = json.loads(LOCAL_PREFIX_PATH.read_text(encoding="utf-8"))
preserve_prefix("local", LOCAL_INDEX, local_prefix)
preserve_prefix("worktree", WORKTREE_INDEX, PREFIX["worktree"])
preserve_prefix("project", PROJECT_INDEX, PREFIX["project"])

local_before = parse(LOCAL_INDEX)
worktree_before = parse(WORKTREE_INDEX)
project_before = parse(PROJECT_INDEX)
local_paths: list[Path] = []
for path in sorted(ROOT.rglob("*"), key=lambda item: item.relative_to(ROOT).as_posix()):
    if path.is_symlink():
        raise SystemExit(f"unexpected evidence symlink: {path.relative_to(ROOT).as_posix()}")
    if path == LOCAL_INDEX:
        continue
    if path.is_file():
        local_paths.append(path)
    elif not path.is_dir():
        raise SystemExit(f"unexpected evidence entry: {path.relative_to(ROOT).as_posix()}")

local_updates = []
for path in local_paths:
    name = path.relative_to(ROOT).as_posix()
    value = digest(path.read_bytes())
    if local_before.get(name) != value:
        local_updates.append(f"{value}  {name}")
append_rows(LOCAL_INDEX, local_updates)
local_latest = parse(LOCAL_INDEX)

sources: dict[str, str] = {}
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    expected, name = line.split("  ", 1)
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != expected:
        raise SystemExit(f"source hash mismatch: {name}")
    sources[name] = expected

worktree_rows = {f"{EVIDENCE_PREFIX}/{name}": value for name, value in local_latest.items()}
worktree_rows.update(sources)
worktree_rows[f"{EVIDENCE_PREFIX}/SHA256SUMS"] = digest(LOCAL_INDEX.read_bytes())
append_rows(
    WORKTREE_INDEX,
    [
        f"{value}  {name}"
        for name, value in sorted(worktree_rows.items())
        if worktree_before.get(name) != value
    ],
)

project_rows = {f"{WORKTREE_PREFIX}/{name}": value for name, value in worktree_rows.items()}
project_rows[f"{WORKTREE_PREFIX}/evidence/SHA256SUMS"] = digest(WORKTREE_INDEX.read_bytes())
append_rows(
    PROJECT_INDEX,
    [
        f"{value}  {name}"
        for name, value in sorted(project_rows.items())
        if project_before.get(name) != value
    ],
)

preserve_prefix("local", LOCAL_INDEX, local_prefix)
preserve_prefix("worktree", WORKTREE_INDEX, PREFIX["worktree"])
preserve_prefix("project", PROJECT_INDEX, PREFIX["project"])
worktree_latest = parse(WORKTREE_INDEX)
project_latest = parse(PROJECT_INDEX)
for name, expected in local_latest.items():
    if digest((ROOT / name).read_bytes()) != expected:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    if worktree_latest.get(f"{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"worktree index mismatch: {name}")
    if project_latest.get(f"{WORKTREE_PREFIX}/{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"project index mismatch: {name}")
for name, expected in sources.items():
    if worktree_latest.get(name) != expected or project_latest.get(f"{WORKTREE_PREFIX}/{name}") != expected:
        raise SystemExit(f"source index mismatch: {name}")
if worktree_latest.get(f"{EVIDENCE_PREFIX}/SHA256SUMS") != digest(LOCAL_INDEX.read_bytes()):
    raise SystemExit("worktree index does not cover the local index")
if project_latest.get(f"{WORKTREE_PREFIX}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not cover the worktree index")

print(
    json.dumps(
        {
            "status": "extended",
            "local_updates": len(local_updates),
            "local_entries": len(local_latest),
            "source_entries": len(sources),
            "historical_prefixes_preserved": True,
        },
        sort_keys=True,
    )
)
