#!/usr/bin/env python3
"""Append CONTINUATION-273 evidence without changing historical manifest prefixes."""
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
        if len(value) == 64 and all(ch in "0123456789abcdef" for ch in value) and name:
            latest[name] = value
    return latest


def preserve_prefix(label: str, path: Path) -> None:
    prior = PREFIX[label]
    data = path.read_bytes() if path.exists() else b""
    lines = data.splitlines(keepends=True)
    count = prior["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != prior["sha256"]:
        raise SystemExit(f"historical {label} manifest prefix changed; refusing to append")


def write_exclusive(path: Path, data: bytes) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def append_rows(path: Path, rows: list[str]) -> None:
    if not rows:
        return
    old = path.read_bytes()
    if old and not old.endswith(b"\n"):
        raise SystemExit(f"{path} does not end in newline; refusing to alter the prior final row")
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "ab") as stream:
        stream.write(("\n".join(rows) + "\n").encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())


local_manifest = ROOT / "SHA256SUMS"
if local_manifest.exists() or local_manifest.is_symlink():
    raise SystemExit("new local manifest already exists; refusing to rewrite it")
local_paths: list[Path] = []
for path in sorted(ROOT.rglob("*"), key=lambda p: p.relative_to(ROOT).as_posix()):
    if path.is_symlink():
        raise SystemExit(f"unexpected local evidence symlink: {path.relative_to(ROOT).as_posix()}")
    if path.is_file():
        local_paths.append(path)
    elif not path.is_dir():
        raise SystemExit(f"unexpected local evidence entry: {path.relative_to(ROOT).as_posix()}")
write_exclusive(
    local_manifest,
    (
        "\n".join(
            f"{digest(path.read_bytes())}  {path.relative_to(ROOT).as_posix()}"
            for path in local_paths
        )
        + "\n"
    ).encode("ascii"),
)

preserve_prefix("worktree", WORKTREE_INDEX)
preserve_prefix("project", PROJECT_INDEX)
worktree_before = parse(WORKTREE_INDEX)
project_before = parse(PROJECT_INDEX)

paths = set(local_paths)
paths.add(local_manifest)
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    expected, name = raw.split("  ", 1)
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != expected:
        raise SystemExit(f"source changed before manifest append: {name}")
    paths.add(path)

worktree_names = {
    path.relative_to(WORKTREE).as_posix(): digest(path.read_bytes()) for path in paths
}
worktree_rows = [
    f"{value}  {name}"
    for name, value in sorted(worktree_names.items())
    if worktree_before.get(name) != value
]
append_rows(WORKTREE_INDEX, worktree_rows)

project_names = {f"{WORKTREE_PREFIX}/{name}": value for name, value in worktree_names.items()}
project_names[f"{WORKTREE_PREFIX}/evidence/SHA256SUMS"] = digest(WORKTREE_INDEX.read_bytes())
project_rows = [
    f"{value}  {name}"
    for name, value in sorted(project_names.items())
    if project_before.get(name) != value
]
append_rows(PROJECT_INDEX, project_rows)

preserve_prefix("worktree", WORKTREE_INDEX)
preserve_prefix("project", PROJECT_INDEX)
print(
    json.dumps(
        {
            "local_files_hashed": len(local_paths),
            "worktree_rows_appended": len(worktree_rows),
            "project_rows_appended": len(project_rows),
            "historical_prefixes_preserved": True,
        },
        sort_keys=True,
    )
)
