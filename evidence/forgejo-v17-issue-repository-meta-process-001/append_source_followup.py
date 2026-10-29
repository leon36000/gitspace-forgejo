#!/usr/bin/env python3
"""Append the upstream route/converter source addendum without rewriting indexes."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat


RUN = Path(__file__).resolve().parent
WORKTREE = RUN.parents[1]
PROJECT = WORKTREE.parents[1]
FOLLOWUP = RUN / "upstream-route-followup.md"
PREFIX = RUN / "followup-manifest-prefix-before.json"
CHECK = RUN / "followup-manifest-check.txt"
LOCAL_INDEX = RUN / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXTRA_PATHS = (Path("docs/checkpoint.md"), Path("docs/source-inventory.md"))


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_file(path: Path) -> str:
    descriptor = os.open(
        path,
        os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
    )
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise OSError(f"not a regular file: {path}")
        value = hashlib.sha256()
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            while chunk := source.read(1024 * 1024):
                value.update(chunk)
        return value.hexdigest()
    finally:
        os.close(descriptor)


def append(index: Path, entries: list[tuple[str, str]]) -> None:
    with index.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


def main() -> int:
    if any(path.exists() or path.is_symlink() for path in (PREFIX, CHECK)):
        raise SystemExit("refusing to overwrite existing source-followup receipts")
    for index in (LOCAL_INDEX, WORKTREE_INDEX, PROJECT_INDEX):
        if not index.is_file() or index.is_symlink():
            raise SystemExit(f"manifest unavailable: {index}")
        if index.read_bytes() and not index.read_bytes().endswith(b"\n"):
            raise SystemExit(f"manifest has no final newline: {index}")

    before = {}
    for label, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        data = index.read_bytes()
        before[label] = {"line_count": len(data.splitlines()), "sha256": digest_bytes(data)}
    PREFIX.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

    local_paths = (FOLLOWUP, Path(__file__).resolve(), PREFIX)
    local_entries = [(digest_file(path), path.relative_to(RUN).as_posix()) for path in local_paths]
    append(LOCAL_INDEX, local_entries)
    local_manifest_digest = digest_file(LOCAL_INDEX)

    worktree_entries = [
        (value, f"evidence/{RUN.name}/{name}") for value, name in local_entries
    ]
    worktree_entries.append(
        (local_manifest_digest, f"evidence/{RUN.name}/SHA256SUMS")
    )
    project_entries = [
        (value, f".runtime/webhook-005-workspace/evidence/{RUN.name}/{name}")
        for value, name in local_entries
    ]
    project_entries.append(
        (local_manifest_digest, f".runtime/webhook-005-workspace/evidence/{RUN.name}/SHA256SUMS")
    )
    for relative in EXTRA_PATHS:
        value = digest_file(WORKTREE / relative)
        worktree_entries.append((value, relative.as_posix()))
        project_entries.append(
            (value, f".runtime/webhook-005-workspace/{relative.as_posix()}")
        )

    append(WORKTREE_INDEX, worktree_entries)
    append(
        PROJECT_INDEX,
        project_entries
        + [(digest_file(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")],
    )

    for label, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        data = index.read_bytes()
        old_count = before[label]["line_count"]
        prefix = b"".join(data.splitlines(keepends=True)[:old_count])
        if digest_bytes(prefix) != before[label]["sha256"]:
            raise SystemExit(f"append-only prefix changed: {label}")
    for value, relative in local_entries:
        if digest_file(RUN / relative) != value:
            raise SystemExit(f"source addendum digest mismatch: {relative}")

    CHECK.write_text(
        "Source follow-up evidence hashes: PASS\n"
        "worktree and project manifest prefixes unchanged: PASS\n"
        f"local source-followup entries appended={len(local_entries)}\n",
        encoding="utf-8",
    )
    print(json.dumps({"append_only_prefixes": "verified", "local_entries": len(local_entries)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
