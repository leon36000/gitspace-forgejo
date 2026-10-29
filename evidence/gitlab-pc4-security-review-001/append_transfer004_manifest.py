#!/usr/bin/env python3
"""Append this turn's supplemental readback to the old transfer index."""

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
INDEX = WORKTREE / "evidence/gitlab-pc4-transfer-004/SHA256SUMS"
EXTRA = WORKTREE / "evidence/gitlab-pc4-transfer-004/remote-readback.json"
PREFIX = RUN / "transfer004-prefix-before.json"
CHECKER = WORKTREE / "scripts/verify_latest_sha256_manifest.py"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def verify() -> dict[str, object]:
    result = subprocess.run(
        [sys.executable, str(CHECKER), str(INDEX), "--root", str(WORKTREE)],
        cwd=WORKTREE,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=60,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stdout)
    return json.loads(result.stdout)


def main() -> int:
    if PREFIX.exists() or PREFIX.is_symlink():
        raise SystemExit(f"refusing to overwrite {PREFIX}")
    if EXTRA.is_symlink() or not stat.S_ISREG(EXTRA.lstat().st_mode):
        raise SystemExit("supplemental readback is not a regular file")
    before = verify()
    raw = INDEX.read_bytes()
    if not raw.endswith(b"\n"):
        raise SystemExit("historical index has no final newline")
    relative = "evidence/gitlab-pc4-transfer-004/remote-readback.json"
    if any(line.endswith("  " + relative) for line in raw.decode("utf-8").splitlines()):
        raise SystemExit("supplemental readback already indexed")
    PREFIX.write_text(
        json.dumps(
            {"line_count": len(raw.splitlines()), "sha256": sha(raw)},
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    with INDEX.open("ab") as output:
        output.write(f"{sha(EXTRA.read_bytes())}  {relative}\n".encode("utf-8"))
        output.flush()
        os.fsync(output.fileno())
    after = INDEX.read_bytes()
    count = int(json.loads(PREFIX.read_text(encoding="utf-8"))["line_count"])
    if sha(b"".join(after.splitlines(keepends=True)[:count])) != sha(raw):
        raise RuntimeError("historical transfer index prefix changed")
    final = verify()
    print(
        json.dumps(
            {
                "prefix_unchanged": True,
                "before_unique_paths": before["unique_paths"],
                "after": final,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
