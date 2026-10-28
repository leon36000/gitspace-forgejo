#!/usr/bin/env python3
from __future__ import annotations
import hashlib
import json
from pathlib import Path
RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
PROJECT = ROOT.parents[1]
PREFIX = json.loads((RUN / "manifest-prefix-before.json").read_text(encoding="utf-8"))
TARGETS = [
    (RUN / "SHA256SUMS", RUN, "evidence/continuation-323-forgejo-web-pre-receive-dismissal-race-001/SHA256SUMS"),
    (ROOT / "evidence/SHA256SUMS", ROOT, "evidence/SHA256SUMS"),
    (PROJECT / "evidence/SHA256SUMS", PROJECT, "../../evidence/SHA256SUMS"),
]
for manifest, base, key in TARGETS:
    record = PREFIX[key]
    lines = manifest.read_bytes().splitlines(keepends=True)
    actual_prefix = b"".join(lines[:record["lines"]])
    assert len(actual_prefix) == record["bytes"], f"prefix length changed: {manifest}"
    assert hashlib.sha256(actual_prefix).hexdigest() == record["sha256"], f"prefix hash changed: {manifest}"
    appended = {}
    for raw in lines[record["lines"]:]:
        line = raw.decode("utf-8").rstrip()
        if line:
            expected, relative = line.split("  ", 1)
            appended[relative] = expected
    for relative, expected in appended.items():
        actual = hashlib.sha256((base / relative).read_bytes()).hexdigest()
        assert actual == expected, f"digest mismatch: {manifest} -> {relative}"
    print(f"verified {manifest}: prefix intact, {len(appended)} appended paths")
for raw in (RUN / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
    expected, relative = raw.split("  ", 1)
    assert hashlib.sha256((RUN / relative).read_bytes()).hexdigest() == expected, relative
print(f"verified local bundle {RUN / 'SHA256SUMS'}")
