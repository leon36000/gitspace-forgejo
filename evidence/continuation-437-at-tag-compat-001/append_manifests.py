#!/usr/bin/env python3
"""Append the CONTINUATION-437 evidence without changing prior index entries."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat


RUN = Path(__file__).resolve().parent
WORKTREE = RUN.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = RUN / "SHA256SUMS"
PREFIX = RUN / "manifest-prefix-before.json"
CHECK = RUN / "manifest-check.txt"
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXTRA_PATHS = (
    Path("docs/checkpoint.md"),
    Path("docs/refusal-evidence-matrix.md"),
    Path("src/forgejo.rs"),
    Path("src/webhook/reconcile.rs"),
    Path("tests/webhook_reconcile_contract.rs"),
)


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_file(path: Path) -> str:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0))
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


def append_entries(index: Path, entries: list[tuple[str, str]]) -> None:
    with index.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


def main() -> int:
    if any(path.exists() or path.is_symlink() for path in (LOCAL, PREFIX, CHECK)):
        raise SystemExit("refusing to overwrite a pre-existing CONTINUATION-437 manifest artifact")
    for index in (WORKTREE_INDEX, PROJECT_INDEX):
        if not index.is_file() or index.is_symlink():
            raise SystemExit(f"append-only index unavailable: {index}")
        if index.read_bytes() and not index.read_bytes().endswith(b"\n"):
            raise SystemExit(f"append-only index has no final newline: {index}")

    before = {}
    for name, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        data = index.read_bytes()
        before[name] = {"line_count": len(data.splitlines()), "sha256": digest_bytes(data)}
    PREFIX.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

    run_files = []
    for path in RUN.rglob("*"):
        relative = path.relative_to(RUN).as_posix()
        if path.is_symlink():
            raise SystemExit(f"symlink in evidence directory: {relative}")
        if path.is_file() and path.name not in EXCLUDED:
            run_files.append(path)
    run_files.sort(key=lambda path: path.relative_to(RUN).as_posix())

    local_entries = [
        (digest_file(path), path.relative_to(RUN).as_posix()) for path in run_files
    ]
    LOCAL.write_text(
        "".join(f"{value}  {name}\n" for value, name in local_entries), encoding="utf-8"
    )
    local_digest = digest_file(LOCAL)

    worktree_entries = [
        (value, f"evidence/{RUN.name}/{name}") for value, name in local_entries
    ]
    worktree_entries.append((local_digest, f"evidence/{RUN.name}/SHA256SUMS"))
    project_entries = [
        (value, f".runtime/webhook-005-workspace/evidence/{RUN.name}/{name}")
        for value, name in local_entries
    ]
    project_entries.append(
        (local_digest, f".runtime/webhook-005-workspace/evidence/{RUN.name}/SHA256SUMS")
    )

    for relative in EXTRA_PATHS:
        worktree_path = WORKTREE / relative
        value = digest_file(worktree_path)
        worktree_entries.append((value, relative.as_posix()))
        project_entries.append((value, f".runtime/webhook-005-workspace/{relative.as_posix()}"))

    append_entries(WORKTREE_INDEX, worktree_entries)
    append_entries(
        PROJECT_INDEX,
        project_entries
        + [
            (
                digest_file(WORKTREE_INDEX),
                ".runtime/webhook-005-workspace/evidence/SHA256SUMS",
            )
        ],
    )

    for name, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        data = index.read_bytes()
        old_count = before[name]["line_count"]
        prefix = b"".join(data.splitlines(keepends=True)[:old_count])
        if digest_bytes(prefix) != before[name]["sha256"]:
            raise SystemExit(f"append-only prefix changed for {name}")
    for value, name in local_entries:
        if digest_file(RUN / name) != value:
            raise SystemExit(f"local evidence hash mismatch: {name}")
    if digest_file(LOCAL) != local_digest:
        raise SystemExit("local evidence index hash mismatch")

    CHECK.write_text(
        "CONTINUATION-437 local evidence hashes: PASS\n"
        "worktree and project append-only prefixes: PASS\n"
        "checkpoint, refusal matrix, source and test hashes appended: PASS\n"
        f"local_evidence_files={len(local_entries)}\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "local_evidence_files": len(local_entries),
                "worktree_entries_appended": len(worktree_entries),
                "project_entries_appended": len(project_entries) + 1,
                "append_only_prefixes": "verified",
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
