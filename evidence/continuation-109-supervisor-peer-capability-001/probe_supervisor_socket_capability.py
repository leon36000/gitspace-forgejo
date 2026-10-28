#!/usr/bin/env python3
"""Probe whether an inherited Unix socket can identify its supervisor across Bwrap."""
from __future__ import annotations

import json
import os
import socket
import struct
import subprocess


CHILD = r'''import json, os, socket, struct, sys
channel = socket.socket(fileno=int(sys.argv[1]))
peer_pid, peer_uid, peer_gid = struct.unpack(
    "3i", channel.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
)
hello = {
    "phase": "hello",
    "peer_pid": peer_pid,
    "peer_uid": peer_uid,
    "peer_gid": peer_gid,
    "self_pid": os.getpid(),
    "self_ppid": os.getppid(),
    "self_netns": os.readlink("/proc/self/ns/net"),
    "self_pidns": os.readlink("/proc/self/ns/pid"),
    "fd_target": os.readlink(f"/proc/self/fd/{channel.fileno()}"),
}
channel.send(json.dumps(hello, sort_keys=True).encode())
grant = channel.recv(128)
if grant != b"permit-once":
    raise SystemExit(4)
channel.send(json.dumps({"phase": "done", "self_pid": os.getpid()}).encode())
'''


def run(*, unshare_pid: bool) -> dict[str, object]:
    supervisor, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    child.set_inheritable(True)
    command = [
        "/usr/bin/bwrap",
        "--unshare-net",
        "--unshare-user",
        "--die-with-parent",
        "--new-session",
    ]
    if unshare_pid:
        command.append("--unshare-pid")
    command.extend(
        [
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
            "--setenv",
            "HOME",
            "/nonexistent",
            "--",
            "/usr/bin/python3",
            "-c",
            CHILD,
            str(child.fileno()),
        ]
    )
    process = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        pass_fds=(child.fileno(),),
        close_fds=True,
    )
    child.close()
    try:
        supervisor.settimeout(5)
        hello = json.loads(supervisor.recv(4096))
        if hello.get("phase") != "hello":
            raise RuntimeError("child did not send a hello record")
        host_netns = os.readlink("/proc/self/ns/net")
        host_pidns = os.readlink("/proc/self/ns/pid")
        child_host_visible_netns: str | None = None
        if not unshare_pid:
            child_host_visible_netns = os.readlink(f"/proc/{hello['self_pid']}/ns/net")
        if process.poll() is not None:
            raise RuntimeError("sandbox child exited before supervisor observation")
        if hello.get("peer_uid") != os.getuid():
            raise RuntimeError("socket peer UID did not match the host supervisor")
        if not str(hello.get("fd_target", "")).startswith("socket:["):
            raise RuntimeError("inherited control descriptor is not a socket")
        if hello.get("self_netns") == host_netns:
            raise RuntimeError("child did not enter a distinct network namespace")
        if unshare_pid:
            if hello.get("peer_pid") != 0 or hello.get("self_pidns") == host_pidns:
                raise RuntimeError("PID namespace probe did not hide the host peer identity")
        else:
            if hello.get("peer_pid") != os.getpid():
                raise RuntimeError("SO_PEERCRED did not identify the host socket creator")
            if hello.get("self_ppid") != process.pid:
                raise RuntimeError("sandbox process parent is not the Bubblewrap launcher")
            if child_host_visible_netns != hello.get("self_netns"):
                raise RuntimeError("host /proc did not corroborate the child's network namespace")
        supervisor.send(b"permit-once")
        done = json.loads(supervisor.recv(4096))
        if done != {"phase": "done", "self_pid": hello["self_pid"]}:
            raise RuntimeError("child did not confirm the one-shot packet")
        returncode = process.wait(timeout=5)
        stderr = process.stderr.read().decode("utf-8", "replace") if process.stderr else ""
        if returncode != 0 or stderr:
            raise RuntimeError(f"probe child failed: exit={returncode}, stderr={stderr!r}")
        return {
            "pid_namespace_unshared": unshare_pid,
            "host_supervisor_pid": os.getpid(),
            "bwrap_parent_pid": process.pid,
            "host_network_namespace": host_netns,
            "host_pid_namespace": host_pidns,
            "child_host_visible_network_namespace": child_host_visible_netns,
            "child": hello,
            "one_shot_packet_confirmed": True,
            "child_exit_code": returncode,
        }
    finally:
        supervisor.close()
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)


def main() -> int:
    python_version = subprocess.run(
        ["/usr/bin/python3", "--version"],
        check=True,
        capture_output=True,
        text=True,
    )
    print(
        json.dumps(
            {
                "schema_version": 1,
                "bubblewrap_version": subprocess.run(
                    ["/usr/bin/bwrap", "--version"],
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout.strip(),
                "python_version": python_version.stdout.strip() or python_version.stderr.strip(),
                "runs": [run(unshare_pid=False), run(unshare_pid=True)],
                "verdict": "passed",
            },
            sort_keys=True,
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
