#!/usr/bin/env python3
"""Attest the private compose namespace inside a host-created outer scope."""

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


def write_receipt(path: Path, value: dict[str, object]) -> None:
    fd = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
        0o600,
    )
    with os.fdopen(fd, "wb") as stream:
        stream.write(json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8") + b"\n")
        stream.flush()
        os.fsync(stream.fileno())


def main() -> int:
    if len(sys.argv) < 5 or sys.argv[3] != "--":
        print("refusing malformed Bwrap composition entry", file=sys.stderr)
        return 125
    host_network_namespace, receipt_text = sys.argv[1], sys.argv[2]
    command = sys.argv[4:]
    receipt_path = Path(receipt_text)
    current_namespace = os.readlink("/proc/self/ns/net")
    if (
        re.fullmatch(r"net:\[[1-9][0-9]*\]", host_network_namespace) is None
        or current_namespace == host_network_namespace
        or os.geteuid() == 0
        or not receipt_path.is_absolute()
        or not command
        or any("\x00" in part for part in command)
    ):
        print("refusing invalid or insufficiently isolated Bwrap composition entry", file=sys.stderr)
        return 125

    snapshot = process_scope.current_attested_process_scope_snapshot()
    loopback = subprocess.run(
        ["/usr/sbin/ip", "-o", "link", "show", "dev", "lo"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=5,
        check=False,
    )
    loopback_state = loopback.stdout.decode("ascii", "replace").strip()
    flags = loopback_state.split("<", 1)[-1].split(">", 1)[0].split(",")
    receipt = {
        "schema_version": 1,
        "host_network_namespace": host_network_namespace,
        "compose_network_namespace": current_namespace,
        "compose_namespace_differs_from_host": current_namespace != host_network_namespace,
        "compose_euid": os.geteuid(),
        "compose_uid_map": Path("/proc/self/uid_map").read_text(encoding="ascii").strip(),
        "compose_user_namespace": os.readlink("/proc/self/ns/user"),
        "compose_mount_namespace": os.readlink("/proc/self/ns/mnt"),
        "outer_scope_attested": snapshot is not None,
        "outer_scope_matches": (
            process_scope.process_scope_snapshot_matches(snapshot)
            if snapshot is not None
            else False
        ),
        "outer_scope": (
            {
                "unit": snapshot.unit,
                "cgroup_path": snapshot.cgroup_path,
                "active_pids": list(snapshot.active_pids),
            }
            if snapshot is not None
            else None
        ),
        "loopback_is_up": loopback.returncode == 0 and "UP" in flags,
        "loopback_state": loopback_state,
        "exec_started": False,
    }
    if receipt_path.exists() or receipt_path.is_symlink():
        print("refusing to overwrite Bwrap launcher receipt", file=sys.stderr)
        return 125
    write_receipt(receipt_path, receipt)
    if (
        snapshot is None
        or not receipt["outer_scope_matches"]
        or not receipt["loopback_is_up"]
    ):
        print("refusing: host scope or private-loopback preflight is not attested", file=sys.stderr)
        return 125
    os.execvpe(command[0], command, os.environ.copy())
    return 125


if __name__ == "__main__":
    raise SystemExit(main())
