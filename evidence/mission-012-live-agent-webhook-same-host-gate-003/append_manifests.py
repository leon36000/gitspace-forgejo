#!/usr/bin/env python3
"""Append this run's local and parent evidence hashes without rewriting prefixes."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import stat


ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
ADMISSION_DIRS = [
    WORKTREE / "evidence" / f"mission-012-live-agent-webhook-same-host-gate-{number}-admission"
    for number in ("001", "002", "003")
]
DOCUMENTS = (
    "docs/architecture.md",
    "docs/checkpoint.md",
    "docs/refusal-evidence-matrix.md",
)


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha(path: Path) -> str:
    return sha_bytes(path.read_bytes())


def latest(path: Path) -> dict[str, str]:
    entries: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line:
            continue
        digest, separator, name = line.partition("  ")
        # Preserve historical malformed records (some old entries contain
        # `None`) and let the new real digest supersede them append-only.
        if not separator or not digest or not name:
            raise SystemExit(f"malformed manifest line in {path}")
        entries[name] = digest
    return entries


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if (
        len(lines) < expected["line_count"]
        or sha_bytes(b"".join(lines[: expected["line_count"]])) != expected["sha256"]
    ):
        raise SystemExit(f"{key} append-only prefix changed")


def regular_files(folder: Path) -> list[Path]:
    if folder.is_symlink() or not folder.is_dir():
        raise SystemExit(f"evidence directory is missing or redirected: {folder}")
    paths: list[Path] = []
    for path in sorted(folder.rglob("*")):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise SystemExit(f"symlink in evidence tree: {path.relative_to(folder)}")
        if stat.S_ISREG(metadata.st_mode):
            paths.append(path)
        elif not stat.S_ISDIR(metadata.st_mode):
            raise SystemExit(f"non-regular evidence entry: {path.relative_to(folder)}")
    return paths


def local_manifest_records(folder: Path) -> list[tuple[str, Path]]:
    manifest = folder / "SHA256SUMS"
    if manifest.is_symlink() or not manifest.is_file():
        raise SystemExit(f"local evidence manifest is missing or redirected: {folder}")
    records: list[tuple[str, Path]] = []
    seen: set[str] = set()
    for name, expected in latest(manifest).items():
        if (
            Path(name).name != name
            or name in {".", "..", "SHA256SUMS"}
            or name in seen
            or re.fullmatch(r"[A-Za-z0-9._-]{1,128}", name) is None
        ):
            raise SystemExit(f"unsafe local evidence manifest entry: {name}")
        seen.add(name)
        path = folder / name
        if path.is_symlink() or not path.is_file() or sha(path) != expected:
            raise SystemExit(f"local evidence hash mismatch: {path}")
        records.append((expected, path))
    actual = {
        path.name for path in regular_files(folder)
        if path != manifest
    }
    if actual != seen:
        raise SystemExit(f"local evidence manifest coverage differs for {folder}")
    return records


for index, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    if index.is_symlink() or not index.is_file():
        raise SystemExit(f"{key} manifest is missing or redirected")
    check_prefix(index, key)

local_latest = latest(LOCAL)
new_local_lines: list[str] = []
for path in regular_files(ROOT):
    if path == LOCAL:
        continue
    relative = path.relative_to(ROOT).as_posix()
    digest = sha(path)
    if local_latest.get(relative) != digest:
        new_local_lines.append(f"{digest}  {relative}\n")
if new_local_lines:
    with LOCAL.open("ab") as stream:
        stream.writelines(line.encode("ascii") for line in new_local_lines)
        stream.flush()
        os.fsync(stream.fileno())

records: list[tuple[str, str, str]] = []
for digest, path in local_manifest_records(ROOT):
    work_relative = path.relative_to(WORKTREE).as_posix()
    project_relative = (Path(".runtime/webhook-005-workspace") / work_relative).as_posix()
    records.append((digest, work_relative, project_relative))
local_relative = LOCAL.relative_to(WORKTREE).as_posix()
records.append((sha(LOCAL), local_relative, (Path(".runtime/webhook-005-workspace") / local_relative).as_posix()))

for folder in ADMISSION_DIRS:
    for digest, path in local_manifest_records(folder):
        work_relative = path.relative_to(WORKTREE).as_posix()
        project_relative = (Path(".runtime/webhook-005-workspace") / work_relative).as_posix()
        records.append((digest, work_relative, project_relative))
    manifest = folder / "SHA256SUMS"
    work_relative = manifest.relative_to(WORKTREE).as_posix()
    project_relative = (Path(".runtime/webhook-005-workspace") / work_relative).as_posix()
    records.append((sha(manifest), work_relative, project_relative))

for name in DOCUMENTS:
    path = WORKTREE / name
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"documentation path is missing or redirected: {name}")
    records.append((sha(path), name, (Path(".runtime/webhook-005-workspace") / name).as_posix()))

worktree_latest = latest(WORKTREE_INDEX)
worktree_lines = [
    f"{digest}  {work_relative}\n"
    for digest, work_relative, _ in records
    if worktree_latest.get(work_relative) != digest
]
if worktree_lines:
    with WORKTREE_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(worktree_lines)
        stream.flush()
        os.fsync(stream.fileno())

project_records = [(digest, project_relative) for digest, _, project_relative in records]
project_records.append((sha(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS"))
project_latest = latest(PROJECT_INDEX)
project_lines = [
    f"{digest}  {project_relative}\n"
    for digest, project_relative in project_records
    if project_latest.get(project_relative) != digest
]
if project_lines:
    with PROJECT_INDEX.open("a", encoding="utf-8") as stream:
        stream.writelines(project_lines)
        stream.flush()
        os.fsync(stream.fileno())

for index, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(index, key)
local_names = {
    path.relative_to(ROOT).as_posix()
    for path in regular_files(ROOT)
    if path != LOCAL
}
if local_names != set(latest(LOCAL)):
    raise SystemExit("run-local manifest does not cover every evidence file")
worktree_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for digest, work_relative, project_relative in records:
    if worktree_latest.get(work_relative) != digest or project_latest.get(project_relative) != digest:
        raise SystemExit(f"global manifest mismatch: {work_relative}")
if project_latest.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORKTREE_INDEX):
    raise SystemExit("parent project index does not bind the current worktree index")

print(json.dumps({
    "local_entries_appended": len(new_local_lines),
    "worktree_entries_appended": len(worktree_lines),
    "project_entries_appended": len(project_lines),
    "local_files_verified": len(local_names),
    "admission_directories_verified": len(ADMISSION_DIRS),
    "prefixes_preserved": True,
}, sort_keys=True))
