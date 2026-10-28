from __future__ import annotations

import os
from pathlib import Path
import runpy
import subprocess
import time
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SUPERVISOR = runpy.run_path(str(ROOT / "scripts/mission_012_mcp_supervisor.py"))
UID = os.getuid()
UNIT = f"run-codex-cgroup-fallback-{os.getpid()}-{time.time_ns()}"
SCOPE = f"/user.slice/user-{UID}.slice/user@{UID}.service/app.slice/{UNIT}.scope"

process = subprocess.Popen(
    [
        "/usr/bin/systemd-run",
        "--user",
        "--scope",
        "--collect",
        f"--unit={UNIT}",
        "/usr/bin/sleep",
        "60",
    ],
    stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.PIPE,
)

try:
    deadline = time.monotonic() + 8
    while SCOPE not in SUPERVISOR["verifier_scope_paths"](process.pid):
        if process.poll() is not None:
            raise RuntimeError(f"scope launcher exited early: {process.returncode}")
        if time.monotonic() >= deadline:
            raise TimeoutError("could not observe the self-created verifier scope")
        time.sleep(0.02)

    original_open = os.open

    def deny_cgroup_kill(path: str | os.PathLike[str], flags: int, *args: object, **kwargs: object) -> int:
        if os.fspath(path) == f"/sys/fs/cgroup{SCOPE}/cgroup.kill":
            raise PermissionError("injected cgroup.kill denial")
        return original_open(path, flags, *args, **kwargs)

    with mock.patch.object(SUPERVISOR["os"], "open", side_effect=deny_cgroup_kill):
        if not SUPERVISOR["kill_verifier_scope"](SCOPE):
            raise RuntimeError("systemd user-manager fallback reported failure")

    return_code = process.wait(timeout=8)
    if return_code == 0:
        raise RuntimeError("sleep unexpectedly completed successfully after SIGKILL")
    print(
        "PASS: cgroup.kill denial fell back to the exact user scope; "
        f"unit={UNIT}; launcher_exit={return_code}"
    )
finally:
    if process.poll() is None:
        subprocess.run(
            [
                "/usr/bin/systemctl",
                "--user",
                "kill",
                "--kill-whom=all",
                "--signal=SIGKILL",
                f"{UNIT}.scope",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=3,
        )
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=2)
