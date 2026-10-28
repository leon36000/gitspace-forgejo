#!/usr/bin/env python3
"""Verify this lot's evidence, relevant composed-run receipts, sources, and old index prefixes."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))

RUN_DIRS = [
    WORKTREE / "evidence" / f"mission-012-live-agent-webhook-{number}-supervised"
    for number in (265, 266, 267, 268, 269)
] + [
    WORKTREE / "evidence" / f"mission-012-live-agent-webhook-{number}-supervised-admission"
    for number in (265, 266, 267, 268, 269)
]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse(path: Path) -> tuple[dict[str, str], int, int]:
    latest: dict[str, str] = {}
    blanks = malformed = 0
    for raw in path.read_bytes().splitlines():
        if not raw.strip():
            blanks += 1
            continue
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            malformed += 1
            continue
        if len(value) != 64 or any(ch not in "0123456789abcdef" for ch in value) or not name:
            malformed += 1
            continue
        latest[name] = value
    return latest, blanks, malformed


def verify_local(directory: Path) -> list[Path]:
    manifest = directory / "SHA256SUMS"
    entries, _, malformed = parse(manifest)
    if malformed:
        raise SystemExit(f"malformed local manifest: {directory}")
    files = sorted(
        path for path in directory.iterdir()
        if path.is_file() and path.name != "SHA256SUMS"
    )
    if set(entries) != {path.name for path in files}:
        raise SystemExit(f"local manifest file set mismatch: {directory}")
    for path in files:
        if path.is_symlink() or entries[path.name] != digest(path.read_bytes()):
            raise SystemExit(f"local evidence hash mismatch: {path}")
    return [manifest, *files]


local_files = verify_local(ROOT)
indexed_files: list[Path] = list(local_files)
for directory in RUN_DIRS:
    if directory.is_dir():
        indexed_files.extend(verify_local(directory))

worktree, worktree_blanks, worktree_bad = parse(WORKTREE_INDEX)
project, project_blanks, project_bad = parse(PROJECT_INDEX)
worktree_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for path in indexed_files:
    name = path.relative_to(WORKTREE).as_posix()
    expected = digest(path.read_bytes())
    if worktree.get(name) != expected or project.get(f"{worktree_prefix}/{name}") != expected:
        raise SystemExit(f"global evidence hash mismatch: {name}")

for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    source = WORKTREE / name
    if source.is_symlink() or not source.is_file() or digest(source.read_bytes()) != value:
        raise SystemExit(f"source hash mismatch: {name}")
    if worktree.get(name) != value or project.get(f"{worktree_prefix}/{name}") != value:
        raise SystemExit(f"global source index mismatch: {name}")

for label, path in (
    ("local", ROOT / "SHA256SUMS"),
    ("worktree", WORKTREE_INDEX),
    ("project", PROJECT_INDEX),
):
    data = path.read_bytes()
    count = PREFIX[label]["line_count"]
    if len(data.splitlines(keepends=True)) < count:
        raise SystemExit(f"manifest prefix shortened: {label}")
    prefix = b"".join(data.splitlines(keepends=True)[:count])
    if digest(prefix) != PREFIX[label]["sha256"]:
        raise SystemExit(f"manifest prefix changed: {label}")

if project.get(f"{worktree_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind current worktree index")

print(json.dumps({
    "status": "verified",
    "local_evidence_files": len(local_files) - 1,
    "composed_run_evidence_files": len(indexed_files) - len(local_files),
    "source_hashes": len((ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines()),
    "historical_prefixes_preserved": True,
    "historical_blank_and_malformed_rows_preserved": {
        "worktree": {"blank": worktree_blanks, "malformed": worktree_bad},
        "project": {"blank": project_blanks, "malformed": project_bad},
    },
}, sort_keys=True))
