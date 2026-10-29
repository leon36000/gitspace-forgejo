#!/usr/bin/env python3
"""Append this lot to the local, worktree, and project SHA-256 indexes."""

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
PREFIX_SNAPSHOT = RUN / "manifest-prefix-before.json"
CHECK = RUN / "manifest-check.txt"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXCLUDED = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
EXTRA_PATHS = (
    Path("docs/checkpoint.md"),
    Path("docs/refusal-evidence-matrix.md"),
    Path("src/webhook.rs"),
    Path("src/webhook/reconcile.rs"),
    Path("tests/webhook_handler_contract.rs"),
)
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def digest_file(path: Path) -> str:
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
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
        raise SystemExit(f"manifest check failed for {index}: {completed.stdout}{completed.stderr}")
    return json.loads(completed.stdout)


def main() -> int:
    if any(path.exists() or path.is_symlink() for path in (LOCAL_INDEX, PREFIX_SNAPSHOT, CHECK)):
        raise SystemExit("refusing to overwrite a pre-existing local manifest artifact")
    for index in (WORKTREE_INDEX, PROJECT_INDEX):
        if not index.is_file() or index.is_symlink():
            raise SystemExit(f"append-only index unavailable: {index}")
        data = index.read_bytes()
        if data and not data.endswith(b"\n"):
            raise SystemExit(f"append-only index lacks a final newline: {index}")

    before = {}
    for name, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        data = index.read_bytes()
        before[name] = {"line_count": len(data.splitlines()), "sha256": digest_bytes(data)}
    PREFIX_SNAPSHOT.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

    run_files: list[Path] = []
    for path in RUN.rglob("*"):
        relative = path.relative_to(RUN).as_posix()
        if path.is_symlink():
            raise SystemExit(f"symlink in evidence directory: {relative}")
        if path.is_file() and path.name not in EXCLUDED:
            run_files.append(path)
    run_files.sort(key=lambda path: path.relative_to(RUN).as_posix())
    if not run_files:
        raise SystemExit("evidence directory is unexpectedly empty")

    local_entries = [(digest_file(path), path.relative_to(RUN).as_posix()) for path in run_files]
    LOCAL_INDEX.write_text(
        "".join(f"{value}  {name}\n" for value, name in local_entries), encoding="utf-8"
    )
    local_digest = digest_file(LOCAL_INDEX)
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
        value = digest_file(WORKTREE / relative)
        worktree_entries.append((value, relative.as_posix()))
        project_entries.append((value, f".runtime/webhook-005-workspace/{relative.as_posix()}"))

    append(WORKTREE_INDEX, worktree_entries)
    project_entries.append(
        (digest_file(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS")
    )
    append(PROJECT_INDEX, project_entries)

    for name, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        data = index.read_bytes()
        old_count = before[name]["line_count"]
        prefix = b"".join(data.splitlines(keepends=True)[:old_count])
        if digest_bytes(prefix) != before[name]["sha256"]:
            raise SystemExit(f"append-only prefix changed for {name}")
    for value, name in local_entries:
        if digest_file(RUN / name) != value:
            raise SystemExit(f"local evidence hash mismatch: {name}")

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
    CHECK.write_text(
        "Local SHA256SUMS: PASS\n"
        "Worktree/project latest-record manifests: PASS\n"
        "Append-only prefixes preserved: PASS\n"
        f"Local entries: {len(local_entries)}\n"
        f"Worktree verified paths: {worktree_check['verified_paths']}\n"
        f"Project verified paths: {project_check['verified_paths']}\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "local_entries": len(local_entries),
                "local_sha256sum_entries_verified": len(local_check.stdout.splitlines()),
                "append_only_prefixes_preserved": True,
                "worktree_verified_paths": worktree_check["verified_paths"],
                "project_verified_paths": project_check["verified_paths"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
