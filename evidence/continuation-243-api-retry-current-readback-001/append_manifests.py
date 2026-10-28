#!/usr/bin/env python3
"""Append this evidence bundle while preserving global index prefixes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL_INDEX = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX_RECEIPT = ROOT / "manifest-prefix-before.json"
FILES = (
    "append_manifests.py",
    "manifest-prefix-before.json",
    "local-run.md",
    "commands.txt",
    "source-review.txt",
    "tool-versions.txt",
    "source-sha256.txt",
    "host-port-before.txt",
    "process-e2e.log",
    "process-e2e.exit",
    "process-e2e-final.log",
    "process-e2e-final.exit",
    "rustfmt.log",
    "rustfmt.exit",
    "rustfmt-final.log",
    "rustfmt-final.exit",
    "diff-check.log",
    "diff-check.exit",
    "diff-check-final.log",
    "diff-check-final.exit",
    "runner-ast.log",
    "runner-ast.exit",
    "runner-ast-final.log",
    "runner-ast-final.exit",
    "target-whitespace.log",
    "target-whitespace.exit",
)
SOURCE_PATHS = (
    "tests/webhook_receiver_process_e2e.rs",
    "scripts/run_webhook_reconcile_cli_e2e.py",
    "src/bin/forgejo-webhook-local.rs",
    "src/bin/forgejo-reconcile-local.rs",
    "src/webhook/reconcile.rs",
    "docs/checkpoint.md",
    "docs/refusal-evidence-matrix.md",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


def latest_entries(path: Path) -> dict[str, str]:
    entries: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and name:
            entries[name] = value
    return entries


for name, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"{name} append-only index unavailable: {index}")
if LOCAL_INDEX.exists() and (LOCAL_INDEX.is_symlink() or LOCAL_INDEX.stat().st_size):
    raise SystemExit("local index is nonempty or symlinked; refusing rewrite")
LOCAL_INDEX.touch(exist_ok=True)
current = {
    "local": {"line_count": 0, "sha256": digest(b"")},
    "worktree": {
        "line_count": len(WORKTREE_INDEX.read_bytes().splitlines()),
        "sha256": digest(WORKTREE_INDEX.read_bytes()),
    },
    "project": {
        "line_count": len(PROJECT_INDEX.read_bytes().splitlines()),
        "sha256": digest(PROJECT_INDEX.read_bytes()),
    },
}
if PREFIX_RECEIPT.is_symlink():
    raise SystemExit("prefix receipt is a symlink")
if PREFIX_RECEIPT.exists():
    before = json.loads(PREFIX_RECEIPT.read_text(encoding="utf-8"))
    if before != current:
        raise SystemExit("prefix receipt does not match untouched indexes")
else:
    before = current
    PREFIX_RECEIPT.write_text(
        json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )

local_entries: list[tuple[str, str]] = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"evidence file unavailable: {name}")
    local_entries.append((digest(path.read_bytes()), name))
for name in SOURCE_PATHS:
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"source file unavailable: {name}")
append(LOCAL_INDEX, local_entries)
local_digest = digest(LOCAL_INDEX.read_bytes())
worktree_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_entries = [(value, f"{worktree_prefix}/{name}") for value, name in local_entries]
project_entries = [(value, f"{project_prefix}/{worktree_prefix}/{name}") for value, name in local_entries]
worktree_entries.append((local_digest, f"{worktree_prefix}/SHA256SUMS"))
project_entries.append((local_digest, f"{project_prefix}/{worktree_prefix}/SHA256SUMS"))
for name in SOURCE_PATHS:
    value = digest((WORKTREE / name).read_bytes())
    worktree_entries.append((value, name))
    project_entries.append((value, f"{project_prefix}/{name}"))
append(WORKTREE_INDEX, worktree_entries)
project_entries.append((digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_entries)

for key, index in (("local", LOCAL_INDEX), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = index.read_bytes().splitlines(keepends=True)
    prefix = b"".join(lines[: before[key]["line_count"]])
    if digest(prefix) != before[key]["sha256"]:
        raise SystemExit(f"{key} index prefix changed")
for index, entries in ((WORKTREE_INDEX, worktree_entries), (PROJECT_INDEX, project_entries)):
    latest = latest_entries(index)
    for value, name in entries:
        if latest.get(name) != value:
            raise SystemExit(f"latest global entry mismatch: {name}")
print(json.dumps({
    "local_entries_appended": len(local_entries),
    "worktree_entries_appended": len(worktree_entries),
    "project_entries_appended": len(project_entries),
    "all_prefixes_preserved": True,
    "latest_global_entries_match": True,
}, sort_keys=True))
