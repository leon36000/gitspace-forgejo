#!/usr/bin/env python3
"""Hash this evidence package and append final source hashes to both indexes."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys


RUN = Path(__file__).resolve().parent
WORKTREE = RUN.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = RUN / "manifest-prefix-before.json"
LOCAL = RUN / "SHA256SUMS"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"

TOUCHED = (
    "src/bin/forgejo-reconcile-local.rs",
    "tests/forgejo_merge_post_crash_recovery_process_e2e.rs",
    "scripts/run_forgejo_mission_delivery_process_e2e.py",
    "docs/checkpoint.md",
    "docs/architecture.md",
    "docs/refusal-evidence-matrix.md",
)


def sha256(path: Path) -> str:
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


def write_all(fd: int, data: bytes) -> None:
    remaining = memoryview(data)
    while remaining:
        count = os.write(fd, remaining)
        if count <= 0:
            raise OSError("short append to manifest")
        remaining = remaining[count:]
    os.fsync(fd)


def read_index(path: Path) -> bytes:
    if path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode):
        raise SystemExit(f"refusing unsafe index: {path}")
    data = path.read_bytes()
    if not data.endswith(b"\n"):
        raise SystemExit(f"index lacks final newline: {path}")
    return data


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


def append_if_needed(index: Path, before: dict[str, object], rows: list[tuple[str, str]]) -> None:
    raw = read_index(index)
    lines = raw.splitlines(keepends=True)
    old_count = int(before["line_count"])
    if len(lines) < old_count:
        raise SystemExit(f"index lost previous rows: {index}")
    prefix = b"".join(lines[:old_count])
    if hashlib.sha256(prefix).hexdigest() != before["sha256"]:
        raise SystemExit(f"append-only prefix changed: {index}")
    latest: dict[str, str] = {}
    for line in lines[old_count:]:
        match = re.fullmatch(rb"([0-9a-f]{64})  (.+)\n", line)
        if match:
            latest[match.group(2).decode("utf-8")] = match.group(1).decode("ascii")
        else:
            raise SystemExit(f"unexpected malformed suffix row in {index}")
    pending = [(digest, name) for digest, name in rows if latest.get(name) != digest]
    if not pending:
        return
    expected = "".join(f"{digest}  {name}\n" for digest, name in pending).encode("utf-8")
    fd = os.open(index, os.O_WRONLY | os.O_APPEND | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0))
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise OSError(f"not a regular index: {index}")
        write_all(fd, expected)
    finally:
        os.close(fd)


def main() -> int:
    if not PREFIX.exists():
        before = {}
        for label, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
            raw = read_index(index)
            before[label] = {
                "line_count": len(raw.splitlines()),
                "sha256": hashlib.sha256(raw).hexdigest(),
            }
        PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    else:
        before = json.loads(PREFIX.read_text(encoding="utf-8"))

    files = []
    for path in sorted(RUN.iterdir()):
        if path.name in {"SHA256SUMS", "__pycache__"}:
            continue
        if path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode):
            raise SystemExit(f"refusing non-regular evidence path: {path}")
        files.append(path)
    package_rows = [(sha256(path), path.relative_to(RUN).as_posix()) for path in files]
    package_content = "".join(f"{digest}  {name}\n" for digest, name in package_rows).encode()
    if LOCAL.exists():
        if LOCAL.is_symlink() or not stat.S_ISREG(LOCAL.lstat().st_mode):
            raise SystemExit("refusing unsafe local package manifest")
        if LOCAL.read_bytes() != package_content:
            fd = os.open(LOCAL, os.O_WRONLY | os.O_TRUNC | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0))
            try:
                if not stat.S_ISREG(os.fstat(fd).st_mode):
                    raise OSError("local package manifest is not regular")
                write_all(fd, package_content)
            finally:
                os.close(fd)
    else:
        fd = os.open(LOCAL, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            write_all(fd, package_content)
        finally:
            os.close(fd)

    package_name = RUN.relative_to(WORKTREE).as_posix()
    local_digest = sha256(LOCAL)
    worktree_rows = [(digest, f"{package_name}/{name}") for digest, name in package_rows]
    worktree_rows.extend(
        (sha256(WORKTREE / name), name) for name in TOUCHED
    )
    worktree_rows.append((local_digest, f"{package_name}/SHA256SUMS"))
    project_rows = [
        (sha256(WORKTREE / name), f".runtime/webhook-005-workspace/{name}") for name in TOUCHED
    ]
    project_rows.extend(
        (digest, f".runtime/webhook-005-workspace/{package_name}/{name}")
        for digest, name in package_rows
    )
    project_rows.append((local_digest, f".runtime/webhook-005-workspace/{package_name}/SHA256SUMS"))

    append_if_needed(WORKTREE_INDEX, before["worktree"], worktree_rows)
    project_rows.append((sha256(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS"))
    append_if_needed(PROJECT_INDEX, before["project"], project_rows)
    print(json.dumps({
        "package_files": len(package_rows),
        "worktree": verify(WORKTREE_INDEX, WORKTREE),
        "project": verify(PROJECT_INDEX, PROJECT),
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
