#!/usr/bin/env python3
"""Run the composed receiver-to-reconciliation scenario in the private netns runner."""

from __future__ import annotations

import runpy
from pathlib import Path
import sys
import tempfile

WORKTREE = Path(__file__).resolve().parents[2]
RUNNER = WORKTREE / "scripts/run_webhook_reconcile_cli_e2e.py"
TEST_NAME = (
    "actual_receiver_refuses_issue_comment_marked_non_pull_with_pr_parent_then_"
    "accepts_same_delivery"
)

real_temporary_directory = tempfile.TemporaryDirectory


def tmpfs_temporary_directory(*args, **kwargs):
    # Host-side staging is bounded; keep it in tmpfs because / is nearly full.
    if kwargs.get("dir") == "/tmp":
        kwargs["dir"] = "/dev/shm"
    return real_temporary_directory(*args, **kwargs)


tempfile.TemporaryDirectory = tmpfs_temporary_directory
sys.path.insert(0, str(RUNNER.parent))
sys.argv = [str(RUNNER), "--scenario", TEST_NAME]
runpy.run_path(str(RUNNER), run_name="__main__")
