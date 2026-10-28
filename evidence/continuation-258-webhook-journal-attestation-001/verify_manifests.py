#!/usr/bin/env python3
"""Verify the CONTINUATION-258 evidence hashes and both append-only index prefixes."""
from __future__ import annotations
import hashlib, json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"

def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def parse(data: bytes) -> tuple[dict[str, str], int, int]:
    latest: dict[str, str] = {}
    blanks = malformed = 0
    for raw in data.splitlines():
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

def verify_prefix(path: Path, label: str) -> None:
    data = path.read_bytes()
    count = int(PREFIX[label]["line_count"])
    old_prefix = b"".join(data.splitlines(keepends=True)[:count])
    if len(data.splitlines(keepends=True)) < count or digest(old_prefix) != PREFIX[label]["sha256"]:
        raise SystemExit(f"historical {label} index prefix changed")

if not LOCAL.is_file() or LOCAL.is_symlink():
    raise SystemExit("local SHA256SUMS missing")
local, local_blanks, local_malformed = parse(LOCAL.read_bytes())
if local_blanks or local_malformed:
    raise SystemExit("local evidence manifest contains malformed rows")
files = sorted(p for p in ROOT.iterdir() if p.is_file() and p.name != "SHA256SUMS")
if set(local) != {p.name for p in files}:
    raise SystemExit("local manifest file inventory differs from evidence directory")
for path in files:
    if digest(path.read_bytes()) != local[path.name]:
        raise SystemExit(f"local evidence hash mismatch: {path.name}")

worktree, wb, wm = parse(WORKTREE_INDEX.read_bytes())
project, pb, pm = parse(PROJECT_INDEX.read_bytes())
verify_prefix(WORKTREE_INDEX, "worktree")
verify_prefix(PROJECT_INDEX, "project")
worktree_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
for name, value in local.items():
    if worktree.get(f"{worktree_prefix}/{name}") != value:
        raise SystemExit(f"worktree index mismatch: {name}")
    if project.get(f"{project_prefix}/{worktree_prefix}/{name}") != value:
        raise SystemExit(f"project index mismatch: {name}")
local_hash = digest(LOCAL.read_bytes())
if worktree.get(f"{worktree_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree local-manifest hash mismatch")
if project.get(f"{project_prefix}/{worktree_prefix}/SHA256SUMS") != local_hash:
    raise SystemExit("project local-manifest hash mismatch")
source_count = 0
for raw in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = raw.split("  ", 1)
    source_count += 1
    if digest((WORKTREE / name).read_bytes()) != value:
        raise SystemExit(f"current source file hash mismatch: {name}")
    if worktree.get(name) != value or project.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"global source index mismatch: {name}")
if project.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind the updated worktree index")
print(json.dumps({"status": "verified", "local_evidence_entries": len(local),
                  "source_files": source_count, "historical_prefixes_preserved": True,
                  "latest_hashes_verified": True,
                  "historical_worktree_blank_rows": wb, "historical_worktree_malformed_rows": wm,
                  "historical_project_blank_rows": pb, "historical_project_malformed_rows": pm}, sort_keys=True))
