#!/usr/bin/env python3
"""Reproduce the held Bubblewrap process-tree observation from CONTINUATION-277."""
from __future__ import annotations

import json
import selectors
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORKTREE = HERE.parents[1]
sys.path.insert(0, str(WORKTREE / "scripts"))
import mission_012_sandboxed_test_adapter as sandbox  # noqa: E402


def read_event(stream, timeout: float = 10) -> dict[str, object]:
    with selectors.DefaultSelector() as selector:
        selector.register(stream, selectors.EVENT_READ)
        if not selector.select(timeout):
            raise RuntimeError("sandbox probe event timed out")
    line = stream.readline(64 * 1024)
    if not line or len(line) >= 64 * 1024:
        raise RuntimeError("sandbox probe event is missing or oversized")
    value = json.loads(line)
    if not isinstance(value, dict):
        raise RuntimeError("sandbox probe event is not an object")
    return value


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="continuation-277-probe-") as directory:
        command = sandbox.bubblewrap_command(
            Path(directory) / "synthetic-credential.txt",
            49152,
            probe_only=True,
            probe_hold_open=True,
        )
        process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=sandbox.HOST_ENV,
            bufsize=0,
            close_fds=True,
            start_new_session=True,
        )
        try:
            assert process.stdin is not None
            assert process.stdout is not None
            assert process.stderr is not None
            first = read_event(process.stdout)
            if first.get("type") != "sandbox_egress_probe_ready":
                raise RuntimeError("sandbox probe did not remain held at the gate")
            first_observation = sandbox.supervisor_observe_sandbox_process_tree(process.pid)

            process.stdin.write(b"reprobe\n")
            process.stdin.flush()
            second = read_event(process.stdout)
            if second.get("type") != "sandbox_egress_probe_after_gate":
                raise RuntimeError("sandbox probe did not complete the gate reprobe")
            second_observation = sandbox.supervisor_observe_sandbox_process_tree(process.pid)

            process.stdin.write(b"release\n")
            process.stdin.flush()
            released = read_event(process.stdout)
            status = process.wait(timeout=5)
            stderr = process.stderr.read(64 * 1024)
            if (
                released.get("type") != "sandbox_egress_probe_released"
                or first_observation != second_observation
                or status != 0
                or stderr
            ):
                raise RuntimeError("sandbox process observation or cleanup was inconclusive")
            print(
                json.dumps(
                    {
                        "probe_type": first["type"],
                        "agent_reported_network_namespace": first.get("network_namespace"),
                        "supervisor_observation": first_observation,
                        "second_probe_type": second["type"],
                        "supervisor_observation_repeated": True,
                        "release_event": released["type"],
                        "exit_code": status,
                        "stderr_empty": True,
                    },
                    sort_keys=True,
                )
            )
            return 0
        finally:
            sandbox.stop_process_group(process)
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream is not None:
                    stream.close()


if __name__ == "__main__":
    raise SystemExit(main())
