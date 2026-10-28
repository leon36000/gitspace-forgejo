#!/usr/bin/env python3
"""Independent black-box acceptance checks, mounted read-only by the verifier."""
import subprocess
import sys


binary = sys.argv[1]
cases = {
    "250ms": "250",
    "2s": "2000",
    "3m": "180000",
    "0042": "42",
    "7MS": "7",
}
for source, expected in cases.items():
    result = subprocess.run([binary, "parse-duration", source], text=True, capture_output=True)
    actual = result.stdout.strip()
    if result.returncode != 0 or actual != expected:
        print(f"FAIL {source!r}: expected {expected} ms, exit={result.returncode}, stdout={actual!r}, stderr={result.stderr.strip()!r}")
        raise SystemExit(1)

for source in ("", "-1s", "1.5s", "999999999999999999999999m", "3xs"):
    result = subprocess.run([binary, "parse-duration", source], text=True, capture_output=True)
    if result.returncode == 0:
        print(f"FAIL {source!r}: invalid duration was accepted as {result.stdout.strip()!r}")
        raise SystemExit(1)

print(f"PASS {len(cases)} valid and 5 invalid duration cases")
