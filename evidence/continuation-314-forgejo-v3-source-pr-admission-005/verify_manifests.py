#!/usr/bin/env python3
"""Verify preserved manifest prefixes and all rows appended for CONTINUATION-314."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
PROJECT = ROOT.parents[1]
PREFIX = json.loads((RUN / "manifest-prefix-before.json").read_text(encoding="utf-8"))
TARGETS = [
    (RUN / "SHA256SUMS", RUN, "evidence/continuation-314-forgejo-v3-source-pr-admission-005/SHA256SUMS"),
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
        if not line:
            continue
        expected, rel = line.split("  ", 1)
        latest_rows[rel] = expected
    for rel, expected in latest_rows.items():
        file = base / rel
        actual = hashlib.sha256(file.read_bytes()).hexdigest()
        assert actual == expected, f"digest mismatch: {manifest} -> {rel}"
    print(f"verified {manifest}: original prefix intact; {len(latest_rows)} appended paths have valid latest hashes")
