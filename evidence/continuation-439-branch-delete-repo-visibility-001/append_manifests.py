#!/usr/bin/env python3
"""Append both evidence packages and touched sources to the SHA-256 indexes."""

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
GITLAB_RUN = WORKTREE / "evidence/gitlab-pc4-security-review-001"
TRANSFER004_EXTRA = (
    WORKTREE / "evidence/gitlab-pc4-transfer-004/remote-readback.json",
    WORKTREE / "evidence/gitlab-pc4-transfer-004/SHA256SUMS",
)
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"
PREFIX = RUN / "manifest-prefix-before.json"


def hash_file(path: Path) -> str:
    descriptor = os.open(
        path,
        os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
    )
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise OSError(f"not a regular file: {path}")
        digest = hashlib.sha256()
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
        return digest.hexdigest()
    finally:
        os.close(descriptor)


def append(index: Path, rows: list[tuple[str, str]]) -> None:
    with index.open("ab") as output:
        for digest, name in rows:
            output.write(f"{digest}  {name}\n".encode("utf-8"))
        output.flush()
        os.fsync(output.fileno())


def current_package_files(directory: Path) -> list[Path]:
    files = []
    for path in sorted(directory.iterdir()):
        if path.name == "SHA256SUMS":
            continue
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode):
            raise SystemExit(f"refusing non-regular evidence path: {path}")
        files.append(path)
    return files


def package_manifest(directory: Path) -> list[tuple[str, str]]:
    rows = [
        (hash_file(path), path.relative_to(directory).as_posix())
        for path in current_package_files(directory)
    ]
    manifest = directory / "SHA256SUMS"
    with manifest.open("xb") as output:
        for digest, name in rows:
            output.write(f"{digest}  {name}\n".encode("utf-8"))
        output.flush()
        os.fsync(output.fileno())
    return rows


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
    packages = (RUN, GITLAB_RUN)
    if PREFIX.exists() or PREFIX.is_symlink():
        raise SystemExit(f"refusing to overwrite {PREFIX}")
    for package in packages:
        if not package.is_dir() or (package / "SHA256SUMS").exists():
            raise SystemExit(f"evidence package missing or already indexed: {package}")

    indexes = (WORKTREE_INDEX, PROJECT_INDEX)
    before = {}
    for name, index in zip(("worktree", "project"), indexes, strict=True):
        raw = index.read_bytes()
        if not raw.endswith(b"\n"):
            raise SystemExit(f"manifest has no final newline: {index}")
        before[name] = {
            "line_count": len(raw.splitlines()),
            "sha256": hashlib.sha256(raw).hexdigest(),
        }
    PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    package_rows = {package: package_manifest(package) for package in packages}
    worktree_rows: list[tuple[str, str]] = []
    project_rows: list[tuple[str, str]] = []
    for package in packages:
        rel = package.relative_to(WORKTREE).as_posix()
        for digest, name in package_rows[package]:
            worktree_rows.append((digest, f"{rel}/{name}"))
            project_rows.append((digest, f".runtime/webhook-005-workspace/{rel}/{name}"))
        manifest_digest = hash_file(package / "SHA256SUMS")
        worktree_rows.append((manifest_digest, f"{rel}/SHA256SUMS"))
        project_rows.append((manifest_digest, f".runtime/webhook-005-workspace/{rel}/SHA256SUMS"))

    touched = (
        WORKTREE / "docs/checkpoint.md",
        WORKTREE / "docs/refusal-evidence-matrix.md",
        WORKTREE / "scripts/run_webhook_reconcile_cli_e2e.py",
        WORKTREE / "tests/webhook_receiver_process_e2e.rs",
        *TRANSFER004_EXTRA,
    )
    for path in touched:
        digest = hash_file(path)
        relative = path.relative_to(WORKTREE).as_posix()
        worktree_rows.append((digest, relative))
        project_rows.append((digest, f".runtime/webhook-005-workspace/{relative}"))

    append(WORKTREE_INDEX, worktree_rows)
    append(
        PROJECT_INDEX,
        project_rows
        + [
            (
                hash_file(WORKTREE_INDEX),
                ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
            )
        ],
    )

    for name, index in zip(("worktree", "project"), indexes, strict=True):
        raw = index.read_bytes()
        count = int(before[name]["line_count"])
        prefix = b"".join(raw.splitlines(keepends=True)[:count])
        if hashlib.sha256(prefix).hexdigest() != before[name]["sha256"]:
            raise RuntimeError(f"append-only prefix changed: {name}")
    worktree_result = verify(WORKTREE_INDEX, WORKTREE)
    project_result = verify(PROJECT_INDEX, PROJECT)
    print(
        json.dumps(
            {
                "append_only_prefixes": "pass",
                "worktree_manifest": worktree_result,
                "project_manifest": project_result,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
