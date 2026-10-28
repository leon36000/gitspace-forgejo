#!/usr/bin/env python3
"""Verify preserved manifest prefixes and latest hashes for CONTINUATION-319."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
PROJECT = ROOT.parents[1]
PREFIX = json.loads((RUN / "manifest-prefix-before.json").read_text(encoding="utf-8"))
TARGETS = [
    (RUN / "SHA256SUMS", RUN, "evidence/continuation-319-forgejo-merge-mode-source-audit-001/SHA256SUMS"),
    (ROOT / "evidence/SHA256SUMS", ROOT, "evidence/SHA256SUMS"),
    (PROJECT / "evidence/SHA256SUMS", PROJECT, "../../evidence/SHA256SUMS"),
]

for manifest, base, key in TARGETS:
    record = PREFIX[key]
    lines = manifest.read_bytes().splitlines(keepends=True)
    prefix = b"".join(lines[: record["lines"]])
    assert len(prefix) == record["bytes"], f"prefix length changed: {manifest}"
    assert hashlib.sha256(prefix).hexdigest() == record["sha256"], f"prefix digest changed: {manifest}"
    latest_rows: dict[str, str] = {}
    for raw in lines[record["lines"] :]:
        line = raw.decode("utf-8").rstrip()
        if line:
            expected, relative = line.split("  ", 1)
            latest_rows[relative] = expected
    for relative, expected in latest_rows.items():
        actual = hashlib.sha256((base / relative).read_bytes()).hexdigest()
        assert actual == expected, f"digest mismatch: {manifest} -> {relative}"
    print(f"verified {manifest}: prefix intact; {len(latest_rows)} latest paths valid")
