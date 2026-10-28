#!/usr/bin/env python3
"""Append final CONT-251 hashes and prove both earlier manifest prefixes remain intact."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
ORIGINAL_PREFIX = ROOT / "manifest-prefix-before.json"
REPAIR_PREFIX = ROOT / "repair-prefix-before.json"
FILES = (
    "append_manifests.py",
    "append_final_manifests.py",
    "manifest-prefix-before.json",
    "repair-prefix-before.json",
    "local-run.md",
    "commands.txt",
    "environment.txt",
    "source-comparison.json",
    "source-review.txt",
    "source-sha256.txt",
    "text-whitespace-check.log",
    "diff-check.log",
)
SOURCES = (
    "scripts/mission_012_mcp_supervisor.py",
    "scripts/mission_012_sandboxed_test_adapter.py",
    "scripts/mission_012_test_agent_adapter.py",
    "scripts/run_mission_012_live_mcp_agent_e2e.py",
    "scripts/run_mission_012_live_composed_e2e.py",
    "tests/test_mission_012_mcp_supervisor.py",
    "tests/test_mission_012_sandboxed_test_adapter.py",
    "tests/test_mission_012_test_agent_adapter.py",
    "src/mission_012_host.rs",
    "docs/checkpoint.md",
    "docs/refusal-evidence-matrix.md",
    "evidence/continuation-245-same-host-gate-001/successful-composition.log",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def latest(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            value, name = raw.decode().split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(value) == 64 and name:
            result[name] = value
    return result


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode())
        stream.flush()
        os.fsync(stream.fileno())


if LOCAL.is_symlink() or not LOCAL.is_file() or not ORIGINAL_PREFIX.is_file() or ORIGINAL_PREFIX.is_symlink():
    raise SystemExit("first CONT-251 manifest or its prefix receipt is unavailable")
if REPAIR_PREFIX.exists() or REPAIR_PREFIX.is_symlink():
    raise SystemExit("final prefix receipt already exists")
for index in (WORKTREE_INDEX, PROJECT_INDEX):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"global manifest unavailable: {index}")

original = json.loads(ORIGINAL_PREFIX.read_text(encoding="utf-8"))
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = path.read_bytes().splitlines(keepends=True)
    prior = original[label]
    if digest(b"".join(lines[: prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"original {label} manifest prefix changed")
local_bytes = LOCAL.read_bytes()
before = {
    "local": {"count": len(local_bytes.splitlines()), "sha256": digest(local_bytes)},
    "worktree": {"count": len(WORKTREE_INDEX.read_bytes().splitlines()), "sha256": digest(WORKTREE_INDEX.read_bytes())},
    "project": {"count": len(PROJECT_INDEX.read_bytes().splitlines()), "sha256": digest(PROJECT_INDEX.read_bytes())},
}
REPAIR_PREFIX.write_text(json.dumps(before, sort_keys=True, indent=2) + "\n", encoding="utf-8")

local_entries: list[tuple[str, str]] = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"final evidence unavailable: {name}")
    local_entries.append((digest(path.read_bytes()), name))
for name in SOURCES:
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"source unavailable: {name}")

append(LOCAL, local_entries)
local_hash = digest(LOCAL.read_bytes())
worktree_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
worktree_entries = [(value, f"{worktree_prefix}/{name}") for value, name in local_entries]
worktree_entries.append((local_hash, f"{worktree_prefix}/SHA256SUMS"))
project_entries = [(value, f"{project_prefix}/{worktree_prefix}/{name}") for value, name in local_entries]
project_entries.append((local_hash, f"{project_prefix}/{worktree_prefix}/SHA256SUMS"))
for name in SOURCES:
    value = digest((WORKTREE / name).read_bytes())
    worktree_entries.append((value, name))
    project_entries.append((value, f"{project_prefix}/{name}"))
append(WORKTREE_INDEX, worktree_entries)
project_entries.append((digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_entries)

for label, path in (("local", LOCAL), ("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    lines = path.read_bytes().splitlines(keepends=True)
    prior = before[label]
    if digest(b"".join(lines[: prior["count"]])) != prior["sha256"]:
        raise SystemExit(f"final {label} manifest prefix changed")
for path, entries in ((WORKTREE_INDEX, worktree_entries), (PROJECT_INDEX, project_entries)):
    current = latest(path)
    if any(current.get(name) != value for value, name in entries):
        raise SystemExit(f"latest entries in {path.name} do not verify")
print(json.dumps({
    "final_entries_appended": len(local_entries),
    "worktree_entries_appended": len(worktree_entries),
    "project_entries_appended": len(project_entries),
    "original_prefixes_preserved": True,
    "intermediate_prefixes_preserved": True,
    "latest_entries_verified": True,
}, sort_keys=True))
