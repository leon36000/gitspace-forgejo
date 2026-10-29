#!/usr/bin/env python3
"""Append final receipt revisions after the initial package was indexed."""

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
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = RUN / "manifest-prefix-before.json"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"


def digest_file(path: Path) -> str:
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0))
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise OSError(f"not a regular file: {path}")
        digest = hashlib.sha256()
        with os.fdopen(fd, "rb", closefd=False) as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
        return digest.hexdigest()
    finally:
        os.close(fd)


def append_bytes(path: Path, payload: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0))
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise OSError(f"not a regular append target: {path}")
        view = memoryview(payload)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                raise OSError(f"short append to {path}")
            view = view[written:]
        os.fsync(fd)
    finally:
        os.close(fd)


def suffix_after_prefix(path: Path, label: str, before: dict[str, object]) -> None:
    raw = path.read_bytes()
    lines = raw.splitlines(keepends=True)
    count = int(before[label]["line_count"])
    if len(lines) < count:
        raise SystemExit(f"pre-existing manifest records were lost: {label}")
    prefix = b"".join(lines[:count])
    if hashlib.sha256(prefix).hexdigest() != before[label]["sha256"]:
        raise SystemExit(f"append-only prefix changed: {label}")


def verify(index: Path, root: Path) -> dict[str, object]:
    result = subprocess.run(
        [sys.executable, str(CHECKER), str(index), "--root", str(root)],
        cwd=WORKTREE,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=120,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stdout)
    return json.loads(result.stdout)


def main() -> int:
    before = json.loads(PREFIX.read_text(encoding="utf-8"))
    if LOCAL.is_symlink() or not stat.S_ISREG(LOCAL.lstat().st_mode):
        raise SystemExit("local receipt manifest is missing or unsafe")
    if WORKTREE_INDEX.is_symlink() or PROJECT_INDEX.is_symlink():
        raise SystemExit("refusing a symlinked global index")
    for label, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        suffix_after_prefix(index, label, before)

    updated = [
        RUN / "local-run.md",
        RUN / "commands.txt",
        Path(__file__).resolve(),
    ]
    rows = [(digest_file(path), path.relative_to(RUN).as_posix()) for path in updated]
    local_raw = LOCAL.read_bytes()
    if not local_raw.endswith(b"\n"):
        raise SystemExit("local receipt manifest lacks final newline")
    append_bytes(LOCAL, "".join(f"{digest}  {name}\n" for digest, name in rows).encode("utf-8"))

    package = RUN.relative_to(WORKTREE).as_posix()
    worktree_rows = [
        (digest, f"{package}/{name}") for digest, name in rows
    ]
    local_digest = digest_file(LOCAL)
    worktree_rows.append((local_digest, f"{package}/SHA256SUMS"))

    project_rows = [
        (digest, f".runtime/webhook-005-workspace/{package}/{name}")
        for digest, name in rows
    ]
    project_rows.append((local_digest, f".runtime/webhook-005-workspace/{package}/SHA256SUMS"))

    append_bytes(
        WORKTREE_INDEX,
        "".join(f"{digest}  {name}\n" for digest, name in worktree_rows).encode("utf-8"),
    )
    project_rows.append(
        (digest_file(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")
    )
    append_bytes(
        PROJECT_INDEX,
        "".join(f"{digest}  {name}\n" for digest, name in project_rows).encode("utf-8"),
    )

    for label, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        suffix_after_prefix(index, label, before)
    result = {
        "append_only_prefixes": "pass",
        "worktree_manifest": verify(WORKTREE_INDEX, WORKTREE),
        "project_manifest": verify(PROJECT_INDEX, PROJECT),
        "receipt_revision_rows": len(rows),
    }
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
