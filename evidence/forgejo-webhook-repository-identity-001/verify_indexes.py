#!/usr/bin/env python3
"""Verify latest append-only SHA256 index rows and this dossier manifest."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PROJECT = ROOT.parent.parent
DOSSIER = Path(__file__).resolve().parent

def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def latest_rows(index: Path) -> tuple[dict[str, str], int]:
    rows: dict[str, str] = {}
    malformed = 0
    for line in index.read_text(encoding="utf-8", errors="replace").splitlines():
        parts = line.split(None, 1)
        if (len(parts) == 2 and len(parts[0]) == 64
                and all(c in "0123456789abcdefABCDEF" for c in parts[0])):
            rows[parts[1]] = parts[0].lower()
        else:
            malformed += 1
    return rows, malformed

def check_index(index: Path, base: Path) -> dict[str, object]:
    rows, malformed = latest_rows(index)
    missing: list[str] = []
    mismatched: list[str] = []
    for rel, expected in rows.items():
        path = base / rel
        if not path.is_file():
            missing.append(rel)
        elif digest(path) != expected:
            mismatched.append(rel)
    return {
        "unique_paths": len(rows),
        "malformed_legacy_lines_preserved": malformed,
        "missing_paths": missing,
        "latest_hash_mismatches": mismatched,
    }

def check_dossier_manifest() -> dict[str, object]:
    manifest = DOSSIER / "SHA256SUMS"
    rows, malformed = latest_rows(manifest)
    missing: list[str] = []
    mismatched: list[str] = []
    for rel, expected in rows.items():
        path = DOSSIER / rel
        if not path.is_file():
            missing.append(rel)
        elif digest(path) != expected:
            mismatched.append(rel)
    return {
        "hashed_files": len(rows),
        "malformed_lines": malformed,
        "missing_files": missing,
        "hash_mismatches": mismatched,
    }

def main() -> int:
    result = {
        "dossier": check_dossier_manifest(),
        "worktree_index": check_index(ROOT / "evidence/SHA256SUMS", ROOT),
        "project_parent_index": check_index(PROJECT / "evidence/SHA256SUMS", PROJECT),
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if all(not item.get(key) for item in result.values() for key in
                    ("missing_paths", "latest_hash_mismatches", "missing_files", "hash_mismatches")) else 1

if __name__ == "__main__":
    raise SystemExit(main())
