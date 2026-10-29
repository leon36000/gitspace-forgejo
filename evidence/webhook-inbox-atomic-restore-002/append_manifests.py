#!/usr/bin/env python3
"""Create this evidence package and append its hashes to the canonical indexes."""

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
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"
PREFIX = RUN / "manifest-prefix-before.json"
TOUCHED = (
    WORKTREE / "docs/checkpoint.md",
    WORKTREE / "docs/refusal-evidence-matrix.md",
    WORKTREE / "src/bin/forgejo-webhook-local.rs",
    WORKTREE / "src/bin/forgejo-reconcile-local.rs",
    WORKTREE / "tests/webhook_inbox_snapshot.rs",
)


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
    descriptor = os.open(
        index,
        os.O_WRONLY | os.O_APPEND | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
    )
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise OSError(f"not a regular manifest: {index}")
        payload = "".join(f"{digest}  {name}\n" for digest, name in rows).encode("utf-8")
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise OSError("manifest append made no progress")
            view = view[written:]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def verify(index: Path, root: Path) -> dict[str, object]:
    result = subprocess.run(
        [sys.executable, str(CHECKER), str(index), "--root", str(root)],
        cwd=WORKTREE,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(f"manifest verifier failed to execute: {result.returncode}")
    return json.loads(result.stdout)


def main() -> int:
    package_index = RUN / "SHA256SUMS"
    if package_index.exists():
        metadata = package_index.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != 0:
            raise RuntimeError("this package manifest is already finalized; use an append-only refinement")

    indexes = (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX))
    before: dict[str, object] = {}
    for name, index in indexes:
        raw = index.read_bytes()
        before[name] = {
            "line_count": len(raw.splitlines()),
            "sha256": hashlib.sha256(raw).hexdigest(),
        }
    PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    package_files = []
    for path in sorted(RUN.iterdir()):
        if path.name == "SHA256SUMS":
            continue
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode):
            raise OSError(f"refusing non-regular evidence path: {path}")
        package_files.append(path)

    package_rows = [
        (hash_file(path), path.relative_to(RUN).as_posix()) for path in package_files
    ]
    if package_index.exists():
        metadata = package_index.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != 0:
            raise RuntimeError("refusing to rewrite a non-empty or non-regular package manifest")
        package_mode = "ab"
    else:
        package_mode = "xb"
    with package_index.open(package_mode) as output:
        for digest, name in package_rows:
            output.write(f"{digest}  {name}\n".encode("utf-8"))
        output.flush()
        os.fsync(output.fileno())

    relative_package = RUN.relative_to(WORKTREE).as_posix()
    worktree_rows = [
        (digest, f"{relative_package}/{name}") for digest, name in package_rows
    ]
    project_rows = [
        (digest, f".runtime/webhook-005-workspace/{relative_package}/{name}")
        for digest, name in package_rows
    ]
    manifest_digest = hash_file(package_index)
    worktree_rows.append((manifest_digest, f"{relative_package}/SHA256SUMS"))
    project_rows.append(
        (manifest_digest, f".runtime/webhook-005-workspace/{relative_package}/SHA256SUMS")
    )

    for path in TOUCHED:
        digest = hash_file(path)
        relative = path.relative_to(WORKTREE).as_posix()
        worktree_rows.append((digest, relative))
        project_rows.append((digest, f".runtime/webhook-005-workspace/{relative}"))

    append(WORKTREE_INDEX, worktree_rows)
    project_rows.append(
        (
            hash_file(WORKTREE_INDEX),
            ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
        )
    )
    append(PROJECT_INDEX, project_rows)

    for name, index in indexes:
        raw = index.read_bytes()
        prior_count = int(before[name]["line_count"])
        prefix = b"".join(raw.splitlines(keepends=True)[:prior_count])
        if hashlib.sha256(prefix).hexdigest() != before[name]["sha256"]:
            raise RuntimeError(f"append-only prefix changed: {name}")

    results = {
        "append_only_prefixes": "pass",
        "worktree": verify(WORKTREE_INDEX, WORKTREE),
        "project": verify(PROJECT_INDEX, PROJECT),
        "package": verify(package_index, RUN),
    }
    print(json.dumps(results, sort_keys=True))
    return 0 if all(value["passed"] for key, value in results.items() if key != "append_only_prefixes") else 1


if __name__ == "__main__":
    raise SystemExit(main())
