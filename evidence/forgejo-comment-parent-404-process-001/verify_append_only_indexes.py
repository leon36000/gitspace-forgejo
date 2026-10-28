#!/usr/bin/env python3
"""Verify evidence coverage and append-only links independently of the writer."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
PREFIX = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))
LOCAL = ROOT / "SHA256SUMS"
WORK = WORKTREE / "evidence/SHA256SUMS"
PARENT = PROJECT / "evidence/SHA256SUMS"
DOCS = ("docs/checkpoint.md", "docs/refusal-evidence-matrix.md", "docs/source-inventory.md")


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, separator, name = line.partition("  ")
        if separator:
            result[name] = value
    return result


def check_prefix(path: Path, key: str) -> None:
    lines = path.read_bytes().splitlines(keepends=True)
    count = PREFIX[key]["line_count"]
    if len(lines) < count or sha(b"".join(lines[:count])) != PREFIX[key]["sha256"]:
        raise SystemExit(f"{key} append-only prefix mismatch")


for path, key in ((LOCAL, "local"), (WORK, "worktree"), (PARENT, "project")):
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"missing or redirected {key} index")
    check_prefix(path, key)

local = latest(LOCAL)
actual = {path.name for path in ROOT.iterdir() if path.is_file() and path != LOCAL}
if actual != set(local):
    raise SystemExit("local index coverage mismatch")
work = latest(WORK)
parent = latest(PARENT)
for name, value in local.items():
    path = ROOT / name
    if path.is_symlink() or sha(path.read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
    work_name = path.relative_to(WORKTREE).as_posix()
    parent_name = (Path(".runtime/webhook-005-workspace") / work_name).as_posix()
    if work.get(work_name) != value or parent.get(parent_name) != value:
        raise SystemExit(f"global evidence hash mismatch: {name}")
for name in DOCS:
    value = sha((WORKTREE / name).read_bytes())
    parent_name = (Path(".runtime/webhook-005-workspace") / name).as_posix()
    if work.get(name) != value or parent.get(parent_name) != value:
        raise SystemExit(f"documentation hash mismatch: {name}")
if parent.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != sha(WORK.read_bytes()):
    raise SystemExit("project index does not bind current worktree index")
print(json.dumps({"local_files_verified": len(actual), "old_prefixes_preserved": True, "project_binds_current_worktree_index": True}, sort_keys=True))
