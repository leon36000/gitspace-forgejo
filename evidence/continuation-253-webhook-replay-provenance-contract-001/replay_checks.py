#!/usr/bin/env python3
"""Rerun CONT-253's bounded offline checks and save exact outputs."""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
CHECKS = (
    ("rust-reconcile-unit", ["cargo", "test", "--offline", "--locked", "--bin", "forgejo-reconcile-local"]),
    ("capabilities-contract", ["cargo", "test", "--offline", "--locked", "--test", "forgejo_reconcile_capabilities_e2e"]),
    ("single-batch-process-contract", ["cargo", "test", "--offline", "--locked", "--test", "reconcile_config_contract"]),
    ("quarantine-cli-versions", ["cargo", "test", "--offline", "--locked", "--test", "webhook_quarantine_cli"]),
    ("journal-lock-cli-version", ["cargo", "test", "--offline", "--locked", "--test", "reconcile_journal_lock_timeout"]),
    ("bubblewrap-batch-readback", [sys.executable, "scripts/run_webhook_reconcile_cli_e2e.py", "--scenario", "actual_reconcile_cli_process_shares_recovered_parent_issue_read_with_comment_batch"]),
    ("python-emitted-consumers", [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_forgejo_emitted_webhook_e2e.py"]),
    ("python-org-review-consumers", [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_forgejo_live_org_team_reviewer_readback.py"]),
    ("python-native-replay-consumers", [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_forgejo_webhook_native_replay_e2e.py"]),
    ("python-runner-syntax", [sys.executable, "-m", "py_compile", "scripts/run_forgejo_native_branch_delete_webhook_reconcile.py", "scripts/run_forgejo_live_webhook_reconcile_emitted.py", "scripts/run_forgejo_live_org_team_reviewer_readback.py", "scripts/run_forgejo_native_push_webhook_reconcile.py", "scripts/run_forgejo_live_webhook_native_replay_e2e.py"]),
    ("rustfmt", ["cargo", "fmt", "--all", "--", "--check"]),
    ("diff-check", ["git", "diff", "--check"]),
    ("cargo-license-policy", [sys.executable, "scripts/verify_cargo_license_state.py"]),
)

summary = []
with (EVIDENCE / "commands.txt").open("w", encoding="utf-8") as commands:
    for name, argv in CHECKS:
        rendered = " ".join(argv)
        commands.write(f"{name}: {rendered}\n")
        process = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True, check=False)
        log = (process.stdout or "")
        if process.stderr:
            log += "\n--- STDERR ---\n" + process.stderr
        (EVIDENCE / f"{name}.log").write_text(log, encoding="utf-8")
        summary.append({"name": name, "argv": argv, "exit_code": process.returncode})
        print(f"{name}: exit {process.returncode}")
        if process.returncode:
            break

(EVIDENCE / "checks-summary.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
if any(item["exit_code"] != 0 for item in summary):
    raise SystemExit(1)
