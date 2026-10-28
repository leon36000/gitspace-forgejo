"""Bounded cleanup for a stage's private POSIX process group."""

from __future__ import annotations

import os
from pathlib import Path
import signal
import subprocess
import time


def child_exit_is_waitable(process: subprocess.Popen[bytes]) -> bool:
    """Observe leader exit without reaping its PID before checking its group."""
    try:
        result = os.waitid(
            os.P_PID,
            process.pid,
            os.WEXITED | os.WNOHANG | os.WNOWAIT,
        )
    except ChildProcessError:
        return process.returncode is not None
    return result is not None and result.si_pid == process.pid


def live_process_group_members(process_group: int, leader_pid: int) -> list[int] | None:
    """List active group members excluding the unreaped leader; None means unknown."""
    try:
        entries = list(Path("/proc").iterdir())
    except OSError:
        return None
    members: list[int] = []
    for entry in entries:
        if not entry.name.isdecimal():
            continue
        pid = int(entry.name)
        if pid == leader_pid:
            continue
        try:
            stat_line = (entry / "stat").read_bytes()
        except FileNotFoundError:
            continue
        except OSError:
            return None
        try:
            fields = stat_line.rsplit(b")", 1)[1].split()
            state = fields[0]
            observed_group = int(fields[2])
        except (IndexError, ValueError):
            return None
        if observed_group == process_group and state not in {b"Z", b"X"}:
            members.append(pid)
    return members


def finish_exited_process_group(
    process: subprocess.Popen[bytes], *, grace_seconds: float
) -> str:
    """Reap an exited leader only after its still-active group children are stopped."""
    if not child_exit_is_waitable(process):
        return "leader_running"
    members = live_process_group_members(process.pid, process.pid)
    if members is None:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        return "unconfirmed"
    if not members:
        process.wait()
        return "clean"

    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + max(0.0, grace_seconds)
    while time.monotonic() < deadline:
        remaining = live_process_group_members(process.pid, process.pid)
        if remaining is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            return "unconfirmed"
        if not remaining:
            process.wait()
            return "descendants_terminated"
        time.sleep(0.02)

    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + 1
    while time.monotonic() < deadline:
        remaining = live_process_group_members(process.pid, process.pid)
        if remaining is None:
            process.wait()
            return "unconfirmed"
        if not remaining:
            process.wait()
            return "descendants_terminated"
        time.sleep(0.02)
    process.wait()
    return "unconfirmed"
