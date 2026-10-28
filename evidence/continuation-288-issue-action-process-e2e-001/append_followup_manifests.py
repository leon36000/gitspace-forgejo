#!/usr/bin/env python3
"""Append latest CONTINUATION-288 digests while preserving all earlier rows."""
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
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text())
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
        raise SystemExit(f"{path} has no final newline; refusing to alter prior rows")
    fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    with os.fdopen(fd, "ab") as stream:
        stream.write(("\n".join(rows) + "\n").encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())


def verify_prefix(label: str, path: Path) -> None:
    expected = PREFIX[label]
    rows = path.read_bytes().splitlines(keepends=True)
    if len(rows) < expected["line_count"] or digest(
        b"".join(rows[: expected["line_count"]])
    ) != expected["sha256"]:
        raise SystemExit(f"historical {label} prefix changed")


for path in (LOCAL_INDEX, WORKTREE_INDEX, PROJECT_INDEX):
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"missing or unsafe manifest: {path}")

sources = parse(ROOT / "source-sha256.txt")
if not sources:
    raise SystemExit("source hash list is empty")
for name, expected in sources.items():
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file() or digest(path.read_bytes()) != expected:
        raise SystemExit(f"source hash mismatch: {name}")

files: list[Path] = []
for path in sorted(ROOT.rglob("*"), key=lambda item: item.relative_to(ROOT).as_posix()):
    if path.is_symlink():
        raise SystemExit(f"unexpected symlink: {path.relative_to(ROOT)}")
    if path == LOCAL_INDEX:
        continue
    if path.is_file():
        files.append(path)
    elif not path.is_dir():
        raise SystemExit(f"unexpected evidence entry: {path.relative_to(ROOT)}")

desired_local = {
    path.relative_to(ROOT).as_posix(): digest(path.read_bytes()) for path in files
}
local_before = parse(LOCAL_INDEX)
append_rows(
    LOCAL_INDEX,
    [
        f"{value}  {name}"
        for name, value in sorted(desired_local.items())
        if local_before.get(name) != value
    ],
)
local_latest = parse(LOCAL_INDEX)
for name, expected in desired_local.items():
    if local_latest.get(name) != expected or digest((ROOT / name).read_bytes()) != expected:
        raise SystemExit(f"latest local digest mismatch: {name}")

worktree_before = parse(WORKTREE_INDEX)
desired_worktree = {
    f"{EVIDENCE_PREFIX}/{name}": value for name, value in local_latest.items()
}
desired_worktree[f"{EVIDENCE_PREFIX}/SHA256SUMS"] = digest(LOCAL_INDEX.read_bytes())
desired_worktree.update(sources)
append_rows(
    WORKTREE_INDEX,
    [
        f"{value}  {name}"
        for name, value in sorted(desired_worktree.items())
        if worktree_before.get(name) != value
    ],
)
worktree_latest = parse(WORKTREE_INDEX)

project_before = parse(PROJECT_INDEX)
desired_project = {
    f"{WORKTREE_PREFIX}/{name}": value for name, value in desired_worktree.items()
}
desired_project[f"{WORKTREE_PREFIX}/evidence/SHA256SUMS"] = digest(
    WORKTREE_INDEX.read_bytes()
)
append_rows(
    PROJECT_INDEX,
    [
        f"{value}  {name}"
        for name, value in sorted(desired_project.items())
        if project_before.get(name) != value
    ],
)
project_latest = parse(PROJECT_INDEX)

verify_prefix("worktree", WORKTREE_INDEX)
verify_prefix("project", PROJECT_INDEX)
for name, expected in local_latest.items():
    if worktree_latest.get(f"{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"worktree index missing latest evidence entry: {name}")
    if project_latest.get(f"{WORKTREE_PREFIX}/{EVIDENCE_PREFIX}/{name}") != expected:
        raise SystemExit(f"project index missing latest evidence entry: {name}")
if worktree_latest.get(f"{EVIDENCE_PREFIX}/SHA256SUMS") != digest(LOCAL_INDEX.read_bytes()):
    raise SystemExit("worktree index does not cover the latest local index")
if project_latest.get(f"{WORKTREE_PREFIX}/evidence/SHA256SUMS") != digest(
    WORKTREE_INDEX.read_bytes()
):
    raise SystemExit("project index does not cover the latest worktree index")
for name, expected in sources.items():
    if worktree_latest.get(name) != expected:
        raise SystemExit(f"worktree source index mismatch: {name}")
    if project_latest.get(f"{WORKTREE_PREFIX}/{name}") != expected:
        raise SystemExit(f"project source index mismatch: {name}")

print(
    json.dumps(
        {
            "status": "verified",
            "latest_local_entries": len(local_latest),
            "source_entries": len(sources),
            "historical_prefixes_preserved": True,
        },
        sort_keys=True,
    )
)
