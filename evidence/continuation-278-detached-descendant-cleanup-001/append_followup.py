#!/usr/bin/env python3
"""Append updated CONTINUATION-278 hashes without rewriting prior rows."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
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


def preserve_prefix(label: str, path: Path) -> None:
    prior = PREFIX[label]
    lines = path.read_bytes().splitlines(keepends=True)
    if len(lines) < prior["line_count"] or digest(b"".join(lines[:prior["line_count"]])) != prior["sha256"]:
        raise SystemExit(f"historical {label} prefix changed; refusing to append")


def append_rows(path: Path, rows: list[str]) -> None:
    if not rows:
        return
    old = path.read_bytes()
    if old and not old.endswith(b"\n"):
        raise SystemExit(f"{path} does not end in newline; refusing to alter its prior final row")
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "ab") as stream:
        stream.write(("\n".join(rows) + "\n").encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())


local_manifest = ROOT / "SHA256SUMS"
local_before = parse(local_manifest)
local_updates: dict[str, str] = {}
for path in sorted(ROOT.rglob("*"), key=lambda item: item.relative_to(ROOT).as_posix()):
    if path == local_manifest:
        continue
    if path.is_symlink():
        raise SystemExit(f"unexpected evidence symlink: {path.relative_to(ROOT).as_posix()}")
    if path.is_file():
        name = path.relative_to(ROOT).as_posix()
        value = digest(path.read_bytes())
        if local_before.get(name) != value:
            local_updates[name] = value
    elif not path.is_dir():
        raise SystemExit(f"unexpected evidence entry: {path.relative_to(ROOT).as_posix()}")
if not local_updates:
    raise SystemExit("no changed local evidence files to append")
append_rows(local_manifest, [f"{value}  {name}" for name, value in sorted(local_updates.items())])

preserve_prefix("worktree", WORKTREE_INDEX)
preserve_prefix("project", PROJECT_INDEX)
worktree_before = parse(WORKTREE_INDEX)
project_before = parse(PROJECT_INDEX)
local_latest = parse(local_manifest)
paths = {ROOT / name for name in local_latest}
paths.add(local_manifest)
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    expected, name = line.split("  ", 1)
    source = WORKTREE / name
    if source.is_symlink() or not source.is_file() or digest(source.read_bytes()) != expected:
        raise SystemExit(f"source changed before follow-up append: {name}")
    paths.add(source)

worktree_names = {path.relative_to(WORKTREE).as_posix(): digest(path.read_bytes()) for path in paths}
append_rows(
    WORKTREE_INDEX,
    [f"{value}  {name}" for name, value in sorted(worktree_names.items()) if worktree_before.get(name) != value],
)
project_names = {f"{WORKTREE_PREFIX}/{name}": value for name, value in worktree_names.items()}
project_names[f"{WORKTREE_PREFIX}/evidence/SHA256SUMS"] = digest(WORKTREE_INDEX.read_bytes())
append_rows(
    PROJECT_INDEX,
    [f"{value}  {name}" for name, value in sorted(project_names.items()) if project_before.get(name) != value],
)
preserve_prefix("worktree", WORKTREE_INDEX)
preserve_prefix("project", PROJECT_INDEX)
print(json.dumps({"updated_local_files": len(local_updates), "historical_prefixes_preserved": True, "status": "appended"}, sort_keys=True))
