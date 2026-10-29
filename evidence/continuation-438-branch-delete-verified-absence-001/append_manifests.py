#!/usr/bin/env python3
"""Append CONTINUATION-438 evidence to local and project SHA-256 indexes."""

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
LOCAL_INDEX = RUN / "SHA256SUMS"
PREFIX_SNAPSHOT = RUN / "manifest-prefix-before.json"
LOCAL_CHECK = RUN / "manifest-check.txt"
GLOBAL_CHECK = RUN / "global-manifest-check.txt"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
EXTRA_PATHS = (
    Path("docs/checkpoint.md"),
    Path("docs/implementation-plan.md"),
    Path("docs/refusal-evidence-matrix.md"),
    Path("scripts/run_forgejo_native_branch_delete_webhook_reconcile.py"),
    Path("scripts/run_webhook_reconcile_cli_e2e.py"),
    Path("src/bin/forgejo-reconcile-local.rs"),
    Path("src/webhook/reconcile.rs"),
    Path("tests/test_forgejo_emitted_webhook_e2e.py"),
    Path("tests/webhook_reconcile_contract.rs"),
    Path("tests/webhook_receiver_process_e2e.rs"),
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_file(path: Path) -> str:
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


def append(index: Path, entries: list[tuple[str, str]]) -> None:
    with index.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


def verify(index: Path, root: Path) -> str:
    completed = subprocess.run(
        [sys.executable, str(CHECKER), str(index), "--root", str(root)],
        cwd=WORKTREE,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
        timeout=60,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"manifest check failed: {completed.stdout}")
    return completed.stdout


def main() -> int:
    for path in (LOCAL_INDEX, PREFIX_SNAPSHOT, LOCAL_CHECK, GLOBAL_CHECK):
        if path.exists() or path.is_symlink():
            raise SystemExit(f"refusing to overwrite evidence artifact: {path.name}")
    for index in (WORKTREE_INDEX, PROJECT_INDEX):
        if not index.is_file() or index.is_symlink():
            raise SystemExit(f"append-only index unavailable: {index}")
        if index.read_bytes() and not index.read_bytes().endswith(b"\n"):
            raise SystemExit(f"append-only index has no trailing newline: {index}")

    before: dict[str, dict[str, object]] = {}
    for name, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        raw = index.read_bytes()
        before[name] = {"line_count": len(raw.splitlines()), "sha256": sha256(raw)}
    PREFIX_SNAPSHOT.write_text(
        json.dumps(before, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    run_files: list[Path] = []
    for path in RUN.rglob("*"):
        relative = path.relative_to(RUN).as_posix()
        if path.is_symlink():
            raise SystemExit(f"symlink in evidence package: {relative}")
        if path.is_file() and path.name not in EXCLUDED:
            run_files.append(path)
    run_files.sort(key=lambda path: path.relative_to(RUN).as_posix())
    local_entries = [
        (digest_file(path), path.relative_to(RUN).as_posix()) for path in run_files
    ]
    LOCAL_INDEX.write_text(
        "".join(f"{digest}  {name}\n" for digest, name in local_entries),
        encoding="utf-8",
    )
    local_index_digest = digest_file(LOCAL_INDEX)

    worktree_entries = [
        (digest, f"evidence/{RUN.name}/{name}") for digest, name in local_entries
    ]
    worktree_entries.append(
        (local_index_digest, f"evidence/{RUN.name}/SHA256SUMS")
    )
    project_entries = [
        (digest, f".runtime/webhook-005-workspace/evidence/{RUN.name}/{name}")
        for digest, name in local_entries
    ]
    project_entries.append(
        (
            local_index_digest,
            f".runtime/webhook-005-workspace/evidence/{RUN.name}/SHA256SUMS",
        )
    )

    for relative in EXTRA_PATHS:
        source = WORKTREE / relative
        value = digest_file(source)
        worktree_entries.append((value, relative.as_posix()))
        project_entries.append(
            (value, f".runtime/webhook-005-workspace/{relative.as_posix()}")
        )

    append(WORKTREE_INDEX, worktree_entries)
    append(
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
        raw = index.read_bytes()
        old_count = int(before[name]["line_count"])
        prefix = b"".join(raw.splitlines(keepends=True)[:old_count])
        if sha256(prefix) != before[name]["sha256"]:
            raise RuntimeError(f"append-only prefix changed: {name}")
    for digest, name in local_entries:
        if digest_file(RUN / name) != digest:
            raise RuntimeError(f"local evidence hash mismatch: {name}")
    if digest_file(LOCAL_INDEX) != local_index_digest:
        raise RuntimeError("local manifest hash mismatch")

    worktree_check = verify(WORKTREE_INDEX, WORKTREE)
    project_check = verify(PROJECT_INDEX, PROJECT)
    LOCAL_CHECK.write_text(
        "local evidence hashes: PASS\n"
        "worktree and project append-only prefixes: PASS\n"
        "canonical source and test hashes appended: PASS\n"
        f"local_evidence_files={len(local_entries)}\n",
        encoding="utf-8",
    )
    GLOBAL_CHECK.write_text(
        "worktree latest SHA-256 manifest: PASS\n"
        "project latest SHA-256 manifest: PASS\n"
        f"worktree_check={worktree_check.strip()}\n"
        f"project_check={project_check.strip()}\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "local_evidence_files": len(local_entries),
                "worktree_entries_appended": len(worktree_entries),
                "project_entries_appended": len(project_entries) + 1,
                "append_only_prefixes": "verified",
                "manifests": "pass",
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
