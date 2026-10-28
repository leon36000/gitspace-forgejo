#!/usr/bin/env python3
"""Append CONTINUATION-282 evidence and inspected-source hashes."""
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
PREFIX_PATH = ROOT / "manifest-prefix-before.json"
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
    rows = path.read_bytes().splitlines(keepends=True)
    count = int(expected["line_count"])
    if len(rows) < count or digest(b"".join(rows[:count])) != expected["sha256"]:
        raise SystemExit(f"historical {label} manifest prefix changed")


if LOCAL_INDEX.exists() or LOCAL_INDEX.is_symlink():
    raise SystemExit("local evidence manifest already exists")
if PREFIX_PATH.exists() or PREFIX_PATH.is_symlink():
    raise SystemExit("manifest prefix snapshot already exists")

prefix = {
    "worktree": {
        "line_count": len(WORKTREE_INDEX.read_bytes().splitlines()),
        "sha256": digest(WORKTREE_INDEX.read_bytes()),
    },
    "project": {
        "line_count": len(PROJECT_INDEX.read_bytes().splitlines()),
        "sha256": digest(PROJECT_INDEX.read_bytes()),
    },
}
write_exclusive(PREFIX_PATH, (json.dumps(prefix, indent=2, sort_keys=True) + "\n").encode())

sources = parse(ROOT / "source-sha256.txt")
source_bytes = (ROOT / "source-sha256.txt").read_bytes()
if not source_bytes.endswith(b"\n") or not sources:
    raise SystemExit("source hash list is empty or lacks its final newline")
for name, expected in sources.items():
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != expected:
        raise SystemExit(f"source hash mismatch: {name}")

files: list[Path] = []
for path in sorted(ROOT.rglob("*"), key=lambda item: item.relative_to(ROOT).as_posix()):
    if path.is_symlink():
        raise SystemExit(f"unexpected symlink: {path.relative_to(ROOT).as_posix()}")
    if path == LOCAL_INDEX:
        continue
    if path.is_file():
        files.append(path)
    elif not path.is_dir():
        raise SystemExit(f"unexpected evidence entry: {path.relative_to(ROOT).as_posix()}")

write_exclusive(
    LOCAL_INDEX,
    (
        "\n".join(
            f"{digest(path.read_bytes())}  {path.relative_to(ROOT).as_posix()}" for path in files
        )
        + "\n"
    ).encode("ascii"),
)
local_entries = parse(LOCAL_INDEX)
worktree_before = parse(WORKTREE_INDEX)
project_before = parse(PROJECT_INDEX)

worktree_rows = {f"{EVIDENCE_PREFIX}/{name}": value for name, value in local_entries.items()}
worktree_rows[f"{EVIDENCE_PREFIX}/SHA256SUMS"] = digest(LOCAL_INDEX.read_bytes())
worktree_rows.update(sources)
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

preserve_prefix("worktree", WORKTREE_INDEX, prefix["worktree"])
preserve_prefix("project", PROJECT_INDEX, prefix["project"])
local_latest = parse(LOCAL_INDEX)
worktree_latest = parse(WORKTREE_INDEX)
project_latest = parse(PROJECT_INDEX)
for name, expected in local_latest.items():
    if digest((ROOT / name).read_bytes()) != expected:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    if worktree_latest.get(f"{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"worktree index missing local entry: {name}")
    if project_latest.get(f"{WORKTREE_PREFIX}/{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"project index missing local entry: {name}")
for name, expected in sources.items():
    if worktree_latest.get(name) != expected or project_latest.get(f"{WORKTREE_PREFIX}/{name}") != expected:
        raise SystemExit(f"global source index mismatch: {name}")
if worktree_latest.get(f"{EVIDENCE_PREFIX}/SHA256SUMS") != digest(LOCAL_INDEX.read_bytes()):
    raise SystemExit("worktree index does not cover the local index")
if project_latest.get(f"{WORKTREE_PREFIX}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not cover the worktree index")

print(
    json.dumps(
        {
            "status": "verified",
            "local_entries": len(local_latest),
            "source_entries": len(sources),
            "historical_prefixes_preserved": True,
        },
        sort_keys=True,
    )
)
