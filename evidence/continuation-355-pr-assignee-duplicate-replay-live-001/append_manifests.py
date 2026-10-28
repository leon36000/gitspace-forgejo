#!/usr/bin/env python3
"""Append CONTINUATION-355 evidence and snapshots to append-only indexes."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL_INDEX = ROOT / "SHA256SUMS"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
EXCLUDED_ROOT_NAMES = {"SHA256SUMS", "manifest-check.txt", "global-manifest-check.txt"}
SNAPSHOTS = (
    "docs/checkpoint.md",
    "scripts/run_forgejo_live_webhook_reconcile_emitted.py",
    "tests/test_forgejo_emitted_webhook_e2e.py",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def append(path: Path, entries: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in entries:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()


def latest_entries(index: Path) -> tuple[dict[str, str], int]:
    latest: dict[str, str] = {}
    malformed = 0
    for line in index.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if match is None:
            malformed += 1
            continue
        latest[match.group(2)] = match.group(1)
    return latest, malformed


prefix_path = ROOT / "manifest-prefix-before.json"
if prefix_path.exists():
    prefix = json.loads(prefix_path.read_text(encoding="utf-8"))
else:
    prefix = {}
    for key, index in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
        if not index.is_file() or index.is_symlink():
            raise SystemExit(f"append-only index unavailable: {index}")
        data = index.read_bytes()
        if data and not data.endswith(b"\n"):
            raise SystemExit(f"append-only index lacks final newline: {index}")
        prefix[key] = {"line_count": len(data.splitlines()), "sha256": digest(data)}
    prefix_path.write_text(json.dumps(prefix, sort_keys=True, indent=2) + "\n", encoding="utf-8")

for relative in SNAPSHOTS:
    path = WORKTREE / relative
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"required source snapshot unavailable: {relative}")

files = []
for path in ROOT.rglob("*"):
    if path.is_symlink():
        raise SystemExit(f"symlink in evidence directory: {path.relative_to(ROOT)}")
    relative = path.relative_to(ROOT)
    if path.is_file() and not (len(relative.parts) == 1 and path.name in EXCLUDED_ROOT_NAMES):
        files.append(path)
files.sort(key=lambda path: path.relative_to(ROOT).as_posix())
local_entries = [
    (digest(path.read_bytes()), path.relative_to(ROOT).as_posix()) for path in files
]
local_latest = latest_entries(LOCAL_INDEX)[0] if LOCAL_INDEX.is_file() else {}
local_append = [(value, name) for value, name in local_entries if local_latest.get(name) != value]
append(LOCAL_INDEX, local_append)
local_digest = digest(LOCAL_INDEX.read_bytes())

worktree_targets = [
    (value, f"evidence/{ROOT.name}/{name}") for value, name in local_entries
]
worktree_targets.append((local_digest, f"evidence/{ROOT.name}/SHA256SUMS"))
project_targets = [
    (value, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/{name}")
    for value, name in local_entries
]
project_targets.append(
    (local_digest, f".runtime/webhook-005-workspace/evidence/{ROOT.name}/SHA256SUMS")
)
for relative in SNAPSHOTS:
    value = digest((WORKTREE / relative).read_bytes())
    worktree_targets.append((value, relative))
    project_targets.append((value, f".runtime/webhook-005-workspace/{relative}"))

worktree_latest, worktree_malformed = latest_entries(WORKTREE_INDEX)
worktree_append = [
    (value, name) for value, name in worktree_targets if worktree_latest.get(name) != value
]
append(WORKTREE_INDEX, worktree_append)

project_latest, project_malformed = latest_entries(PROJECT_INDEX)
project_append = [
    (value, name) for value, name in project_targets if project_latest.get(name) != value
]
project_tail_name = ".runtime/webhook-005-workspace/evidence/SHA256SUMS"
project_append.append((digest(WORKTREE_INDEX.read_bytes()), project_tail_name))
append(PROJECT_INDEX, project_append)

def assert_prefix(key: str, index: Path) -> None:
    before = prefix[key]
    lines_before = int(before["line_count"])
    prior_lines = index.read_bytes().splitlines(keepends=True)[:lines_before]
    prior = b"".join(prior_lines)
    if digest(prior) != before["sha256"]:
        raise SystemExit(f"append-only prefix changed: {key}")


assert_prefix("worktree", WORKTREE_INDEX)
assert_prefix("project", PROJECT_INDEX)
for value, name in local_entries:
    if digest((ROOT / name).read_bytes()) != value:
        raise SystemExit(f"local evidence hash mismatch: {name}")
worktree_latest = latest_entries(WORKTREE_INDEX)[0]
project_latest = latest_entries(PROJECT_INDEX)[0]
for value, name in worktree_targets:
    if worktree_latest.get(name) != value:
        raise SystemExit(f"worktree latest entry mismatch: {name}")
for value, name in project_targets:
    if project_latest.get(name) != value:
        raise SystemExit(f"project latest entry mismatch: {name}")
if project_latest.get(project_tail_name) != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project manifest does not bind the current worktree index")

(ROOT / "manifest-check.txt").write_text(
    "\n".join(
        [
            "append_only_prefixes_preserved=true",
            f"worktree_prefix_lines={prefix['worktree']['line_count']}",
            f"project_prefix_lines={prefix['project']['line_count']}",
            f"local_evidence_files={len(local_entries)}",
            f"local_entries_appended={len(local_append)}",
            f"worktree_entries_appended={len(worktree_append)}",
            f"project_entries_appended={len(project_append)}",
            f"worktree_malformed_legacy_lines_ignored={worktree_malformed}",
            f"project_malformed_legacy_lines_ignored={project_malformed}",
            "all_local_evidence_hashes_valid=true",
            "all_appended_global_latest_entries_valid=true",
            "project_tail_binds_current_worktree_index=true",
        ]
    )
    + "\n",
    encoding="utf-8",
)
