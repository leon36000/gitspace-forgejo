#!/usr/bin/env python3
"""Independently verify this receipt and its append-only global index links."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, sep, rel = line.partition("  ")
        if sep:
            out[rel] = value
    return out


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(lines) < expected["line_count"] or digest(b"".join(lines[: expected["line_count"]])) != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix mismatch")


for path, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(path, key)

local_latest = latest(LOCAL)
actual = {path.name for path in ROOT.iterdir() if path.is_file() and path != LOCAL}
if actual != set(local_latest):
    raise SystemExit("local evidence manifest is incomplete")
work_latest = latest(WORKTREE_INDEX)
project_latest = latest(PROJECT_INDEX)
for name in sorted(actual):
    path = ROOT / name
    value = digest(path.read_bytes())
    work_rel = path.relative_to(WORKTREE).as_posix()
    project_rel = (Path(".runtime/webhook-005-workspace") / work_rel).as_posix()
    if local_latest[name] != value or work_latest.get(work_rel) != value or project_latest.get(project_rel) != value:
        raise SystemExit(f"evidence hash mismatch: {name}")

for rel in ("docs/architecture.md", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md"):
    value = digest((WORKTREE / rel).read_bytes())
    project_rel = (Path(".runtime/webhook-005-workspace") / rel).as_posix()
    if work_latest.get(rel) != value or project_latest.get(project_rel) != value:
        raise SystemExit(f"documentation hash mismatch: {rel}")
if project_latest.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("parent index does not bind current child index")

print(json.dumps({
    "local_evidence_files_verified": len(actual),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
    "parent_binds_current_worktree_index": True,
}, sort_keys=True))
