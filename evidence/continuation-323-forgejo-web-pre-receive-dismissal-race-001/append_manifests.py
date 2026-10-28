#!/usr/bin/env python3
from __future__ import annotations
import hashlib
import json
from pathlib import Path
RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
PROJECT = ROOT.parents[1]
PREFIX = json.loads((RUN / "manifest-prefix-before.json").read_text(encoding="utf-8"))
KEYS = {
    RUN / "SHA256SUMS": "evidence/continuation-323-forgejo-web-pre-receive-dismissal-race-001/SHA256SUMS",
    ROOT / "evidence/SHA256SUMS": "evidence/SHA256SUMS",
    PROJECT / "evidence/SHA256SUMS": "../../evidence/SHA256SUMS",
}
def verify_prefix(path: Path) -> None:
    record = PREFIX[KEYS[path]]
    lines = path.read_bytes().splitlines(keepends=True) if path.exists() else []
    actual = b"".join(lines[:record["lines"]])
    if len(actual) != record["bytes"] or hashlib.sha256(actual).hexdigest() != record["sha256"]:
        raise SystemExit(f"manifest prefix changed; refusing append: {path}")
def append(manifest: Path, base: Path, files: list[Path]) -> None:
    verify_prefix(manifest)
    existing = set(manifest.read_text(encoding="utf-8").splitlines()) if manifest.exists() else set()
    rows = []
    for file in sorted(set(files)):
        if file.is_file():
            relative = file.relative_to(base).as_posix()
            row = f"{hashlib.sha256(file.read_bytes()).hexdigest()}  {relative}"
            if row not in existing:
                rows.append(row)
    if rows:
        with manifest.open("ab") as stream:
            stream.write((chr(10).join(rows) + chr(10)).encode("utf-8"))
    print(f"{manifest}: appended {len(rows)} rows")
bundle = [p for p in RUN.rglob("*") if p.is_file() and p.name != "SHA256SUMS"]
docs = [ROOT / "docs/checkpoint.md", ROOT / "docs/architecture.md", ROOT / "docs/source-inventory.md", ROOT / "docs/refusal-evidence-matrix.md"]
append(RUN / "SHA256SUMS", RUN, bundle)
root_files = bundle + [RUN / "SHA256SUMS"] + docs
append(ROOT / "evidence/SHA256SUMS", ROOT, root_files)
project_files = [p for p in root_files if p.is_relative_to(PROJECT)]
project_files.append(ROOT / "evidence/SHA256SUMS")
append(PROJECT / "evidence/SHA256SUMS", PROJECT, project_files)
