#!/usr/bin/env python3
"""Relay MCP stdio and gracefully finalize the trusted host on client exit.

Claude Code may terminate a stdio MCP child after a print-mode turn instead
of closing its stdin first. The host uses a separate process group so relay
cancellation cannot kill it before finalization. This supervisor translates
EOF or SIGTERM into a clean stdin EOF, waits for host finalization, and kills
the host itself if its bounded shutdown expires.
"""

from __future__ import annotations

import os
import select
import signal
import stat
import subprocess
import sys
import time

MAX_FRAME_BYTES = 128 * 1024 + 1
HOST_FINALIZATION_TIMEOUT_SECONDS = 15
HOST_GROUP_TERMINATION_GRACE_SECONDS = 1
VERIFIER_SCOPE_KILL_TIMEOUT_SECONDS = 2
VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS = 76
HOST_ENV = {
    "PATH": "/usr/bin:/bin",
    "HOME": "/nonexistent",
    "LANG": "C.UTF-8",
    "LC_ALL": "C.UTF-8",
}


def terminate_host_group(host: subprocess.Popen[bytes]) -> bool:
    """Terminate the host and report whether every verifier kill request was accepted."""
    # The verifier is launched in a separate process group and bubblewrap starts
    # a new session. If the host misses its finalization deadline, killing only
    # the host group can strand that verifier. Before terminating the host,
    # kill only transient verifier scopes reachable from its process tree.
    verifier_kill_requests_accepted = True
    for scope_path in verifier_scope_paths(host.pid):
        if not kill_verifier_scope(scope_path):
            verifier_kill_requests_accepted = False

    try:
        os.killpg(host.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        host.wait(timeout=HOST_GROUP_TERMINATION_GRACE_SECONDS)
    except subprocess.TimeoutExpired:
        pass
    deadline = time.monotonic() + HOST_GROUP_TERMINATION_GRACE_SECONDS
    while time.monotonic() < deadline:
        try:
            os.killpg(host.pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.025)
    try:
        os.killpg(host.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    if host.poll() is None:
        host.wait()
    return verifier_kill_requests_accepted


def verifier_scope_unit(scope_path: str) -> str | None:
    """Return a scope unit only when it belongs to this user's app.slice."""
    uid = os.getuid()
    expected_prefix = (
        f"/user.slice/user-{uid}.slice/"
        f"user@{uid}.service/app.slice/"
    )
    if not scope_path.startswith(expected_prefix):
        return None
    unit = scope_path[len(expected_prefix) :]
    if (
        "/" in unit
        or not unit.startswith("run-")
        or not unit.endswith(".scope")
        or not all(
            character.isascii() and (character.isalnum() or character in "-_.")
            for character in unit
        )
    ):
        return None
    return unit


def verifier_systemd_environment() -> dict[str, str] | None:
    """Build a minimal systemd user-bus environment after checking its paths."""
    uid = os.getuid()
    runtime_directory = f"/run/user/{uid}"
    bus_socket = os.path.join(runtime_directory, "bus")
    try:
        runtime_stat = os.stat(runtime_directory, follow_symlinks=False)
        bus_stat = os.stat(bus_socket, follow_symlinks=False)
    except OSError:
        return None
    if (
        not stat.S_ISDIR(runtime_stat.st_mode)
        or runtime_stat.st_uid != uid
        or runtime_stat.st_mode & 0o077
        or not stat.S_ISSOCK(bus_stat.st_mode)
        or bus_stat.st_uid != uid
    ):
        return None
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": "/nonexistent",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "XDG_RUNTIME_DIR": runtime_directory,
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path={bus_socket}",
    }


def kill_verifier_scope(scope_path: str) -> bool:
    """Stop one validated scope through cgroup.kill or the same-user manager."""
    unit = verifier_scope_unit(scope_path)
    if unit is None:
        print(f"refusing invalid verifier scope path: {scope_path}", file=sys.stderr)
        return False

    cgroup_kill = os.path.join("/sys/fs/cgroup", scope_path.lstrip("/"), "cgroup.kill")
    try:
        fd = os.open(
            cgroup_kill,
            os.O_WRONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
        )
        try:
            if os.write(fd, b"1") != 1:
                raise OSError("short write to verifier cgroup.kill")
        finally:
            os.close(fd)
        return True
    except OSError as cgroup_error:
        environment = verifier_systemd_environment()
        if environment is not None:
            try:
                subprocess.run(
                    [
                        "/usr/bin/systemctl",
                        "--user",
                        "kill",
                        "--kill-whom=all",
                        "--signal=SIGKILL",
                        unit,
                    ],
                    env=environment,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=VERIFIER_SCOPE_KILL_TIMEOUT_SECONDS,
                    check=True,
                )
                return True
            except (OSError, subprocess.SubprocessError) as systemd_error:
                print(
                    f"could not kill verifier cgroup {scope_path}: "
                    f"cgroup.kill={cgroup_error}; systemd user-manager fallback={systemd_error}",
                    file=sys.stderr,
                )
                return False
        print(
            f"could not kill verifier cgroup {scope_path}: "
            f"cgroup.kill={cgroup_error}; validated systemd user bus unavailable",
            file=sys.stderr,
        )
        return False


def verifier_scope_paths(root_pid: int) -> list[str]:
    """Return this host's validated systemd verifier scopes from /proc."""
    expected_prefix = (
        f"/user.slice/user-{os.getuid()}.slice/"
        f"user@{os.getuid()}.service/app.slice/"
    )
    pending = [root_pid]
    seen: set[int] = set()
    scopes: set[str] = set()

    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        try:
            task_ids = os.listdir(f"/proc/{pid}/task")
        except OSError:
            task_ids = []
        for task_id in task_ids:
            try:
                with open(f"/proc/{pid}/task/{task_id}/children", encoding="ascii") as stream:
                    children = stream.read()
                pending.extend(int(child) for child in children.split())
            except (OSError, ValueError):
                pass
        try:
            with open(f"/proc/{pid}/cgroup", encoding="ascii") as stream:
                cgroup = stream.read()
        except OSError:
            continue
        for line in cgroup.splitlines():
            path = line.split(":", 2)[-1]
            if not path.startswith(expected_prefix):
                continue
            unit = path[len(expected_prefix) :]
            if verifier_scope_unit(path) == unit:
                scopes.add(path)

    return sorted(scopes)


def main() -> int:
    try:
        separator = sys.argv.index("--")
    except ValueError:
        print("usage: mission_012_mcp_supervisor.py -- HOST [ARGS...]", file=sys.stderr)
        return 64
    host_command = sys.argv[separator + 1 :]
    if not host_command or not os.path.isabs(host_command[0]):
        print("MCP host command must be an absolute executable path", file=sys.stderr)
        return 64

    shutdown_requested = False

    def stop_gracefully(_signum: int, _frame: object) -> None:
        nonlocal shutdown_requested
        shutdown_requested = True

    for handled_signal in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(handled_signal, stop_gracefully)

    try:
        host = subprocess.Popen(
            host_command,
            env=HOST_ENV,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=None,
            bufsize=0,
            close_fds=True,
            start_new_session=True,
        )
    except OSError:
        print("could not start the bounded MCP host", file=sys.stderr)
        return 69
    assert host.stdin is not None
    assert host.stdout is not None

    input_failed = False
    client_stdout_open = True
    client_stdin_open = True
    host_stdin_open = True
    host_stdout_open = True
    input_frame = bytearray()
    host_output_line_bytes = 0
    verifier_kill_request_failed = False
    shutdown_deadline: float | None = None

    def close_host_input() -> None:
        nonlocal host_stdin_open, shutdown_deadline
        if host_stdin_open:
            host_stdin_open = False
            if shutdown_deadline is None:
            # EOF is a clean shutdown request too; bound finalization without
            # requiring the client to send a separate signal.
                shutdown_deadline = time.monotonic() + HOST_FINALIZATION_TIMEOUT_SECONDS
            try:
                host.stdin.close()
            except OSError:
                pass

    client_input_fd = sys.stdin.fileno()
    client_output_fd = sys.stdout.fileno()
    host_input_fd = host.stdin.fileno()
    host_output_fd = host.stdout.fileno()

    def write_all(fd: int, data: bytes | bytearray) -> None:
        view = memoryview(data)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                raise OSError("short MCP pipe write")
            view = view[written:]

    try:
        while True:
            if shutdown_requested and shutdown_deadline is None:
                shutdown_deadline = time.monotonic() + HOST_FINALIZATION_TIMEOUT_SECONDS
                if client_stdin_open:
                    client_stdin_open = False
                    close_host_input()
            readable = []
            if client_stdin_open and host_stdin_open:
                readable.append(client_input_fd)
            if host_stdout_open:
                readable.append(host_output_fd)
            if not readable:
                break
            try:
                ready, _, _ = select.select(readable, [], [], 0.1)
            except InterruptedError:
                continue
            if shutdown_requested and client_stdin_open:
                continue
            if client_input_fd in ready and client_stdin_open and host_stdin_open:
                try:
                    incoming = os.read(client_input_fd, 65536)
                except OSError:
                    incoming = b""
                    input_failed = True
                if not incoming:
                    client_stdin_open = False
                    if input_frame:
                        try:
                            write_all(host_input_fd, input_frame)
                        except (BrokenPipeError, OSError, ValueError):
                            input_failed = True
                    close_host_input()
                else:
                    input_frame.extend(incoming)
                    while True:
                        newline = input_frame.find(b"\n")
                        if newline < 0:
                            break
                        frame_size = newline + 1
                        if frame_size > MAX_FRAME_BYTES:
                            input_failed = True
                            client_stdin_open = False
                            close_host_input()
                            input_frame.clear()
                            break
                        frame = bytes(input_frame[:frame_size])
                        del input_frame[:frame_size]
                        try:
                            write_all(host_input_fd, frame)
                        except (BrokenPipeError, OSError, ValueError):
                            input_failed = True
                            client_stdin_open = False
                            close_host_input()
                            break
                    if len(input_frame) >= MAX_FRAME_BYTES:
                        input_failed = True
                        client_stdin_open = False
                        input_frame.clear()
                        close_host_input()
            if host_output_fd in ready and host_stdout_open:
                try:
                    outgoing = os.read(host_output_fd, 65536)
                except OSError:
                    outgoing = b""
                    input_failed = True
                if not outgoing:
                    host_stdout_open = False
                    continue
                for index, byte in enumerate(outgoing):
                    if byte == 10:
                        host_output_line_bytes = 0
                    else:
                        host_output_line_bytes += 1
                        if host_output_line_bytes >= MAX_FRAME_BYTES:
                            input_failed = True
                            verifier_kill_request_failed |= not terminate_host_group(host)
                            host_stdout_open = False
                            outgoing = outgoing[:index]
                            break
                if client_stdout_open:
                    try:
                        write_all(client_output_fd, outgoing)
                    except (BrokenPipeError, OSError):
                        # Drain the trusted host even after the MCP client has
                        # gone away so its final receipt can reach disk.
                        client_stdout_open = False
            if not host_stdout_open:
                break
            if shutdown_deadline is not None and time.monotonic() >= shutdown_deadline:
                verifier_kill_request_failed |= not terminate_host_group(host)
                return (
                    VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS
                    if verifier_kill_request_failed
                    else 75
                )
    except OSError:
        input_failed = True
        verifier_kill_request_failed |= not terminate_host_group(host)
    finally:
        close_host_input()

    try:
        host_status = host.wait(timeout=15)
    except subprocess.TimeoutExpired:
        verifier_kill_request_failed |= not terminate_host_group(host)
        return (
            VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS
            if verifier_kill_request_failed
            else 75
        )
    if verifier_kill_request_failed:
        return VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS
    return 65 if input_failed else host_status


if __name__ == "__main__":
    raise SystemExit(main())
