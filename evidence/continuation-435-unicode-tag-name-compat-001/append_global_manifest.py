#!/usr/bin/env python3
"""Append hashes for CONTINUATION-435 artifacts and changed source files."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path, PurePosixPath
import stat


ROOT = Path(__file__).resolve().parents[2]
RUN = Path("evidence/continuation-435-unicode-tag-name-compat-001")
MANIFEST = ROOT / "evidence/SHA256SUMS"
EXTRA_PATHS = (
    Path("docs/checkpoint.md"),
    Path("src/forgejo.rs"),
    Path("tests/webhook_reconcile_contract.rs"),
)


def digest_file(path: Path) -> str:
    value = hashlib.sha256()
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags)
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise OSError(f"not a regular file: {path}")
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            while chunk := source.read(1024 * 1024):
                value.update(chunk)
    finally:
        os.close(descriptor)
    return value.hexdigest()


def main() -> int:
    paths = sorted((*EXTRA_PATHS, *(p.relative_to(ROOT) for p in (ROOT / RUN).rglob("*") if p.is_file())))
    records = []
    for relative in paths:
        pure = PurePosixPath(relative.as_posix())
        if pure.is_absolute() or any(part in {"", ".", ".."} for part in pure.parts):
            raise ValueError(f"unsafe path: {relative}")
        path = ROOT / relative
        if path.is_symlink():
            raise ValueError(f"symlink refused: {relative}")
        records.append(f"{digest_file(path)}  {relative.as_posix()}\n")

    with MANIFEST.open("a", encoding="utf-8", newline="\n") as target:
        target.writelines(records)
    print(f"appended {len(records)} SHA-256 records to evidence/SHA256SUMS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
