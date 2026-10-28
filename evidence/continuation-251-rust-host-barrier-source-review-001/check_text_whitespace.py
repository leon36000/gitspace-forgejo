#!/usr/bin/env python3
"""Check trailing whitespace in CONTINUATION-251 text evidence and current docs."""
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
PATHS = [ROOT / "docs/checkpoint.md", ROOT / "docs/refusal-evidence-matrix.md"]
PATHS.extend(sorted(EVIDENCE.glob("*")))
failures: list[str] = []
inspected = 0
for path in PATHS:
    if path.is_symlink() or not path.is_file() or path.suffix not in {".py", ".md", ".txt", ".log"}:
        continue
    inspected += 1
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if line.endswith((" ", "\t")):
            failures.append(f"{path.relative_to(ROOT)}:{line_number}: trailing whitespace")
if failures:
    print("\n".join(failures))
    raise SystemExit(1)
print(f"no trailing whitespace in {inspected} inspected text files")
