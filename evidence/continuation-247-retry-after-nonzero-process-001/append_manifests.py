#!/usr/bin/env python3
"""Append CONTINUATION-247 evidence while preserving all manifest prefixes."""
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
PREFIX = ROOT / "manifest-prefix-before.json"
FILES = (
    "append_manifests.py", "manifest-prefix-before.json", "local-run.md", "commands.txt",
    "source-review.txt", "tool-versions.txt", "source-sha256.txt", "test.log", "test.exit",
    "format-check.log", "format-check.exit", "diff-check.log", "diff-check.exit",
    "runner-ast.log", "runner-ast.exit", "license-metadata.log", "license-metadata.exit",
    "license-guard-tests.log", "license-guard-tests.exit",
)
SOURCES = (
    "Cargo.toml", "Cargo.lock", "tests/webhook_receiver_process_e2e.rs",
    "scripts/run_webhook_reconcile_cli_e2e.py", "src/bin/forgejo-reconcile-local.rs",
    "src/webhook/reconcile.rs", "docs/checkpoint.md", "docs/refusal-evidence-matrix.md",
    "evidence/continuation-247-retry-after-nonzero-process-001/test.log",
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


for path in (WI, PI):
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"global index unavailable: {path}")
if LOCAL.exists() and (LOCAL.is_symlink() or LOCAL.stat().st_size):
    raise SystemExit("local index is nonempty or symlinked")
LOCAL.touch(exist_ok=True)
current = {
    "local": {"count": len(LOCAL.read_bytes().splitlines()), "sha256": digest(LOCAL.read_bytes())},
    "worktree": {"count": len(WI.read_bytes().splitlines()), "sha256": digest(WI.read_bytes())},
    "project": {"count": len(PI.read_bytes().splitlines()), "sha256": digest(PI.read_bytes())},
}
if PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("prefix receipt already exists")
PREFIX.write_text(json.dumps(current, sort_keys=True, indent=2) + "\n", encoding="utf-8")
local_items: list[tuple[str, str]] = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"local evidence unavailable: {name}")
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
    data = path.read_bytes()
    lines = data.splitlines(keepends=True)
    prior = current[label]
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
    "all_prefixes_preserved": True,
    "latest_entries_verified": True,
}, sort_keys=True))
