#!/usr/bin/env python3
"""Append the parent-manifest repair receipt to all local SHA-256 indexes."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess


RUN = Path(__file__).resolve().parent
WORKTREE = RUN.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL_INDEX = RUN / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX_SNAPSHOT = RUN / "final-manifest-prefix-before.json"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"
NEW_FILES = (
    "repair_parent_manifest.py",
    "parent-manifest-reconciliation.json",
    "manifest-reconciliation.md",
    "finalize_manifests.py",
    "final-manifest-prefix-before.json",
)


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def digest_file(path: Path) -> str:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0))
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


def append(index: Path, entries: list[tuple[str, str]]) -> None:
    with index.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


def verify(index: Path, root: Path, cwd: Path) -> dict[str, object]:
    completed = subprocess.run(
        ["python3", str(CHECKER), str(index), "--root", str(root)],
        cwd=cwd,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        raise SystemExit(
            f"manifest verification failed for {index}: {completed.stdout}{completed.stderr}"
        )
    return json.loads(completed.stdout)


def main() -> int:
    indexes = (LOCAL_INDEX, WORKTREE_INDEX, PROJECT_INDEX)
    before = {}
    for index in indexes:
        if not index.is_file() or index.is_symlink():
            raise SystemExit(f"manifest unavailable: {index}")
        data = index.read_bytes()
        if data and not data.endswith(b"\n"):
            raise SystemExit(f"manifest lacks a final newline: {index}")
        before[str(index)] = {
            "line_count": len(data.splitlines()),
            "sha256": digest_bytes(data),
        }
    if PREFIX_SNAPSHOT.exists() or PREFIX_SNAPSHOT.is_symlink():
        raise SystemExit("refusing to overwrite final-manifest-prefix-before.json")
    PREFIX_SNAPSHOT.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

    current_local_lines = LOCAL_INDEX.read_text(encoding="utf-8").splitlines()
    existing_local_names = {
        line.split("  ", 1)[1]
        for line in current_local_lines
        if len(line) >= 67 and line[64:66] == "  "
    }
    if any(name in existing_local_names for name in NEW_FILES):
        raise SystemExit("a final reconciliation artifact is already indexed")
    local_entries = []
    for name in NEW_FILES:
        path = RUN / name
        if path.is_symlink() or not path.is_file():
            raise SystemExit(f"required final artifact is absent or unsafe: {name}")
        local_entries.append((digest_file(path), name))
    append(LOCAL_INDEX, local_entries)
    local_sha = digest_file(LOCAL_INDEX)

    worktree_entries = [
        (value, f"evidence/{RUN.name}/{name}") for value, name in local_entries
    ]
    worktree_entries.append((local_sha, f"evidence/{RUN.name}/SHA256SUMS"))
    append(WORKTREE_INDEX, worktree_entries)

    project_entries = [
        (value, f".runtime/webhook-005-workspace/evidence/{RUN.name}/{name}")
        for value, name in local_entries
    ]
    project_entries.append((local_sha, f".runtime/webhook-005-workspace/evidence/{RUN.name}/SHA256SUMS"))
    project_entries.append(
        (
            digest_file(WORKTREE_INDEX),
            ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
        )
    )
    append(PROJECT_INDEX, project_entries)

    for index in indexes:
        data = index.read_bytes()
        count = before[str(index)]["line_count"]
        prefix = b"".join(data.splitlines(keepends=True)[:count])
        if digest_bytes(prefix) != before[str(index)]["sha256"]:
            raise SystemExit(f"append-only prefix changed: {index}")
    for value, name in local_entries:
        if digest_file(RUN / name) != value:
            raise SystemExit(f"local hash mismatch: {name}")

    local_check = subprocess.run(
        ["sha256sum", "-c", "SHA256SUMS"],
        cwd=RUN,
        text=True,
        capture_output=True,
        check=False,
    )
    if local_check.returncode:
        raise SystemExit(f"local evidence manifest failed: {local_check.stdout}{local_check.stderr}")
    worktree_check = verify(WORKTREE_INDEX, WORKTREE, WORKTREE)
    project_check = verify(PROJECT_INDEX, PROJECT, PROJECT)
    print(
        json.dumps(
            {
                "appended_local_entries": len(local_entries),
                "local_sha256sum_entries_verified": len(local_check.stdout.splitlines()),
                "append_only_prefixes_preserved": True,
                "worktree_manifest": {
                    "passed": worktree_check["passed"],
                    "unique_paths": worktree_check["unique_paths"],
                    "verified_paths": worktree_check["verified_paths"],
                },
                "project_manifest": {
                    "passed": project_check["passed"],
                    "unique_paths": project_check["unique_paths"],
                    "verified_paths": project_check["verified_paths"],
                },
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
