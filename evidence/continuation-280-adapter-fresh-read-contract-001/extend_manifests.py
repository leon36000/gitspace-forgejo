#!/usr/bin/env python3
"""Append later CONTINUATION-280 evidence without rewriting manifest prefixes."""
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
WORKTREE_PREFIX = WORKTREE.relative_to(PROJECT).as_posix()
EVIDENCE_PREFIX = ROOT.relative_to(WORKTREE).as_posix()
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
LOCAL_PREFIX_PATH = ROOT / "extend-prefix-before.json"


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


def write_exclusive(path: Path, data: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def append_rows(path: Path, rows: list[str]) -> None:
    if not rows:
        return
    old = path.read_bytes()
    if old and not old.endswith(b"\n"):
        raise SystemExit(f"{path} has no final newline; refusing to alter prior row")
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
preserve_prefix("local evidence", LOCAL_INDEX, local_prefix)
preserve_prefix("worktree", WORKTREE_INDEX, PREFIX["worktree"])
preserve_prefix("project", PROJECT_INDEX, PREFIX["project"])

local_before = parse(LOCAL_INDEX)
worktree_before = parse(WORKTREE_INDEX)
project_before = parse(PROJECT_INDEX)

local_paths: list[Path] = []
for path in sorted(ROOT.rglob("*"), key=lambda item: item.relative_to(ROOT).as_posix()):
    if path.is_symlink():
        raise SystemExit(f"unexpected evidence symlink: {path.relative_to(ROOT).as_posix()}")
    if path.is_file() and path != LOCAL_INDEX:
        local_paths.append(path)
    elif not path.is_file() and not path.is_dir():
        raise SystemExit(f"unexpected evidence entry: {path.relative_to(ROOT).as_posix()}")

local_updates = []
for path in local_paths:
    name = path.relative_to(ROOT).as_posix()
    value = digest(path.read_bytes())
    if local_before.get(name) != value:
        local_updates.append(f"{value}  {name}")
append_rows(LOCAL_INDEX, local_updates)
local_after = parse(LOCAL_INDEX)

source_rows = []
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    expected, name = line.split("  ", 1)
    source = WORKTREE / name
    if source.is_symlink() or not source.is_file() or digest(source.read_bytes()) != expected:
        raise SystemExit(f"source hash mismatch: {name}")
    source_rows.append((expected, name))

worktree_rows = {
    f"{EVIDENCE_PREFIX}/{name}": value for name, value in local_after.items()
}
worktree_rows.update({name: value for value, name in source_rows})
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

preserve_prefix("local evidence", LOCAL_INDEX, local_prefix)
preserve_prefix("worktree", WORKTREE_INDEX, PREFIX["worktree"])
preserve_prefix("project", PROJECT_INDEX, PREFIX["project"])
print(
    json.dumps(
        {
            "status": "extended",
            "local_updates": len(local_updates),
            "historical_prefixes_preserved": True,
        },
        sort_keys=True,
    )
)
