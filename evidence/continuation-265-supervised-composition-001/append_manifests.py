#!/usr/bin/env python3
"""Append this lot's checksums while preserving historical evidence-index prefixes."""
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
RUN_DIRS = [
    WORKTREE / "evidence" / f"mission-012-live-agent-webhook-{number}-supervised"
    for number in (265, 266, 267, 268, 269)
] + [
    WORKTREE / "evidence" / f"mission-012-live-agent-webhook-{number}-supervised-admission"
    for number in (265, 266, 267, 268, 269)
]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        if not raw.strip():
            continue
        try:
            checksum, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(checksum) == 64 and all(ch in "0123456789abcdef" for ch in checksum) and name:
            latest[name] = checksum
    return latest


def verify_prefix(label: str, path: Path) -> None:
    previous = PREFIX[label]
    data = path.read_bytes()
    lines = data.splitlines(keepends=True)
    count = previous["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != previous["sha256"]:
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
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "ab") as stream:
        stream.write(("\n".join(rows) + "\n").encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())


local_manifest = ROOT / "SHA256SUMS"
local_entries: list[str] = []
for path in sorted(ROOT.iterdir(), key=lambda item: item.name):
    if path.name == "SHA256SUMS":
        continue
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"unexpected local evidence entry: {path.name}")
    local_entries.append(f"{digest(path.read_bytes())}  {path.name}")
if not local_manifest.exists():
    write_exclusive(local_manifest, ("\n".join(local_entries) + "\n").encode("ascii"))
else:
    if local_manifest.is_symlink() or not local_manifest.is_file():
        raise SystemExit("local evidence checksum manifest is not a regular file")
    local_before = parse(local_manifest)
    append_rows(local_manifest, [
        row for row in local_entries
        if local_before.get(row.split("  ", 1)[1]) != row.split("  ", 1)[0]
    ])

for directory in RUN_DIRS:
    if not directory.is_dir():
        continue
    manifest = directory / "SHA256SUMS"
    if manifest.is_symlink() or not manifest.is_file():
        raise SystemExit(f"composed-run checksum manifest missing: {directory.name}")
    for raw in manifest.read_text(encoding="utf-8").splitlines():
        checksum, filename = raw.split("  ", 1)
        path = directory / filename
        if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != checksum:
            raise SystemExit(f"composed-run local hash mismatch: {path}")

verify_prefix("worktree", WORKTREE_INDEX)
verify_prefix("project", PROJECT_INDEX)
worktree_before = parse(WORKTREE_INDEX)
project_before = parse(PROJECT_INDEX)

paths: set[Path] = set()
for path in ROOT.iterdir():
    if path.name == "SHA256SUMS":
        continue
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"unexpected local evidence entry: {path.name}")
    paths.add(path)
paths.add(local_manifest)
for directory in RUN_DIRS:
    if directory.is_dir():
        for path in directory.iterdir():
            if path.is_symlink() or not path.is_file():
                raise SystemExit(f"unexpected composed-run evidence entry: {path}")
            paths.add(path)

source_rows = (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines()
source_paths: list[Path] = []
for raw in source_rows:
    checksum, name = raw.split("  ", 1)
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != checksum:
        raise SystemExit(f"source file changed before manifest append: {name}")
    source_paths.append(path)
paths.update(source_paths)

def index_rows(index: dict[str, str], names: dict[str, str]) -> list[str]:
    rows = []
    for name, checksum in sorted(names.items()):
        if index.get(name) != checksum:
            rows.append(f"{checksum}  {name}")
    return rows


worktree_names = {
    path.relative_to(WORKTREE).as_posix(): digest(path.read_bytes()) for path in paths
}
project_names = {
    f"{WORKTREE_PREFIX}/{name}": checksum for name, checksum in worktree_names.items()
}
worktree_rows = index_rows(worktree_before, worktree_names)
append_rows(WORKTREE_INDEX, worktree_rows)

project_names[f"{WORKTREE_PREFIX}/evidence/SHA256SUMS"] = digest(WORKTREE_INDEX.read_bytes())
project_rows = index_rows(project_before, project_names)
append_rows(PROJECT_INDEX, project_rows)

print(json.dumps({
    "local_manifest_entries": len(parse(local_manifest)),
    "worktree_rows_appended": len(worktree_rows),
    "project_rows_appended": len(project_rows),
    "historical_prefixes_preserved": True,
}, sort_keys=True))
