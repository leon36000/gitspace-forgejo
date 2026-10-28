#!/usr/bin/env python3
"""Verify CONTINUATION-336 hashes and append-only index prefixes."""

from __future__ import annotations

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
EXCLUDED_NAMES = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
DOCS = ("docs/checkpoint.md",)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    values = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        value, separator, name = line.partition("  ")
        if separator:
            values[name] = value
    return values


for path, key in ((WORKTREE_INDEX, "worktree"), (PROJECT_INDEX, "project")):
    lines = path.read_bytes().splitlines(keepends=True)
    expected = PREFIX[key]
    if len(lines) < expected["line_count"] or digest(b"".join(lines[: expected["line_count"]])) != expected["sha256"]:
        raise SystemExit(f"{key} append-only prefix mismatch")

actual = {}
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence tree: {path.relative_to(ROOT)}")
    if path.is_file() and path.name not in EXCLUDED_NAMES:
        actual[path.relative_to(ROOT).as_posix()] = digest(path.read_bytes())
if actual != latest(LOCAL):
    raise SystemExit("local evidence manifest incomplete or stale")

work = latest(WORKTREE_INDEX)
project = latest(PROJECT_INDEX)
for relative, value in actual.items():
    work_relative = (ROOT / relative).relative_to(WORKTREE).as_posix()
    project_relative = (Path(".runtime/webhook-005-workspace") / work_relative).as_posix()
    if work.get(work_relative) != value or project.get(project_relative) != value:
        raise SystemExit(f"global hash mismatch: {relative}")
local_relative = LOCAL.relative_to(WORKTREE).as_posix()
if work.get(local_relative) != digest(LOCAL.read_bytes()):
    raise SystemExit("worktree index does not bind local manifest")
if project.get(f".runtime/webhook-005-workspace/{local_relative}") != digest(LOCAL.read_bytes()):
    raise SystemExit("project index does not bind local manifest")
for relative in DOCS:
    value = digest((WORKTREE / relative).read_bytes())
    if work.get(relative) != value or project.get(f".runtime/webhook-005-workspace/{relative}") != value:
        raise SystemExit(f"documentation hash mismatch: {relative}")
if project.get(".runtime/webhook-005-workspace/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("parent index does not bind current worktree index")

print(
    json.dumps(
        {
            "local_evidence_files_verified": len(actual),
            "worktree_prefix_preserved": True,
            "project_prefix_preserved": True,
            "parent_binds_current_worktree_index": True,
        },
        sort_keys=True,
    )
)
