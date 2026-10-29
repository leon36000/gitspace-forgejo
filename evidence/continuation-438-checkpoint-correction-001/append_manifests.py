#!/usr/bin/env python3
"""Append the CONTINUATION-438 checkpoint correction to SHA-256 indexes."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys


RUN = Path(__file__).resolve().parent
WORKTREE = RUN.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = RUN / "SHA256SUMS"
PREFIX = RUN / "manifest-prefix-before.json"
CHECK = RUN / "manifest-check.txt"
GLOBAL_CHECK = RUN / "global-manifest-check.txt"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def hash_file(path: Path) -> str:
    descriptor = os.open(
        path,
        os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
    )
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise OSError(f"not a regular file: {path}")
        hasher = hashlib.sha256()
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            while chunk := source.read(1024 * 1024):
                hasher.update(chunk)
        return hasher.hexdigest()
    finally:
        os.close(descriptor)


def append(index: Path, entries: list[tuple[str, str]]) -> None:
    with index.open("ab") as output:
        for value, name in entries:
            output.write(f"{value}  {name}\n".encode("utf-8"))
        output.flush()
        os.fsync(output.fileno())


def check(index: Path, root: Path) -> str:
    completed = subprocess.run(
        [sys.executable, str(CHECKER), str(index), "--root", str(root)],
        cwd=WORKTREE,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=60,
        check=False,
    )
    if completed.returncode:
        raise RuntimeError(completed.stdout)
    return completed.stdout


def main() -> int:
    for path in (LOCAL, PREFIX, CHECK, GLOBAL_CHECK):
        if path.exists() or path.is_symlink():
            raise SystemExit(f"refusing to overwrite {path.name}")
    indexes = (WORKTREE_INDEX, PROJECT_INDEX)
    before: dict[str, dict[str, object]] = {}
    for label, index in zip(("worktree", "project"), indexes, strict=True):
        raw = index.read_bytes()
        if not raw.endswith(b"\n"):
            raise SystemExit(f"index has no final newline: {index}")
        before[label] = {"line_count": len(raw.splitlines()), "sha256": digest(raw)}
    PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    files = [RUN / "append_manifests.py", RUN / "commands.txt", RUN / "local-run.md", PREFIX]
    local_entries = [(hash_file(path), path.relative_to(RUN).as_posix()) for path in files]
    LOCAL.write_text(
        "".join(f"{value}  {name}\n" for value, name in local_entries), encoding="utf-8"
    )
    local_digest = hash_file(LOCAL)
    worktree_entries = [
        (value, f"evidence/{RUN.name}/{name}") for value, name in local_entries
    ]
    worktree_entries.append((local_digest, f"evidence/{RUN.name}/SHA256SUMS"))
    checkpoint_digest = hash_file(WORKTREE / "docs/checkpoint.md")
    worktree_entries.append((checkpoint_digest, "docs/checkpoint.md"))
    project_entries = [
        (value, f".runtime/webhook-005-workspace/evidence/{RUN.name}/{name}")
        for value, name in local_entries
    ]
    project_entries.append(
        (local_digest, f".runtime/webhook-005-workspace/evidence/{RUN.name}/SHA256SUMS")
    )
    project_entries.append(
        (checkpoint_digest, ".runtime/webhook-005-workspace/docs/checkpoint.md")
    )

    append(WORKTREE_INDEX, worktree_entries)
    append(
        PROJECT_INDEX,
        project_entries
        + [
            (
                hash_file(WORKTREE_INDEX),
                ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
            )
        ],
    )

    for label, index in zip(("worktree", "project"), indexes, strict=True):
        raw = index.read_bytes()
        count = int(before[label]["line_count"])
        if digest(b"".join(raw.splitlines(keepends=True)[:count])) != before[label]["sha256"]:
            raise RuntimeError(f"append-only prefix changed: {label}")
    for value, name in local_entries:
        if hash_file(RUN / name) != value:
            raise RuntimeError(f"local evidence digest mismatch: {name}")
    if hash_file(WORKTREE / "docs/checkpoint.md") != checkpoint_digest:
        raise RuntimeError("checkpoint digest changed during append")

    worktree_check = check(WORKTREE_INDEX, WORKTREE)
    project_check = check(PROJECT_INDEX, PROJECT)
    CHECK.write_text(
        "checkpoint and local evidence hashes: PASS\n"
        "worktree and project append-only prefixes: PASS\n",
        encoding="utf-8",
    )
    GLOBAL_CHECK.write_text(
        f"worktree latest SHA-256 manifest: PASS\n{worktree_check}"
        f"project latest SHA-256 manifest: PASS\n{project_check}",
        encoding="utf-8",
    )
    print(json.dumps({"worktree": "pass", "project": "pass", "prefixes": "verified"}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
