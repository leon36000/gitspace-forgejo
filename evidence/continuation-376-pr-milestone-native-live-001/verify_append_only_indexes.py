#!/usr/bin/env python3
"""Verify CONTINUATION-376 local evidence and append-only index bindings."""
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
DOCS = (
    "docs/checkpoint.md",
    "docs/implementation-plan.md",
    "docs/refusal-evidence-matrix.md",
    "docs/architecture.md",
    "docs/source-inventory.md",
)
EXCLUDED_NAMES = {"manifest-check.txt", "global-manifest-check.txt"}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def entries(path: Path) -> dict[str, str]:
    values = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, separator, name = line.partition("  ")
        if separator:
            values[name] = value
    return values


for key, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(lines) < expected["line_count"] or digest(
        b"".join(lines[: expected["line_count"]])
    ) != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix mismatch")

actual = {}
for path in ROOT.rglob("*"):
    if "__pycache__" in path.parts:
        continue
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence tree: {path.relative_to(ROOT)}")
    if path.is_file() and path != LOCAL and path.name not in EXCLUDED_NAMES:
        actual[path.relative_to(ROOT).as_posix()] = digest(path.read_bytes())
if actual != entries(LOCAL):
    raise SystemExit("local evidence manifest incomplete or stale")
work = entries(WORKTREE_INDEX)
project = entries(PROJECT_INDEX)
for name, value in actual.items():
    work_name = f"evidence/{ROOT.name}/{name}"
    project_name = f".runtime/webhook-005-workspace/{work_name}"
    if work.get(work_name) != value or project.get(project_name) != value:
        raise SystemExit(f"global index mismatch: {name}")
local_hash = digest(LOCAL.read_bytes())
if work.get(f"evidence/{ROOT.name}/SHA256SUMS") != local_hash:
    raise SystemExit("worktree index does not bind local manifest")
if project.get(f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS") != local_hash:
    raise SystemExit("project index does not bind local manifest")
for relative in DOCS:
    value = digest((WORKTREE / relative).read_bytes())
    if work.get(relative) != value or project.get(f".runtime/webhook-005-workspace/{relative}") != value:
        raise SystemExit(f"documentation hash mismatch: {relative}")
if project.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != digest(
    WORKTREE_INDEX.read_bytes()
):
    raise SystemExit("project index does not bind current worktree index")
print(json.dumps({
    "local_evidence_files_verified": len(actual),
    "worktree_prefix_preserved": True,
    "project_prefix_preserved": True,
    "parent_binds_current_worktree_index": True,
}, sort_keys=True))
