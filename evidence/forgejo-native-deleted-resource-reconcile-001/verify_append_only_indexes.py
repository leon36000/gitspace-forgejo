#!/usr/bin/env python3
"""Independently verify evidence coverage and append-only manifest links."""
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
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, sep, name = line.partition("  ")
        if sep:
            result[name] = value
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(lines) < expected["line_count"] or digest(b"".join(lines[: expected["line_count"]])) != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix mismatch")


for path, key in ((LOCAL, "local"), (WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    check_prefix(path, key)

local = latest(LOCAL)
actual = {path.name for path in ROOT.iterdir() if path.is_file() and path != LOCAL}
if actual != set(local):
    raise SystemExit("local manifest does not cover every evidence file")
work = latest(WORKTREE_INDEX)
project = latest(PROJECT_INDEX)
for name, value in local.items():
    path = ROOT / name
    if digest(path.read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    work_name = path.relative_to(WORKTREE).as_posix()
    project_name = (Path(".runtime/webhook-005-workspace") / work_name).as_posix()
    if work.get(work_name) != value or project.get(project_name) != value:
        raise SystemExit(f"global evidence hash mismatch: {name}")
for name in ("docs/architecture.md", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md"):
    value = digest((WORKTREE / name).read_bytes())
    project_name = (Path(".runtime/webhook-005-workspace") / name).as_posix()
    if work.get(name) != value or project.get(project_name) != value:
        raise SystemExit(f"documentation hash mismatch: {name}")
if project.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not bind current worktree index")

print(json.dumps({
    "local_files_verified": len(actual),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
    "project_binds_current_worktree_index": True,
}, sort_keys=True))
