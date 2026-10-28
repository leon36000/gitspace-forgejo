#!/usr/bin/env python3
"""Append CONTINUATION-280 evidence and source hashes without rewriting prefixes."""
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
PREFIX_PATH = ROOT / "manifest-prefix-before.json"
WORKTREE_PREFIX = WORKTREE.relative_to(PROJECT).as_posix()
SOURCE_NAMES = [
    "Cargo.toml",
    "scripts/test_cargo_license_state.py",
    "scripts/verify_cargo_license_state.py",
    "src/webhook/reconcile.rs",
    "tests/webhook_reconcile_contract.rs",
    "docs/architecture.md",
    "docs/checkpoint.md",
]


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
        raise SystemExit(f"{path} has no final newline; refusing to alter prior row")
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "ab") as stream:
        stream.write(("\n".join(rows) + "\n").encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())


if not PREFIX_PATH.exists():
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
    write_exclusive(
        PREFIX_PATH,
        (json.dumps(prefix, indent=2, sort_keys=True) + "\n").encode("utf-8"),
    )
prefix = json.loads(PREFIX_PATH.read_text(encoding="utf-8"))

source_rows = []
for name in SOURCE_NAMES:
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"missing/non-regular source: {name}")
    source_rows.append(f"{digest(path.read_bytes())}  {name}")
source_data = ("\n".join(source_rows) + "\n").encode("ascii")
source_path = ROOT / "source-sha256.txt"
if source_path.exists():
    if source_path.is_symlink() or source_path.read_bytes() != source_data:
        raise SystemExit("pre-existing source hash file differs; refusing to rewrite it")
else:
    write_exclusive(source_path, source_data)

local_manifest = ROOT / "SHA256SUMS"
if local_manifest.exists() or local_manifest.is_symlink():
    raise SystemExit("local evidence manifest exists; refusing to rewrite it")
local_paths = []
for path in sorted(ROOT.rglob("*"), key=lambda item: item.relative_to(ROOT).as_posix()):
    if path.is_symlink():
        raise SystemExit(f"unexpected evidence symlink: {path.relative_to(ROOT).as_posix()}")
    if path.is_file():
        local_paths.append(path)
    elif not path.is_dir():
        raise SystemExit(f"unexpected evidence entry: {path.relative_to(ROOT).as_posix()}")
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


def preserve_prefix(label: str, path: Path) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    count = prefix[label]["line_count"]
    if len(lines) < count or digest(b"".join(lines[:count])) != prefix[label]["sha256"]:
        raise SystemExit(f"historical {label} manifest prefix changed")


preserve_prefix("worktree", WORKTREE_INDEX)
preserve_prefix("project", PROJECT_INDEX)
worktree_before = parse(WORKTREE_INDEX)
project_before = parse(PROJECT_INDEX)
paths = set(local_paths) | {local_manifest}
for line in source_rows:
    expected, name = line.split("  ", 1)
    source = WORKTREE / name
    if digest(source.read_bytes()) != expected:
        raise SystemExit(f"source changed before append: {name}")
    paths.add(source)

worktree_rows = {
    path.relative_to(WORKTREE).as_posix(): digest(path.read_bytes()) for path in paths
}
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
preserve_prefix("worktree", WORKTREE_INDEX)
preserve_prefix("project", PROJECT_INDEX)
print(
    json.dumps(
        {"status": "appended", "local_entries": len(local_paths), "prefixes_preserved": True},
        sort_keys=True,
    )
)
