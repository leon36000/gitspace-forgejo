#!/usr/bin/env python3
"""Local-only canary for bwrap network and host /tmp isolation primitives."""

from __future__ import annotations

import os
from pathlib import Path
import socket
import subprocess
import tempfile


def main() -> None:
    bwrap = Path("/usr/bin/bwrap")
    python = Path("/usr/bin/python3")
    if not bwrap.is_file() or not python.is_file():
        raise SystemExit("required local bwrap/Python binary is absent")

    marker_fd, marker_name = tempfile.mkstemp(prefix="agent-forge-canary-")
    marker = Path(marker_name)
    try:
        os.write(marker_fd, b"synthetic-canary-only\n")
        os.close(marker_fd)

        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", 0))
        listener.listen(2)
        listener.settimeout(2)
        port = listener.getsockname()[1]

        # Establish that the host-side fixture is reachable before the sandbox probe.
        with socket.create_connection(("127.0.0.1", port), timeout=2):
            accepted, _ = listener.accept()
            accepted.close()

        inner = r"""
import pathlib, socket, sys
port, marker = int(sys.argv[1]), pathlib.Path(sys.argv[2])
try:
    socket.create_connection(("127.0.0.1", port), timeout=1).close()
except OSError:
    print("sandbox_network=blocked")
else:
    print("sandbox_network=connected")
    raise SystemExit(11)
if marker.exists():
    print("host_tmp_canary=visible")
    raise SystemExit(12)
print("host_tmp_canary=hidden")
"""
        command = [
            str(bwrap),
            "--die-with-parent",
            "--unshare-net",
            "--unshare-pid",
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
            str(python),
            "-c",
            inner,
            str(port),
            str(marker),
        ]
        result = subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=10,
            check=False,
            text=True,
        )
        if result.returncode != 0:
            raise SystemExit(f"sandbox probe failed with exit {result.returncode}: {result.stdout.strip()}")
        lines = result.stdout.splitlines()
        if lines != ["sandbox_network=blocked", "host_tmp_canary=hidden"]:
            raise SystemExit(f"unexpected sandbox probe output: {result.stdout.strip()}")

        listener.setblocking(False)
        try:
            extra, _ = listener.accept()
        except BlockingIOError:
            pass
        else:
            extra.close()
            raise SystemExit("sandbox process connected to the host loopback listener")
        finally:
            listener.close()

        print("host_fixture=reachable")
        print("sandbox_network=blocked")
        print("host_tmp_canary=hidden")
        print("external_network_used=false")
    finally:
        try:
            os.close(marker_fd)
        except OSError:
            pass
        marker.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
