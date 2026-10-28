#!/usr/bin/env python3
"""Summarize current versus historical digests without rewriting either index."""

from __future__ import annotations

import hashlib
import re
import subprocess
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[4]
WORKTREE = PROJECT / ".runtime/webhook-005-workspace"


def inspect(label: str, root: Path, index: Path) -> str:
    rows = index.read_bytes().splitlines()
    stale = malformed = missing = 0
    for row in rows:
        try:
            encoded = row.decode("utf-8")
            expected, relative = encoded.split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            malformed += 1
            continue
        if not re.fullmatch(r"[0-9a-fA-F]{64}", expected):
            malformed += 1
            continue
        path = root / relative
        if not path.is_file():
            missing += 1
            continue
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            stale += 1
    result = subprocess.run(
        ["sha256sum", "-c", str(index.relative_to(root))],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
    )
    return (
        f"{label}: exit={result.returncode}; rows={len(rows)}; "
        f"stale={stale}; malformed={malformed}; missing={missing}"
    )


if __name__ == "__main__":
    print(inspect("worktree", WORKTREE, WORKTREE / "evidence/SHA256SUMS"))
    print(inspect("project", PROJECT, PROJECT / "evidence/SHA256SUMS"))
