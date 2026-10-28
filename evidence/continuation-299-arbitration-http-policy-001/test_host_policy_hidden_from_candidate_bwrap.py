#!/usr/bin/env python3
"""Probe that the MISSION-012 candidate bind does not expose host .runtime."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path


def main() -> int:
    bwrap = Path("/usr/bin/bwrap")
    if not bwrap.is_file():
        print(json.dumps({"status": "blocked", "reason": "bubblewrap_missing"}))
        return 2

    with tempfile.TemporaryDirectory(prefix="mission-policy-bwrap-probe-") as temporary:
        host = Path(temporary) / "host-checkout"
        candidate = Path(temporary) / "candidate"
        policy_dir = host / ".runtime" / "mission-arbitration"
        policy_dir.mkdir(parents=True, mode=0o700)
        os.chmod(host, 0o700)
        os.chmod(policy_dir.parent, 0o700)
        os.chmod(policy_dir, 0o700)
        candidate.mkdir(mode=0o700)
        policy = policy_dir / "duration-parser.json"
        policy.write_text('{"authorized_reviewer_ids":[9001]}\n', encoding="utf-8")
        os.chmod(policy, 0o600)

        command = [
            str(bwrap),
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--assert-userns-disabled",
            "--die-with-parent",
            "--new-session",
            "--unshare-pid",
            "--dir",
            "/etc",
            "--ro-bind",
            "/etc/alternatives",
            "/etc/alternatives",
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/bin",
            "/bin",
            "--ro-bind",
            str(candidate),
            "/candidate",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--chdir",
            "/candidate",
            "--clearenv",
            "--",
            "/usr/bin/test",
            "!",
            "-e",
            str(policy),
        ]
        result = subprocess.run(command, check=False, capture_output=True, text=True, timeout=10)
        version = subprocess.run(
            [str(bwrap), "--version"], check=True, capture_output=True, text=True, timeout=5
        ).stdout.strip()
        record = {
            "status": "passed" if result.returncode == 0 else "failed",
            "bwrap": version,
            "host_policy_exists": policy.is_file(),
            "candidate_exit_code": result.returncode,
            "candidate_stderr": result.stderr.strip(),
            "probe": "sandbox mounts candidate_dir only and cannot stat the host policy path",
            "limit": "same-uid processes outside this namespace are not isolated by this bind layout",
        }
        print(json.dumps(record, sort_keys=True))
        return 0 if result.returncode == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
