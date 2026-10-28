#!/usr/bin/env python3
"""Verify the last valid SHA-256 record per path in an append-only manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
from typing import Any


HASH_LINE = re.compile(r"^([0-9a-f]{64})  (.+)$")
LEGACY_UNHASHED_LINE = re.compile(r"^None  (.+)$")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise OSError("manifest target is not a regular file")
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
    finally:
        os.close(descriptor)
    return digest.hexdigest()


def verify_latest_manifest(manifest: Path, root: Path) -> dict[str, Any]:
    """Check each path against its last valid digest; never trust path traversal."""
    manifest = manifest.resolve(strict=True)
    root = root.resolve(strict=True)
    latest: dict[str, tuple[str | None, int]] = {}
    valid_records = 0
    legacy_unhashed_records = 0
    blank_lines = 0
    malformed: list[dict[str, Any]] = []

    for line_number, raw_line in enumerate(manifest.read_text(encoding="utf-8").splitlines(), 1):
        line = raw_line.removesuffix("\r")
        if not line:
            blank_lines += 1
            continue
        match = HASH_LINE.fullmatch(line)
        if match:
            digest, name = match.groups()
            latest[name] = (digest, line_number)
            valid_records += 1
            continue
        legacy = LEGACY_UNHASHED_LINE.fullmatch(line)
        if legacy:
            latest[legacy.group(1)] = (None, line_number)
            legacy_unhashed_records += 1
            continue
        if len(malformed) < 20:
            malformed.append({"line": line_number, "sample": line[:160]})

    missing: list[dict[str, Any]] = []
    mismatched: list[dict[str, Any]] = []
    unhashed: list[dict[str, Any]] = []
    unsafe: list[dict[str, Any]] = []
    verified_paths = 0

    for name, (expected, line_number) in latest.items():
        relative = PurePosixPath(name)
        if relative.is_absolute() or not relative.parts or any(part in {"", ".", ".."} for part in relative.parts):
            unsafe.append({"path": name, "line": line_number, "reason": "not a normalized relative path"})
            continue
        current = root
        invalid_path = False
        missing_path = False
        for index, part in enumerate(relative.parts):
            current = current / part
            try:
                mode = current.lstat().st_mode
            except FileNotFoundError:
                missing_path = True
                break
            except OSError as error:
                unsafe.append({"path": name, "line": line_number, "reason": type(error).__name__})
                invalid_path = True
                break
            if stat.S_ISLNK(mode):
                unsafe.append({"path": name, "line": line_number, "reason": "symlink component"})
                invalid_path = True
                break
            final = index == len(relative.parts) - 1
            if (final and not stat.S_ISREG(mode)) or (not final and not stat.S_ISDIR(mode)):
                unsafe.append({"path": name, "line": line_number, "reason": "not a regular file path"})
                invalid_path = True
                break
        if invalid_path:
            continue
        if missing_path:
            missing.append({"path": name, "line": line_number})
            continue
        if expected is None:
            unhashed.append({"path": name, "line": line_number})
            continue
        actual = sha256_file(current)
        if actual != expected:
            mismatched.append({"path": name, "line": line_number, "expected": expected, "actual": actual})
        else:
            verified_paths += 1

    passed = not (malformed or missing or mismatched or unhashed or unsafe)
    return {
        "manifest": str(manifest),
        "root": str(root),
        "valid_records": valid_records,
        "unique_paths": len(latest),
        "verified_paths": verified_paths,
        "blank_lines_ignored": blank_lines,
        "legacy_unhashed_records": legacy_unhashed_records,
        "malformed_records": malformed,
        "missing_paths": missing[:20],
        "missing_path_count": len(missing),
        "mismatched_paths": mismatched[:20],
        "mismatched_path_count": len(mismatched),
        "unhashed_paths": unhashed[:20],
        "unhashed_path_count": len(unhashed),
        "unsafe_paths": unsafe[:20],
        "unsafe_path_count": len(unsafe),
        "passed": passed,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    result = verify_latest_manifest(args.manifest, args.root)
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
