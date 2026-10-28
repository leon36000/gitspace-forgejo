#!/usr/bin/env python3
"""Check pre-existing manifest prefixes and the latest hashes for this evidence lot."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
PROJECT = WORKTREE.parents[1]
WORKTREE_INDEX = WORKTREE / "evidence/SHA256SUMS"
PROJECT_INDEX = PROJECT / "evidence/SHA256SUMS"
PREFIX_RECORD = json.loads((ROOT / "manifest-prefix-before.json").read_text(encoding="utf-8"))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_latest(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split("  ", 1)
        if len(parts) == 2:
            latest[parts[1]] = parts[0]
    return latest


def verify_prefix(index: Path, name: str) -> None:
    raw = index.read_bytes()
    lines = raw.splitlines(keepends=True)
    prefix_count = PREFIX_RECORD[name]["line_count_before_append"]
    prefix = b"".join(lines[:prefix_count])
    if hashlib.sha256(prefix).hexdigest() != PREFIX_RECORD[name]["sha256_before_append"]:
        raise ValueError(f"append-only prefix changed: {name}")


def verify_local_manifest(directory: Path) -> int:
    manifest = directory / "SHA256SUMS"
    count = 0
    for line in manifest.read_text(encoding="utf-8").splitlines():
        expected, name = line.split("  ", 1)
        target = (directory / name).resolve(strict=True)
        if directory.resolve() not in target.parents or digest(target) != expected:
            raise ValueError(f"local evidence hash mismatch: {name}")
        count += 1
    return count


def verify() -> dict[str, object]:
    verify_prefix(WORKTREE_INDEX, "worktree")
    verify_prefix(PROJECT_INDEX, "project")
    local_latest = read_latest(WORKTREE_INDEX)
    project_latest = read_latest(PROJECT_INDEX)
    artifact_files = sorted(path for path in ROOT.rglob("*") if path.is_file())
    source_files = [
        WORKTREE / "docs/checkpoint.md",
        WORKTREE / "docs/source-inventory.md",
        WORKTREE / "docs/refusal-evidence-matrix.md",
        WORKTREE / "scripts/run_forgejo_native_branch_delete_webhook_reconcile.py",
        WORKTREE / "tests/test_forgejo_emitted_webhook_e2e.py",
    ]
    checked = 0
    for path in artifact_files + source_files:
        local_name = path.relative_to(WORKTREE).as_posix()
        project_name = (Path(".runtime/webhook-005-workspace") / local_name).as_posix()
        observed = digest(path)
        if local_latest.get(local_name) != observed:
            raise ValueError(f"latest worktree index entry differs: {local_name}")
        if project_latest.get(project_name) != observed:
            raise ValueError(f"latest project index entry differs: {project_name}")
        checked += 1
    worktree_index_name = ".runtime/webhook-005-workspace/evidence/SHA256SUMS"
    if project_latest.get(worktree_index_name) != digest(WORKTREE_INDEX):
        raise ValueError("project index does not contain the current worktree index digest")
    return {
        "worktree_prefix_preserved": True,
        "project_prefix_preserved": True,
        "latest_paths_checked": checked,
        "local_evidence_files_checked": verify_local_manifest(ROOT),
        "worktree_index_tracks_all_latest_artifacts_and_sources": True,
        "project_index_tracks_all_latest_artifacts_sources_and_worktree_index": True,
    }


if __name__ == "__main__":
    print(json.dumps(verify(), sort_keys=True, indent=2))
