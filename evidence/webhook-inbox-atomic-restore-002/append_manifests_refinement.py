#!/usr/bin/env python3
"""Append final evidence and source hashes without rewriting any manifest prefix."""

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
PACKAGE_INDEX = RUN / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"
PREFIX = RUN / "manifest-prefix-refinement-001.json"
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
            raise OSError(f"not a regular index: {index}")
        data = "".join(f"{digest}  {name}\n" for digest, name in rows).encode("utf-8")
        view = memoryview(data)
        while view:
            count = os.write(descriptor, view)
            if count <= 0:
                raise OSError("manifest append made no progress")
            view = view[count:]
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
        raise RuntimeError(f"manifest verifier could not run: {result.returncode}")
    return json.loads(result.stdout)


def main() -> int:
    if PREFIX.exists():
        raise RuntimeError("refusing to overwrite the refinement prefix receipt")
    for path in (PACKAGE_INDEX, WORKTREE_INDEX, PROJECT_INDEX):
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode):
            raise RuntimeError(f"refusing non-regular manifest: {path}")

    indexes = (
        ("package", PACKAGE_INDEX),
        ("worktree", WORKTREE_INDEX),
        ("project", PROJECT_INDEX),
    )
    before: dict[str, object] = {}
    for name, index in indexes:
        raw = index.read_bytes()
        if raw and not raw.endswith(b"\n"):
            raise RuntimeError(f"manifest lacks final newline: {index}")
        before[name] = {
            "line_count": len(raw.splitlines()),
            "sha256": hashlib.sha256(raw).hexdigest(),
        }
    PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    package_files = []
    for path in sorted(RUN.iterdir()):
        if path.name == "SHA256SUMS":
            continue
        if not stat.S_ISREG(path.lstat().st_mode):
            raise RuntimeError(f"refusing non-regular package path: {path}")
        package_files.append(path)
    package_rows = [
        (hash_file(path), path.relative_to(RUN).as_posix()) for path in package_files
    ]
    append(PACKAGE_INDEX, package_rows)

    relative = RUN.relative_to(WORKTREE).as_posix()
    worktree_rows = [(digest, f"{relative}/{name}") for digest, name in package_rows]
    project_rows = [
        (digest, f".runtime/webhook-005-workspace/{relative}/{name}")
        for digest, name in package_rows
    ]
    package_digest = hash_file(PACKAGE_INDEX)
    worktree_rows.append((package_digest, f"{relative}/SHA256SUMS"))
    project_rows.append((package_digest, f".runtime/webhook-005-workspace/{relative}/SHA256SUMS"))

    for path in TOUCHED:
        digest = hash_file(path)
        name = path.relative_to(WORKTREE).as_posix()
        worktree_rows.append((digest, name))
        project_rows.append((digest, f".runtime/webhook-005-workspace/{name}"))

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
        count = int(before[name]["line_count"])
        prefix = b"".join(raw.splitlines(keepends=True)[:count])
        if hashlib.sha256(prefix).hexdigest() != before[name]["sha256"]:
            raise RuntimeError(f"append-only prefix changed: {name}")

    results = {
        "append_only_prefixes": "pass",
        "package": verify(PACKAGE_INDEX, RUN),
        "worktree": verify(WORKTREE_INDEX, WORKTREE),
        "project": verify(PROJECT_INDEX, PROJECT),
    }
    print(json.dumps(results, sort_keys=True))
    return 0 if all(value["passed"] for name, value in results.items() if name != "append_only_prefixes") else 1


if __name__ == "__main__":
    raise SystemExit(main())
