#!/usr/bin/env python3
"""Capture bounded CONTINUATION-197 checks serially on the local host."""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(__file__).resolve().parent
COMMANDS: list[tuple[str, list[str]]] = [
    (
        "py_compile.log",
        [
            sys.executable,
            "-m",
            "py_compile",
            "scripts/systemd_process_scope.py",
            "scripts/systemd_scope_exec.py",
            "scripts/run_mission_012_live_composed_e2e.py",
            "scripts/run_mission_012_live_mcp_agent_e2e.py",
            "tests/test_mission_012_live_composed_e2e.py",
            "tests/test_mission_012_mcp_supervisor.py",
            "tests/test_mission_012_live_mcp_agent_e2e.py",
        ],
    ),
    (
        "composed-tests.log",
        [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_mission_012_live_composed_e2e.py", "-v"],
    ),
    (
        "supervisor-tests.log",
        [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_mission_012_mcp_supervisor.py", "-v"],
    ),
    (
        "agent-command-tests.log",
        [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_mission_012_live_mcp_agent_e2e.py", "-v"],
    ),
    ("diff-check.log", ["git", "diff", "--check"]),
    (
        "active-stage-scopes.log",
        ["systemctl", "--user", "list-units", "--state=active", "--type=scope", "--no-legend", "run-mission012-stage-*"],
    ),
]


def main() -> int:
    summary: list[dict[str, object]] = []
    failed = False
    for filename, command in COMMANDS:
        result = subprocess.run(
            command,
            cwd=ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
            timeout=120,
        )
        log = result.stdout
        (EVIDENCE / filename).write_bytes(log)
        item: dict[str, object] = {
            "file": filename,
            "argv": command,
            "exit_code": result.returncode,
            "output_bytes": len(log),
        }
        if filename == "active-stage-scopes.log":
            item["active_scope_count"] = len(log.splitlines())
            if result.returncode or log.strip():
                failed = True
        elif result.returncode:
            failed = True
        summary.append(item)
        print(json.dumps(item, sort_keys=True))
    (EVIDENCE / "check-summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
