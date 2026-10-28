#!/usr/bin/env python3
"""Verify the documentation reconciliation and its referenced receipts."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess


WORKTREE = Path(__file__).resolve().parents[2]
PROJECT = WORKTREE.parents[1]
CURRENT = WORKTREE / "evidence/continuation-233-workflow-action-matrix-reconcile-001"
PREVIOUS = WORKTREE / "evidence/continuation-232-workflow-job-persisted-action-mismatch-001"
REPORT_IDS = (
    "continuation-229-workflow-action-status-001",
    "continuation-230-workflow-action-process-refusal-001",
    "continuation-231-persisted-action-mismatch-cli-001",
    "continuation-232-workflow-job-persisted-action-mismatch-001",
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_latest(path: Path) -> dict[str, str]:
    latest: dict[str, str] = {}
    for raw in path.read_bytes().splitlines():
        try:
            digest, name = raw.decode("utf-8").split("  ", 1)
        except (UnicodeDecodeError, ValueError):
            continue
        if len(digest) == 64:
            latest[name] = digest
    return latest


def main() -> None:
    for report_id in REPORT_IDS:
        report = WORKTREE / "evidence" / report_id / "local-run.md"
        if not report.is_file():
            raise SystemExit(f"missing report: {report_id}")

    manifest_path = PREVIOUS / "SHA256SUMS"
    for line in manifest_path.read_text(encoding="utf-8").splitlines():
        expected, name = line.split("  ", 1)
        if sha256((PREVIOUS / name).read_bytes()) != expected:
            raise SystemExit(f"CONT-232 manifest mismatch: {name}")

    prefix = json.loads((PREVIOUS / "manifest-prefix-before.json").read_text(encoding="utf-8"))
    indexes = {
        "local": manifest_path,
        "worktree": WORKTREE / "evidence/SHA256SUMS",
        "project": PROJECT / "evidence/SHA256SUMS",
    }
    for key, path in indexes.items():
        data = path.read_bytes()
        lines = data.splitlines(keepends=True)
        before = prefix[key]
        actual_prefix = sha256(b"".join(lines[: before["line_count"]]))
        if actual_prefix != before["sha256"]:
            raise SystemExit(f"CONT-232 {key} manifest prefix changed")

    worktree_latest = read_latest(indexes["worktree"])
    project_latest = read_latest(indexes["project"])
    previous_prefix = "evidence/" + PREVIOUS.name + "/"
    project_prefix = WORKTREE.relative_to(PROJECT).as_posix() + "/" + previous_prefix
    for line in manifest_path.read_text(encoding="utf-8").splitlines():
        expected, name = line.split("  ", 1)
        for index, manifest_name in (
            (worktree_latest, previous_prefix + name),
            (project_latest, project_prefix + name),
        ):
            if index.get(manifest_name) != expected:
                raise SystemExit(f"latest global manifest entry mismatch: {manifest_name}")
    for index, manifest_name, expected in (
        (worktree_latest, f"evidence/{PREVIOUS.name}/SHA256SUMS", sha256(manifest_path.read_bytes())),
        (project_latest, f"{WORKTREE.relative_to(PROJECT).as_posix()}/evidence/{PREVIOUS.name}/SHA256SUMS", sha256(manifest_path.read_bytes())),
    ):
        if index.get(manifest_name) != expected:
            raise SystemExit(f"latest manifest index checksum mismatch: {manifest_name}")

    matrix = (WORKTREE / "docs/refusal-evidence-matrix.md").read_text(encoding="utf-8")
    expected_terms = (
        "SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS",
        "CONTINUATION-229",
        "CONTINUATION-230",
        "CONTINUATION-231",
        "CONTINUATION-232",
        "ni runner Actions",
        "HMAC",
    )
    if not all(term in matrix for term in expected_terms):
        raise SystemExit("workflow refusal row is incomplete")

    diff_check = subprocess.run(
        ["git", "diff", "--check"],
        cwd=WORKTREE,
        capture_output=True,
        check=False,
        text=True,
    )
    if diff_check.returncode != 0:
        raise SystemExit(f"git diff --check failed: {diff_check.stderr}{diff_check.stdout}")
    print(
        json.dumps(
            {
                "reports_present": len(REPORT_IDS),
                "previous_local_manifest_entries_verified": len(manifest_path.read_text(encoding="utf-8").splitlines()),
                "previous_worktree_project_prefixes_preserved": True,
                "previous_latest_global_entries_match": True,
                "workflow_refusal_row_complete": True,
                "git_diff_check": "passed",
                "tests_rerun": False,
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
