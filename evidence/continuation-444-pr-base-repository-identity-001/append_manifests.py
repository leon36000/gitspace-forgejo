#!/usr/bin/env python3
"""Create this run's receipt manifest and append its hashes to both indexes."""

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
PREFIX = RUN / "manifest-prefix-before.json"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"


def hash_file(path: Path) -> str:
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


def append_rows(index: Path, rows: list[tuple[str, str]]) -> None:
    fd = os.open(
        index,
        os.O_WRONLY | os.O_APPEND | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
    )
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise OSError(f"not a regular manifest: {index}")
        for digest, name in rows:
            os.write(fd, f"{digest}  {name}\n".encode("utf-8"))
        os.fsync(fd)
    finally:
        os.close(fd)


def package_files() -> list[Path]:
    result = []
    for path in sorted(RUN.iterdir()):
        if path.name in {"SHA256SUMS", "__pycache__"}:
            continue
        if not stat.S_ISREG(path.lstat().st_mode):
            raise SystemExit(f"refusing non-regular evidence path: {path}")
        result.append(path)
    return result


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
    indexes = (WORKTREE_INDEX, PROJECT_INDEX)
    local_manifest = RUN / "SHA256SUMS"
    if PREFIX.is_symlink():
        raise SystemExit(f"refusing symlinked prefix receipt: {PREFIX}")
    if PREFIX.exists():
        before = json.loads(PREFIX.read_text(encoding="utf-8"))
        if set(before) != {"worktree", "project"}:
            raise SystemExit("prefix receipt has an unexpected shape")
    else:
        if local_manifest.exists() or local_manifest.is_symlink():
            raise SystemExit("local package manifest exists without its prefix receipt")
        before = {}
        for label, index in zip(("worktree", "project"), indexes, strict=True):
            if index.is_symlink() or not stat.S_ISREG(index.lstat().st_mode):
                raise SystemExit(f"refusing unsafe manifest: {index}")
            raw = index.read_bytes()
            if not raw.endswith(b"\n"):
                raise SystemExit(f"manifest lacks final newline: {index}")
            before[label] = {
                "line_count": len(raw.splitlines()),
                "sha256": hashlib.sha256(raw).hexdigest(),
            }
        PREFIX.write_text(
            json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    def verify_prefix_and_suffix(index: Path, label: str) -> bytes:
        if index.is_symlink() or not stat.S_ISREG(index.lstat().st_mode):
            raise SystemExit(f"refusing unsafe manifest: {index}")
        raw = index.read_bytes()
        lines = raw.splitlines(keepends=True)
        old_count = before[label]["line_count"]
        if len(lines) < old_count:
            raise SystemExit(f"manifest lost pre-existing records: {label}")
        prefix = b"".join(lines[:old_count])
        if hashlib.sha256(prefix).hexdigest() != before[label]["sha256"]:
            raise SystemExit(f"append-only prefix changed: {label}")
        return b"".join(lines[old_count:])

    rows = [
        (hash_file(path), path.relative_to(RUN).as_posix())
        for path in package_files()
    ]
    expected_local = "".join(f"{digest}  {name}\n" for digest, name in rows).encode("utf-8")
    if local_manifest.is_symlink():
        raise SystemExit("refusing symlinked local package manifest")
    if local_manifest.exists():
        if not stat.S_ISREG(local_manifest.lstat().st_mode):
            raise SystemExit("refusing non-regular local package manifest")
        existing_local = local_manifest.read_bytes()
        if existing_local == b"":
            with local_manifest.open("ab") as output:
                output.write(expected_local)
                output.flush()
                os.fsync(output.fileno())
        elif existing_local != expected_local:
            raise SystemExit("local package manifest differs from the expected receipt rows")
    else:
        with local_manifest.open("xb") as output:
            output.write(expected_local)
            output.flush()
            os.fsync(output.fileno())

    package_name = RUN.relative_to(WORKTREE).as_posix()
    worktree_rows = [
        (digest, f"{package_name}/{name}")
        for digest, name in rows
    ]
    local_manifest_digest = hash_file(local_manifest)
    worktree_rows.append((local_manifest_digest, f"{package_name}/SHA256SUMS"))

    touched = (
        WORKTREE / "src/forgejo.rs",
        WORKTREE / "tests/forgejo_merge_preflight.rs",
        WORKTREE / "src/bin/forgejo-mission-local.rs",
        WORKTREE / "src/bin/forgejo-reconcile-local.rs",
        WORKTREE / "tests/forgejo_mission_delivery_process_e2e.rs",
        WORKTREE / "scripts/run_forgejo_mission_delivery_process_e2e.py",
        WORKTREE / "tests/forgejo_preflight_process_e2e.rs",
        WORKTREE / "tests/forgejo_merge_post_crash_recovery_process_e2e.rs",
        WORKTREE / "tests/forgejo_reconcile_capabilities_e2e.rs",
        WORKTREE / "tests/test_mission_requirement_arbitration_cli_process.py",
        WORKTREE / "docs/architecture.md",
        WORKTREE / "docs/checkpoint.md",
        WORKTREE / "docs/refusal-evidence-matrix.md",
    )
    project_rows = []
    for path in touched:
        digest = hash_file(path)
        relative = path.relative_to(WORKTREE).as_posix()
        worktree_rows.append((digest, relative))
        project_rows.append((digest, f".runtime/webhook-005-workspace/{relative}"))
    for digest, name in rows:
        project_rows.append((digest, f".runtime/webhook-005-workspace/{package_name}/{name}"))
    project_rows.append((local_manifest_digest, f".runtime/webhook-005-workspace/{package_name}/SHA256SUMS"))

    expected_worktree = "".join(
        f"{digest}  {name}\n" for digest, name in worktree_rows
    ).encode("utf-8")
    worktree_suffix = verify_prefix_and_suffix(WORKTREE_INDEX, "worktree")
    if worktree_suffix == b"":
        append_rows(WORKTREE_INDEX, worktree_rows)
    elif worktree_suffix != expected_worktree:
        raise SystemExit("worktree manifest suffix is neither empty nor this run's exact append")
    project_rows.append((hash_file(WORKTREE_INDEX), ".runtime/webhook-005-workspace/evidence/SHA256SUMS"))
    expected_project = "".join(
        f"{digest}  {name}\n" for digest, name in project_rows
    ).encode("utf-8")
    project_suffix = verify_prefix_and_suffix(PROJECT_INDEX, "project")
    if project_suffix == b"":
        append_rows(PROJECT_INDEX, project_rows)
    elif project_suffix != expected_project:
        raise SystemExit("project manifest suffix is neither empty nor this run's exact append")

    verify_prefix_and_suffix(WORKTREE_INDEX, "worktree")
    verify_prefix_and_suffix(PROJECT_INDEX, "project")

    results = {
        "append_only_prefixes": "pass",
        "worktree_manifest": verify(WORKTREE_INDEX, WORKTREE),
        "project_manifest": verify(PROJECT_INDEX, PROJECT),
        "package_manifest_entries": len(rows),
    }
    print(json.dumps(results, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
