#!/usr/bin/env python3
"""Validate a host-attested scope, then enter a private rootless user/net namespace."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import systemd_process_scope as process_scope  # noqa: E402


def write_private_json(path: Path, value: dict[str, object]) -> None:
    if path.is_symlink() or path.exists() or not path.parent.is_dir():
        raise RuntimeError("launcher receipt path must be a new file in an existing directory")
    fd = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
        0o600,
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8", closefd=False) as stream:
            json.dump(value, stream, sort_keys=True, separators=(",", ":"))
            stream.write("\n")
            stream.flush()
            os.fsync(fd)
    finally:
        os.close(fd)


def main() -> int:
    if len(sys.argv) < 5 or sys.argv[3] != "--":
        print("refusing malformed rootless namespace launch", file=sys.stderr)
        return 125
    host_network_namespace, receipt_path = sys.argv[1], Path(sys.argv[2])
    command = sys.argv[4:]
    if (
        re.fullmatch(r"net:\[[1-9][0-9]*\]", host_network_namespace) is None
        or not command
        or any("\x00" in part for part in command)
        or not receipt_path.is_absolute()
    ):
        print("refusing invalid rootless namespace launch fields", file=sys.stderr)
        return 125

    namespace = os.readlink("/proc/self/ns/net")
    lo_up = subprocess.run(
        ["/usr/sbin/ip", "link", "set", "dev", "lo", "up"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=5,
        check=False,
    )
    lo_state = subprocess.run(
        ["/usr/sbin/ip", "-o", "link", "show", "dev", "lo"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=5,
        check=False,
    )
    snapshot = process_scope.current_attested_process_scope_snapshot()
    receipt = {
        "schema_version": 1,
        "host_network_namespace": host_network_namespace,
        "private_network_namespace": namespace,
        "network_namespace_is_private": namespace != host_network_namespace,
        "euid_in_user_namespace": os.geteuid(),
        "uid_map": Path("/proc/self/uid_map").read_text(encoding="ascii").strip(),
        "cgroup": next(
            (
                line[3:]
                for line in Path("/proc/self/cgroup").read_text(encoding="ascii").splitlines()
                if line.startswith("0::/")
            ),
            None,
        ),
        "host_scope_attested": snapshot is not None,
        "host_scope_matches": (
            process_scope.process_scope_snapshot_matches(snapshot)
            if snapshot is not None
            else False
        ),
        "host_scope": (
            {
                "unit": snapshot.unit,
                "cgroup_path": snapshot.cgroup_path,
                "active_pids": list(snapshot.active_pids),
            }
            if snapshot is not None
            else None
        ),
        "loopback_up_command_exit": lo_up.returncode,
        "loopback_state": lo_state.stdout.decode("ascii", "replace").strip(),
        "loopback_state_error": lo_state.stderr.decode("ascii", "replace")[:512],
        "exec_started": False,
    }
    write_private_json(receipt_path, receipt)
    if (
        namespace == host_network_namespace
        or snapshot is None
        or not receipt["host_scope_matches"]
        or lo_up.returncode != 0
        or lo_state.returncode != 0
        or "UP" not in receipt["loopback_state"].split("<", 1)[-1].split(">", 1)[0].split(",")
    ):
        print("refusing: private namespace, attested outer scope, or loopback preflight failed", file=sys.stderr)
        return 125
    if not os.environ.get("PATH"):
        print("refusing empty command PATH", file=sys.stderr)
        return 125
    os.execvpe(command[0], command, os.environ.copy())
    return 125


if __name__ == "__main__":
    raise SystemExit(main())
