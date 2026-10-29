#!/usr/bin/env python3
"""Capture bounded local verification commands for CONTINUATION-438."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys


RUN = Path(__file__).resolve().parent
ROOT = RUN.parents[1]
COMMANDS: tuple[tuple[str, tuple[str, ...]], ...] = (
    (
        "reconcile-contract",
        ("cargo", "test", "--locked", "--offline", "--test", "webhook_reconcile_contract", "--", "--test-threads=1"),
    ),
    (
        "repository-route-policy",
        (
            "cargo",
            "test",
            "--locked",
            "--offline",
            "--bin",
            "forgejo-reconcile-local",
            "reconciliation_policy_allows_only_exact_configured_repository_visibility_read",
            "--",
            "--nocapture",
        ),
    ),
    (
        "process-target-default",
        ("cargo", "test", "--locked", "--offline", "--test", "webhook_receiver_process_e2e", "--", "--test-threads=1"),
    ),
    (
        "process-isolated-scenarios",
        (
            "python3",
            "scripts/run_webhook_reconcile_cli_e2e.py",
            "--scenario",
            "actual_reconcile_cli_process_confirms_native_delete_404_after_repository_and_fresh_reads",
            "--scenario",
            "actual_reconcile_cli_process_refuses_branch_absence_when_repository_identity_mismatches",
            "--scenario",
            "actual_reconcile_cli_process_keeps_create_inconclusive_and_delete_absent_in_batch",
        ),
    ),
    (
        "python-validator-suite",
        ("python3", "tests/test_forgejo_emitted_webhook_e2e.py"),
    ),
    ("all-targets-check", ("cargo", "check", "--locked", "--offline", "--all-targets")),
    ("rustfmt", ("cargo", "fmt", "--all", "--", "--check")),
    ("diff-check", ("git", "diff", "--check")),
    ("cargo-license-policy", ("python3", "scripts/verify_cargo_license_state.py")),
    (
        "canonical-doc-links",
        ("python3", "evidence/documentation-link-audit-001/verify_doc_links.py"),
    ),
)


def main() -> int:
    results: list[dict[str, object]] = []
    for label, argv in COMMANDS:
        completed = subprocess.run(
            argv,
            cwd=ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=900,
            check=False,
        )
        (RUN / f"{label}-attempt-004.log").write_bytes(completed.stdout)
        (RUN / f"{label}-attempt-004.exit-code").write_text(
            f"{completed.returncode}\n", encoding="ascii"
        )
        results.append(
            {
                "label": label,
                "argv": list(argv),
                "exit_code": completed.returncode,
                "output_bytes": len(completed.stdout),
            }
        )
        print(json.dumps(results[-1], ensure_ascii=False, sort_keys=True), flush=True)
    (RUN / "verification-results-attempt-004.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0 if all(result["exit_code"] == 0 for result in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
