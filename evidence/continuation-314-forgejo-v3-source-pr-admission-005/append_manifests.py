#!/usr/bin/env python3
"""Append-only SHA-256 registration for the CONTINUATION-314 evidence bundle."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
PROJECT = ROOT.parents[1]
PREFIX = json.loads((RUN / "manifest-prefix-before.json").read_text(encoding="utf-8"))
PREFIX_KEYS = {
    RUN / "SHA256SUMS": "evidence/continuation-314-forgejo-v3-source-pr-admission-005/SHA256SUMS",
    ROOT / "evidence/SHA256SUMS": "evidence/SHA256SUMS",
    PROJECT / "evidence/SHA256SUMS": "../../evidence/SHA256SUMS",
}


def check_prefix(path: Path) -> None:
    record = PREFIX[PREFIX_KEYS[path]]
    lines = path.read_bytes().splitlines(keepends=True)
    prefix = b"".join(lines[: record["lines"]])
    if len(prefix) != record["bytes"] or hashlib.sha256(prefix).hexdigest() != record["sha256"]:
        raise SystemExit(f"manifest prefix changed; refusing append: {path}")


def append_entries(manifest: Path, base: Path, files: list[Path]) -> int:
    check_prefix(manifest)
    existing = set(manifest.read_text(encoding="utf-8").splitlines())
    rows: list[str] = []
    for file in sorted(set(files)):
        if not file.is_file():
            continue
        rel = file.relative_to(base).as_posix()
        row = f"{hashlib.sha256(file.read_bytes()).hexdigest()}  {rel}"
        if row not in existing:
            rows.append(row)
    if rows:
        with manifest.open("ab") as stream:
            stream.write((chr(10).join(rows) + chr(10)).encode("utf-8"))
    print(f"{manifest}: appended {len(rows)} rows")
    return len(rows)


bundle_files = [p for p in RUN.rglob("*") if p.is_file() and p.name != "SHA256SUMS"]
source_files = [
    ROOT / "scripts/run_forgejo_live_webhook_mission_admission.py",
    ROOT / "tests/test_forgejo_live_webhook_mission_admission.py",
    ROOT / "src/mission_admission.rs",
    ROOT / "tests/mission_requirement_arbitration_contract.rs",
    ROOT / "docs/checkpoint.md",
    ROOT / "docs/architecture.md",
    ROOT / "docs/source-inventory.md",
    ROOT / "docs/refusal-evidence-matrix.md",
]

# Update the bundle-local manifest first, then register its digest globally.
append_entries(RUN / "SHA256SUMS", RUN, bundle_files)
all_product_files = bundle_files + [RUN / "SHA256SUMS"] + source_files
append_entries(ROOT / "evidence/SHA256SUMS", ROOT, all_product_files)
project_files = [f for f in all_product_files if f.is_relative_to(PROJECT)]
project_files.append(ROOT / "evidence/SHA256SUMS")
append_entries(PROJECT / "evidence/SHA256SUMS", PROJECT, project_files)
