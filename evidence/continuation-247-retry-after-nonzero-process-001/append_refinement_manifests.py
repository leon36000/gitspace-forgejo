#!/usr/bin/env python3
"""Append final timing-proof refinements without replacing any manifest entry."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WI = WORKTREE / "evidence/SHA256SUMS"
PI = PROJECT / "evidence/SHA256SUMS"
PREFIX = ROOT / "manifest-prefix-after-001.json"
FILES = (
    "append_refinement_manifests.py", "manifest-prefix-after-001.json", "local-run.md",
    "commands.txt", "source-review.txt", "source-sha256.txt", "test.log",
    "final-run-001.log", "initial-timing-probe-before-refinement.log",
)
SOURCES = (
    "Cargo.toml", "Cargo.lock", "tests/webhook_receiver_process_e2e.rs",
    "scripts/run_webhook_reconcile_cli_e2e.py", "src/bin/forgejo-reconcile-local.rs",
    "src/webhook/reconcile.rs", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md",
    "evidence/continuation-247-retry-after-nonzero-process-001/test.log",
    "evidence/continuation-247-retry-after-nonzero-process-001/final-run-001.log",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    output: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode().split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and name:
            output[name] = value
    return output


def append(path: Path, items: list[tuple[str, str]]) -> None:
    with path.open("ab") as handle:
        for value, name in items:
            handle.write(f"{value}  {name}\n".encode())
        handle.flush()
        os.fsync(handle.fileno())


for path in (LOCAL, WI, PI):
    if path.is_symlink() or (path != LOCAL and not path.is_file()):
        raise SystemExit(f"manifest unavailable or symlinked: {path}")
if PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("refinement prefix receipt already exists")
before = {
    "local": {"count": len(LOCAL.read_bytes().splitlines()), "sha256": digest(LOCAL.read_bytes())},
    "worktree": {"count": len(WI.read_bytes().splitlines()), "sha256": digest(WI.read_bytes())},
    "project": {"count": len(PI.read_bytes().splitlines()), "sha256": digest(PI.read_bytes())},
}
PREFIX.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")
local_items: list[tuple[str, str]] = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"refinement evidence unavailable: {name}")
    local_items.append((digest(path.read_bytes()), name))
for name in SOURCES:
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"source unavailable: {name}")
append(LOCAL, local_items)
local_hash = digest(LOCAL.read_bytes())
worktree_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_items = [(value, f"{worktree_prefix}/{name}") for value, name in local_items]
worktree_items.append((local_hash, f"{worktree_prefix}/SHA256SUMS"))
project_items = [(value, f"{project_prefix}/{worktree_prefix}/{name}") for value, name in local_items]
project_items.append((local_hash, f"{project_prefix}/{worktree_prefix}/SHA256SUMS"))
for name in SOURCES:
    value = digest((WORKTREE / name).read_bytes())
    worktree_items.append((value, name))
    project_items.append((value, f"{project_prefix}/{name}"))
append(WI, worktree_items)
project_items.append((digest(WI.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"))
append(PI, project_items)
for label, path in (("local", LOCAL), ("worktree", WI), ("project", PI)):
    lines = path.read_bytes().splitlines(keepends=True)
    prior = before[label]
    if digest(b"".join(lines[: prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"{label} manifest prefix changed")
for path, items in ((WI, worktree_items), (PI, project_items)):
    seen = latest(path)
    for value, name in items:
        if seen.get(name) != value:
            raise SystemExit(f"latest global hash mismatch: {name}")
print(json.dumps({
    "local_entries_appended": len(local_items),
    "worktree_entries_appended": len(worktree_items),
    "project_entries_appended": len(project_items),
    "all_previous_prefixes_preserved": True,
    "latest_entries_verified": True,
}, sort_keys=True))
