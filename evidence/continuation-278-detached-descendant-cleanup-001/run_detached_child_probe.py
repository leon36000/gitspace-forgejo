#!/usr/bin/env python3
"""Observe host-side cleanup of a detached Bubblewrap test descendant."""
from __future__ import annotations

import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time

WORKTREE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(WORKTREE / "scripts"))
import mission_012_sandboxed_test_adapter as sandbox  # noqa: E402


def read_stat(pid: int) -> dict[str, int | str]:
    raw = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    close_paren = raw.rfind(")")
    if close_paren < 0:
        raise RuntimeError("process stat record is malformed")
    fields = raw[close_paren + 2 :].split()
    if len(fields) < 20:
        raise RuntimeError("process stat record is incomplete")
    return {
        "state": fields[0],
        "ppid": int(fields[1]),
        "pgrp": int(fields[2]),
        "session": int(fields[3]),
        "start_time": int(fields[19]),
    }


def host_process_tree(root_pid: int) -> tuple[dict[int, dict[str, int | str]], list[int]]:
    records: dict[int, dict[str, int | str]] = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        try:
            records[pid] = read_stat(pid)
        except (FileNotFoundError, ProcessLookupError, PermissionError, ValueError):
            continue
    descendants: list[int] = []
    frontier = [root_pid]
    while frontier:
        parent = frontier.pop()
        children = sorted(
            pid for pid, record in records.items()
            if pid != root_pid and record["ppid"] == parent
        )
        descendants.extend(children)
        frontier.extend(children)
    return records, descendants


def still_present(identities: dict[int, int]) -> list[int]:
    present: list[int] = []
    for pid, start_time in identities.items():
        try:
            current = read_stat(pid)
        except (FileNotFoundError, ProcessLookupError, PermissionError, ValueError):
            continue
        if current["start_time"] == start_time:
            present.append(pid)
    return present


def kill_observed_process(pid: int, start_time: int) -> None:
    pidfd_open = getattr(os, "pidfd_open", None)
    pidfd_send_signal = getattr(signal, "pidfd_send_signal", None)
    if not callable(pidfd_open) or not callable(pidfd_send_signal):
        return
    try:
        pidfd = pidfd_open(pid, 0)
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return
    try:
        if read_stat(pid)["start_time"] == start_time:
            pidfd_send_signal(pidfd, signal.SIGKILL)
    except (FileNotFoundError, ProcessLookupError, PermissionError, ValueError):
        pass
    finally:
        os.close(pidfd)


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="continuation-278-detached-child-") as directory:
        command = sandbox.bubblewrap_command(
            Path(directory) / "synthetic-credential.txt",
            49152,
            probe_only=True,
        )
        python_index = command.index(str(sandbox.PYTHON))
        program = (
            "import os,time; child=os.fork(); "
            "(os.setsid() if child == 0 else None); "
            "print('ready', flush=True); time.sleep(120)"
        )
        command = command[:python_index + 1] + ["-c", program]
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=sandbox.HOST_ENV,
            bufsize=0,
            close_fds=True,
            start_new_session=True,
        )
        identities: dict[int, int] = {}
        try:
            assert process.stdout is not None and process.stderr is not None
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                if not selector.select(10):
                    raise RuntimeError("detached sandbox child did not become ready")
            if process.stdout.readline(4096) != b"ready\n":
                raise RuntimeError("detached sandbox child emitted an unexpected ready event")

            records, descendants = host_process_tree(process.pid)
            if len(descendants) < 3:
                raise RuntimeError(f"expected at least three sandbox descendants, observed {descendants}")
            detached = [
                pid for pid in descendants
                if records[pid]["ppid"] in records
                and records[pid]["ppid"] != process.pid
                and records[pid]["session"] == pid
                and records[pid]["session"] != records[int(records[pid]["ppid"])]["session"]
            ]
            if len(detached) != 1 or records[detached[0]]["pgrp"] == process.pid:
                raise RuntimeError("the test child did not escape the launch process group")
            identities = {pid: int(records[pid]["start_time"]) for pid in descendants}
            before = {
                str(pid): records[pid]
                for pid in descendants
            }

            sandbox.stop_process_group(process)
            deadline = time.monotonic() + 3
            survivors = still_present(identities)
            while survivors and time.monotonic() < deadline:
                time.sleep(0.02)
                survivors = still_present(identities)
            stderr = process.stderr.read(64 * 1024)
            if process.returncode is None or survivors or stderr:
                raise RuntimeError(
                    f"cleanup inconclusive: returncode={process.returncode}, "
                    f"survivors={survivors}, stderr_nonempty={bool(stderr)}"
                )
            print(json.dumps({
                "probe": "detached_bubblewrap_descendant_cleanup",
                "source": "host_procfs_and_process_group_cleanup",
                "root_pid": process.pid,
                "root_returncode": process.returncode,
                "termination_signal": "SIGTERM",
                "descendants_before": before,
                "detached_descendant_pid": detached[0],
                "detached_descendant_session_differs_from_parent": True,
                "observed_pid_start_times_remaining": survivors,
                "stderr_empty": True,
                "cleanup_wait_seconds_max": 3,
            }, sort_keys=True))
            return 0
        finally:
            sandbox.stop_process_group(process)
            for pid, start_time in identities.items():
                kill_observed_process(pid, start_time)
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream is not None:
                    stream.close()


if __name__ == "__main__":
    raise SystemExit(main())
