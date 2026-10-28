#!/usr/bin/env python3
"""Capture focused CONTINUATION-280 checks and Cargo metadata readback."""
from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORKTREE = ROOT.parents[1]


def run(label: str, command: list[str]) -> int:
    result = subprocess.run(command, cwd=WORKTREE, capture_output=True, text=True)
    (ROOT / f"{label}.log").write_text(
        f"$ {' '.join(command)}\nexit_code={result.returncode}\n\n"
        f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}",
        encoding="utf-8",
    )
    return result.returncode


checks = [
    (
        "final-tests",
        [
            "cargo",
            "test",
            "--locked",
            "--offline",
            "--test",
            "webhook_reconcile_contract",
            "--test",
            "webhook_reconcile_pipeline",
        ],
    ),
    ("format-check", ["cargo", "fmt", "--all", "--", "--check"]),
    ("diff-check", ["git", "diff", "--check"]),
    ("license-guard", ["python3", "scripts/verify_cargo_license_state.py"]),
    (
        "license-policy-tests",
        ["python3", "-m", "unittest", "-v", "scripts.test_cargo_license_state"],
    ),
]
results = {label: run(label, command) for label, command in checks}

metadata_result = subprocess.run(
    [
        "cargo",
        "metadata",
        "--locked",
        "--offline",
        "--format-version",
        "1",
        "--no-deps",
    ],
    cwd=WORKTREE,
    capture_output=True,
    text=True,
)
(ROOT / "metadata-command.log").write_text(
    f"exit_code={metadata_result.returncode}\n{metadata_result.stderr}",
    encoding="utf-8",
)
results["cargo-metadata"] = metadata_result.returncode
if metadata_result.returncode == 0:
    cargo_metadata = json.loads(metadata_result.stdout)
    package = next(
        package
        for package in cargo_metadata["packages"]
        if package["name"] == "agent-forge-core"
    )
    licensing = package["metadata"]["agent-forge"]["licensing"]
    (ROOT / "metadata-readback.json").write_text(
        json.dumps(
            {
                "package": package["name"],
                "license": package["license"],
                "license_file": package["license_file"],
                "publish": package["publish"],
                "licensing_review": licensing,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

toolchain = {
    name: subprocess.run(
        command, cwd=WORKTREE, capture_output=True, text=True, check=True
    ).stdout.strip()
    for name, command in {
        "rustc": ["rustc", "--version"],
        "cargo": ["cargo", "--version"],
        "kernel": ["uname", "-sm"],
    }.items()
}
(ROOT / "toolchain.txt").write_text(
    "\n".join(f"{name}: {value}" for name, value in toolchain.items()) + "\n",
    encoding="utf-8",
)

print(json.dumps({"status": "captured", "exit_codes": results}, sort_keys=True))
if any(code != 0 for code in results.values()):
    raise SystemExit(1)
