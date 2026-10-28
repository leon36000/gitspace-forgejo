#!/usr/bin/env python3
"""Check whether explicitly inherited synthetic FDs cross a bwrap boundary."""

from __future__ import annotations

import os
from pathlib import Path
import socket
import subprocess


BWRAP = Path("/usr/bin/bwrap")
PYTHON = Path("/usr/bin/python3")
SYNTHETIC_CREDENTIAL = b"SYNTHETIC-CREDENTIAL-CANARY-ONLY"
SOCKET_CANARY = b"INHERITED-SOCKET-CANARY"


def sandbox_command(fd: int, probe: str) -> list[str]:
    return [
        str(BWRAP),
        "--die-with-parent",
        "--unshare-net",
        "--unshare-pid",
        "--new-session",
        "--ro-bind",
        "/usr",
        "/usr",
        "--dir",
        "/etc",
        "--ro-bind",
        "/etc/alternatives",
        "/etc/alternatives",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/bin",
        "/bin",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/home",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/tmp",
        str(PYTHON),
        "-c",
        probe,
        str(fd),
    ]


def run_probe(fd: int, probe: str, *, pass_fd: bool) -> str:
    options: dict[str, object] = {
        "stdin": subprocess.DEVNULL,
        "stdout": subprocess.PIPE,
        "stderr": subprocess.STDOUT,
        "timeout": 10,
        "check": False,
        "text": True,
        "close_fds": True,
    }
    if pass_fd:
        options["pass_fds"] = (fd,)
    result = subprocess.run(sandbox_command(fd, probe), **options)
    if result.returncode != 0:
        raise RuntimeError(
            f"bwrap probe returned {result.returncode}: {result.stdout.strip()}"
        )
    return result.stdout.strip()


def main() -> None:
    if not BWRAP.is_file() or not PYTHON.is_file():
        raise SystemExit("required local bwrap/Python executable is absent")

    credential_fd = os.memfd_create("synthetic-credential-canary")
    try:
        os.write(credential_fd, SYNTHETIC_CREDENTIAL)
        os.lseek(credential_fd, 0, os.SEEK_SET)
        os.set_inheritable(credential_fd, True)
        credential_probe = r"""
import os, sys
fd = int(sys.argv[1])
try:
    value = os.read(fd, 128)
except OSError:
    print("credential_fd=unavailable")
else:
    print("credential_fd=visible" if value == b"SYNTHETIC-CREDENTIAL-CANARY-ONLY" else "credential_fd=other")
"""
        explicit_credential = run_probe(
            credential_fd, credential_probe, pass_fd=True
        )
        if explicit_credential != "credential_fd=visible":
            raise RuntimeError(f"explicit credential FD probe was unexpected: {explicit_credential}")

        closed_credential = run_probe(
            credential_fd, credential_probe, pass_fd=False
        )
        if closed_credential == "credential_fd=visible":
            raise RuntimeError("credential FD remained visible with close_fds=True")
    finally:
        os.close(credential_fd)

    socket_probe = r"""
import os, sys
fd = int(sys.argv[1])
try:
    os.write(fd, b"INHERITED-SOCKET-CANARY")
except OSError:
    print("socket_fd=unavailable")
else:
    print("socket_fd=write-succeeded")
"""
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    listener.settimeout(2)
    client = socket.create_connection(listener.getsockname(), timeout=2)
    accepted, _ = listener.accept()
    accepted.settimeout(0.25)
    try:
        os.set_inheritable(client.fileno(), True)
        explicit_socket = run_probe(client.fileno(), socket_probe, pass_fd=True)
        if explicit_socket != "socket_fd=write-succeeded":
            raise RuntimeError(f"explicit socket FD probe was unexpected: {explicit_socket}")
        received = accepted.recv(128)
        if received != SOCKET_CANARY:
            raise RuntimeError(f"listener did not receive explicit socket canary: {received!r}")

        closed_socket = run_probe(client.fileno(), socket_probe, pass_fd=False)
        if closed_socket == "socket_fd=write-succeeded":
            # A reused unrelated descriptor may be writable; delivery is the
            # security assertion, checked below against the host listener.
            pass
        accepted.settimeout(0.25)
        try:
            unexpected = accepted.recv(128)
        except TimeoutError:
            unexpected = b""
        if unexpected:
            raise RuntimeError(f"unpassed socket FD delivered bytes: {unexpected!r}")
    finally:
        accepted.close()
        client.close()
        listener.close()

    print("explicit_fd_credential=visible")
    print("close_fds_credential=not_visible")
    print("explicit_fd_socket=reachable_across_unshare_net")
    print("close_fds_socket=not_delivered")
    print("external_network_used=false")


if __name__ == "__main__":
    main()
