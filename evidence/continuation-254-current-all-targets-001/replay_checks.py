#!/usr/bin/env python3
"""Replay the full locked, offline Rust target suite and retain its output."""
from __future__ import annotations

import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]
COMMAND = ["cargo", "test", "--offline", "--locked", "--all-targets"]


def run(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, check=False, timeout=900)


rustc = run(["rustc", "--version"], WORKTREE)
cargo = run(["cargo", "--version"], WORKTREE)
if rustc.returncode or cargo.returncode:
    raise SystemExit("unable to record Rust tool versions")

result = run(COMMAND, WORKTREE)
(ROOT / "all-targets.log").write_text(result.stdout, encoding="utf-8")
summaries = [
    {"passed": int(passed), "failed": int(failed), "ignored": int(ignored),
     "measured": int(measured)}
    for passed, failed, ignored, measured in re.findall(
        r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured;",
        result.stdout,
    )
]
if not summaries and result.returncode == 0:
    # Cargo can omit the test summary for targets containing zero tests.
    summaries = []
summary = {
    "command": COMMAND,
    "exit_code": result.returncode,
    "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=WORKTREE, text=True).strip(),
    "rustc": rustc.stdout.strip(),
    "cargo": cargo.stdout.strip(),
    "target_summaries": summaries,
    "totals": {key: sum(row[key] for row in summaries)
               for key in ("passed", "failed", "ignored", "measured")},
    "completed_at_utc": datetime.now(timezone.utc).isoformat(),
}
(ROOT / "checks-summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n",
                                         encoding="utf-8")
print(json.dumps(summary, sort_keys=True))
if result.returncode:
    raise SystemExit(result.returncode)
