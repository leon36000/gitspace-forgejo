#!/usr/bin/env python3
"""Append current hashes for stale workspace paths in the project index."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess


RUN = Path(__file__).resolve().parent
WORKTREE = RUN.parents[1]
PROJECT = WORKTREE.parents[1]
INDEX = PROJECT / "evidence/SHA256SUMS"
REPORT = RUN / "parent-manifest-reconciliation.json"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"
HASH_LINE = re.compile(r"^([0-9a-f]{64})  (.+)$")
LEGACY_LINE = re.compile(r"^None  (.+)$")
EXPECTED_MISMATCHES = 52


def hash_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def hash_file(path: Path) -> str:
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


def verify_project_index() -> dict[str, object]:
    completed = subprocess.run(
        [
            "python3",
            str(CHECKER),
            "evidence/SHA256SUMS",
            "--root",
            ".",
        ],
        cwd=PROJECT,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode not in (0, 1):
        raise SystemExit(f"manifest verifier infrastructure exit {completed.returncode}")
    return json.loads(completed.stdout)


def main() -> int:
    if REPORT.exists() or REPORT.is_symlink():
        raise SystemExit("refusing to overwrite parent-manifest-reconciliation.json")
    if not INDEX.is_file() or INDEX.is_symlink():
        raise SystemExit("project append-only index is unavailable")

    before_bytes = INDEX.read_bytes()
    if before_bytes and not before_bytes.endswith(b"\n"):
        raise SystemExit("project index has no final newline")
    before_hash = hash_bytes(before_bytes)
    before_count = len(before_bytes.splitlines())

    latest: dict[str, str | None] = {}
    malformed = []
    for line_number, raw_line in enumerate(before_bytes.decode("utf-8").splitlines(), 1):
        match = HASH_LINE.fullmatch(raw_line)
        if match:
            digest, name = match.groups()
            latest[name] = digest
            continue
        legacy = LEGACY_LINE.fullmatch(raw_line)
        if legacy:
            latest[legacy.group(1)] = None
            continue
        if raw_line:
            malformed.append(line_number)
    if malformed:
        raise SystemExit(f"project index has malformed records at lines: {malformed[:10]}")

    mismatches = []
    missing = []
    unhashed = []
    for name, expected in latest.items():
        relative = PurePosixPath(name)
        if (
            relative.is_absolute()
            or not relative.parts
            or any(part in {"", ".", ".."} for part in relative.parts)
        ):
            raise SystemExit(f"unsafe project-index path: {name}")
        current = PROJECT
        for part in relative.parts:
            current /= part
            try:
                mode = current.lstat().st_mode
            except FileNotFoundError:
                missing.append(name)
                break
            if stat.S_ISLNK(mode):
                raise SystemExit(f"symlink in project-index path: {name}")
        else:
            if expected is None:
                unhashed.append(name)
                continue
            actual = hash_file(current)
            if actual != expected:
                mismatches.append({"path": name, "previous_sha256": expected, "current_sha256": actual})

    if missing or unhashed:
        raise SystemExit(
            json.dumps({"missing": missing[:10], "unhashed": unhashed[:10]}, sort_keys=True)
        )
    if len(mismatches) != EXPECTED_MISMATCHES:
        raise SystemExit(f"mismatch count changed: expected {EXPECTED_MISMATCHES}, got {len(mismatches)}")
    if any(not item["path"].startswith(".runtime/webhook-005-workspace/") for item in mismatches):
        raise SystemExit("refusing to update paths outside the active workspace")

    with INDEX.open("ab") as stream:
        for item in mismatches:
            stream.write(f"{item['current_sha256']}  {item['path']}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())

    after_bytes = INDEX.read_bytes()
    prefix = b"".join(after_bytes.splitlines(keepends=True)[:before_count])
    if hash_bytes(prefix) != before_hash:
        raise SystemExit("project manifest append-only prefix changed")

    verified = verify_project_index()
    if not verified.get("passed"):
        raise SystemExit("project index still fails after current-hash append")
    result = {
        "before": {
            "line_count": before_count,
            "sha256": before_hash,
            "mismatched_count": len(mismatches),
        },
        "appended_current_hashes": mismatches,
        "append_only_prefix_preserved": True,
        "after": {
            "line_count": len(after_bytes.splitlines()),
            "sha256": hash_bytes(after_bytes),
            "passed": verified["passed"],
            "unique_paths": verified["unique_paths"],
            "verified_paths": verified["verified_paths"],
            "mismatched_path_count": verified["mismatched_path_count"],
        },
    }
    REPORT.write_text(json.dumps(result, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "updated_paths": len(mismatches),
                "prefix_preserved": True,
                "project_manifest_passed": verified["passed"],
                "unique_paths": verified["unique_paths"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
