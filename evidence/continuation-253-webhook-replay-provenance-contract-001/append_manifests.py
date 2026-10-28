#!/usr/bin/env python3
"""Append CONT-253 evidence and source hashes without rewriting prefixes."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
LOCAL = ROOT / "SHA256SUMS"
PREFIX = ROOT / "manifest-prefix-before.json"
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
FILES = (
    "append_manifests.py",
    "verify_manifests.py",
    "replay_checks.py",
    "commands.txt",
    "checks-summary.json",
    "manifest-prefix-before.json",
    "environment.json",
    "output-contract.json",
    "red-phase.txt",
    "local-run.md",
    "source-sha256.txt",
    "rust-reconcile-unit.log",
    "capabilities-contract.log",
    "single-batch-process-contract.log",
    "quarantine-cli-versions.log",
    "journal-lock-cli-version.log",
    "bubblewrap-batch-readback.log",
    "python-emitted-consumers.log",
    "python-org-review-consumers.log",
    "python-native-replay-consumers.log",
    "python-runner-syntax.log",
    "rustfmt.log",
    "diff-check.log",
    "cargo-license-policy.log",
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parsed(path: Path) -> tuple[dict[str, str], int, int]:
    latest: dict[str, str] = {}
    blanks = malformed = 0
    for raw in path.read_bytes().splitlines():
        if not raw.strip():
            blanks += 1
            continue
        try:
            value, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            malformed += 1
            continue
        if (
            len(value) != 64
            or any(char not in "0123456789abcdef" for char in value)
            or not name
        ):
            malformed += 1
            continue
        latest[name] = value
    return latest, blanks, malformed


def append(path: Path, values: list[tuple[str, str]]) -> None:
    with path.open("ab") as stream:
        for value, name in values:
            stream.write(f"{value}  {name}\n".encode("utf-8"))
        stream.flush()
        os.fsync(stream.fileno())


if LOCAL.exists() or LOCAL.is_symlink() or PREFIX.exists() or PREFIX.is_symlink():
    raise SystemExit("local manifest or prefix receipt already exists")
for index in (WORKTREE_INDEX, PROJECT_INDEX):
    if not index.is_file() or index.is_symlink():
        raise SystemExit(f"global manifest unavailable: {index}")
    existing = index.read_bytes()
    if existing and not existing.endswith(b"\n"):
        raise SystemExit(f"global manifest has an unterminated final row: {index}")

before: dict[str, dict[str, int | str]] = {}
for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    content = path.read_bytes()
    before[label] = {"count": len(content.splitlines(keepends=True)), "sha256": digest(content)}
PREFIX.write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")

local_values: list[tuple[str, str]] = []
for name in FILES:
    path = ROOT / name
    if not path.is_file() or path.is_symlink():
        raise SystemExit(f"evidence file unavailable: {name}")
    local_values.append((digest(path.read_bytes()), name))

source_values: list[tuple[str, str]] = []
for line in (ROOT / "source-sha256.txt").read_text(encoding="utf-8").splitlines():
    value, name = line.split("  ", 1)
    path = WORKTREE / name
    if not path.is_file() or path.is_symlink() or digest(path.read_bytes()) != value:
        raise SystemExit(f"source hash is stale or unavailable: {name}")
    source_values.append((value, name))

with LOCAL.open("xb") as stream:
    for value, name in local_values:
        stream.write(f"{value}  {name}\n".encode("utf-8"))
    stream.flush()
    os.fsync(stream.fileno())

evidence_prefix = ROOT.relative_to(WORKTREE).as_posix()
project_prefix = WORKTREE.relative_to(PROJECT).as_posix()
local_manifest_hash = digest(LOCAL.read_bytes())
worktree_values = [(value, f"{evidence_prefix}/{name}") for value, name in local_values]
worktree_values.append((local_manifest_hash, f"{evidence_prefix}/SHA256SUMS"))
worktree_values.extend(source_values)
project_values = [
    (value, f"{project_prefix}/{evidence_prefix}/{name}") for value, name in local_values
]
project_values.append((local_manifest_hash, f"{project_prefix}/{evidence_prefix}/SHA256SUMS"))
project_values.extend((value, f"{project_prefix}/{name}") for value, name in source_values)
append(WORKTREE_INDEX, worktree_values)
project_values.append((digest(WORKTREE_INDEX.read_bytes()), f"{project_prefix}/evidence/SHA256SUMS"))
append(PROJECT_INDEX, project_values)

for label, path in (("worktree", WORKTREE_INDEX), ("project", PROJECT_INDEX)):
    data = path.read_bytes()
    old = before[label]
    old_lines = data.splitlines(keepends=True)[: int(old["count"])]
    if digest(b"".join(old_lines)) != old["sha256"]:
        raise SystemExit(f"original {label} prefix changed")

local_latest, local_blanks, local_bad = parsed(LOCAL)
worktree_latest, worktree_blanks, worktree_bad = parsed(WORKTREE_INDEX)
project_latest, project_blanks, project_bad = parsed(PROJECT_INDEX)
if local_blanks or local_bad:
    raise SystemExit("local evidence index unexpectedly has blank or malformed rows")
for value, name in local_values:
    if local_latest.get(name) != value:
        raise SystemExit(f"local index mismatch: {name}")
    if worktree_latest.get(f"{evidence_prefix}/{name}") != value:
        raise SystemExit(f"worktree index mismatch: {name}")
    if project_latest.get(f"{project_prefix}/{evidence_prefix}/{name}") != value:
        raise SystemExit(f"project index mismatch: {name}")
for value, name in source_values:
    if worktree_latest.get(name) != value or project_latest.get(f"{project_prefix}/{name}") != value:
        raise SystemExit(f"source index mismatch: {name}")
if worktree_latest.get(f"{evidence_prefix}/SHA256SUMS") != local_manifest_hash:
    raise SystemExit("worktree index does not match local evidence manifest")
if project_latest.get(f"{project_prefix}/evidence/SHA256SUMS") != digest(WORKTREE_INDEX.read_bytes()):
    raise SystemExit("project index does not match worktree evidence manifest")

print(json.dumps({
    "status": "verified",
    "local_evidence_entries": len(local_values),
    "source_entries_verified": len(source_values),
    "worktree_rows_appended": len(worktree_values),
    "project_rows_appended": len(project_values),
    "original_prefixes_preserved": True,
    "latest_hashes_verified": True,
    "historical_blank_or_malformed_rows_retained": worktree_blanks + worktree_bad + project_blanks + project_bad,
}, sort_keys=True))
