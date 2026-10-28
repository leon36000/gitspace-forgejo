#!/usr/bin/env python3
from __future__ import annotations
import hashlib
import json
from pathlib import Path

RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
PROJECT = ROOT.parents[1]
RUN321 = ROOT / "evidence/continuation-321-forgejo-pre-receive-admin-bypass-boundary-001"
PREFIX = json.loads((RUN / "manifest-prefix-before.json").read_text(encoding="utf-8"))
PREFIX_KEYS = {
    RUN / "SHA256SUMS": "evidence/continuation-322-forgejo-pre-receive-dismissal-race-001/SHA256SUMS",
    ROOT / "evidence/SHA256SUMS": "evidence/SHA256SUMS",
    PROJECT / "evidence/SHA256SUMS": "../../evidence/SHA256SUMS",
}

def check_prefix(path: Path) -> None:
    record = PREFIX[PREFIX_KEYS[path]]
    lines = path.read_bytes().splitlines(keepends=True) if path.exists() else []
    prefix = b"".join(lines[:record["lines"]])
    if len(prefix) != record["bytes"] or hashlib.sha256(prefix).hexdigest() != record["sha256"]:
        raise SystemExit(f"manifest prefix changed; refusing append: {path}")

def append_entries(manifest: Path, base: Path, files: list[Path]) -> int:
    check_prefix(manifest)
    existing = set(manifest.read_text(encoding="utf-8").splitlines()) if manifest.exists() else set()
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

bundle322 = [p for p in RUN.rglob("*") if p.is_file() and p.name != "SHA256SUMS"]
bundle321 = [p for p in RUN321.rglob("*") if p.is_file() and p.name != "SHA256SUMS"]
local321_manifest = RUN321 / "SHA256SUMS"
docs = [ROOT / "docs/checkpoint.md", ROOT / "docs/architecture.md", ROOT / "docs/source-inventory.md", ROOT / "docs/refusal-evidence-matrix.md"]
append_entries(RUN / "SHA256SUMS", RUN, bundle322)
root_files = bundle321 + [local321_manifest] + bundle322 + [RUN / "SHA256SUMS"] + docs
append_entries(ROOT / "evidence/SHA256SUMS", ROOT, root_files)
project_files = [p for p in root_files if p.is_relative_to(PROJECT)]
project_files.append(ROOT / "evidence/SHA256SUMS")
append_entries(PROJECT / "evidence/SHA256SUMS", PROJECT, project_files)
