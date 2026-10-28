#!/usr/bin/env python3
"""Relay MCP stdio and gracefully finalize the trusted host on client exit.

Claude Code may terminate a stdio MCP child after a print-mode turn instead
of closing its stdin first. The host uses a separate process group so relay
cancellation cannot kill it before finalization. This supervisor translates
EOF or SIGTERM into a clean stdin EOF, waits for host finalization, and kills
the host itself if its bounded shutdown expires.
"""

from __future__ import annotations

import argparse
import json
import os
import select
import secrets
import signal
import stat
import subprocess
import sys
import time
from collections.abc import Callable

MAX_FRAME_BYTES = 128 * 1024 + 1
HOST_FINALIZATION_TIMEOUT_SECONDS = 15
HOST_GROUP_TERMINATION_GRACE_SECONDS = 1
VERIFIER_SCOPE_KILL_TIMEOUT_SECONDS = 2
VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS = 76
GATE_BARRIER_TIMEOUT_SECONDS = 300
GATE_BARRIER_TIMEOUT_EXIT_STATUS = 77
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


def process_binding(pid: int) -> dict[str, object]:
    """Bind one live PID to its start time and executable path."""
    if pid <= 0:
        raise ValueError("process PID must be positive")
    stat_path = f"/proc/{pid}/stat"
    try:
        with open(stat_path, encoding="ascii") as stream:
            before = stream.read()
        close_paren = before.rfind(")")
        if close_paren < 0:
            raise ValueError("malformed process stat")
        before_fields = before[close_paren + 1 :].split()
        if not before_fields or before_fields[0] in {"Z", "X"}:
            raise ValueError("process is not live")
        start_time_ticks = int(before_fields[19])
        executable = os.readlink(f"/proc/{pid}/exe")
        with open(stat_path, encoding="ascii") as stream:
            after = stream.read()
        after_close_paren = after.rfind(")")
        if after_close_paren < 0:
            raise ValueError("malformed process stat")
        after_fields = after[after_close_paren + 1 :].split()
        if not after_fields or after_fields[0] in {"Z", "X"}:
            raise ValueError("process is not live")
        if start_time_ticks != int(after_fields[19]):
            raise ValueError("PID was reused while reading process binding")
    except (OSError, ValueError, IndexError) as error:
        raise RuntimeError("cannot read a stable MCP host process binding") from error
    if start_time_ticks <= 0 or not os.path.isabs(executable):
        raise RuntimeError("MCP host process binding has invalid fields")
    return {
        "pid": pid,
        "start_time_ticks": start_time_ticks,
        "executable": executable.removesuffix(" (deleted)"),
    }


def validate_gate_barrier_paths(ready_path: str | None, release_path: str | None) -> bool:
    """Validate the optional private ready/release files and report if enabled."""
    if ready_path is None and release_path is None:
        return False
    if ready_path is None or release_path is None:
        raise ValueError("gate barrier requires both ready and release paths")
    if not os.path.isabs(ready_path) or not os.path.isabs(release_path):
        raise ValueError("gate barrier paths must be absolute")
    ready = os.path.abspath(ready_path)
    release = os.path.abspath(release_path)
    runtime_dir = os.path.dirname(ready)
    try:
        metadata = os.stat(runtime_dir, follow_symlinks=False)
    except OSError as error:
        raise ValueError("gate barrier runtime directory is unavailable") from error
    if (
        os.path.basename(runtime_dir) != ".runtime"
        or os.path.dirname(release) != runtime_dir
        or os.path.basename(ready) != "mcp-gate-ready.json"
        or os.path.basename(release) != "mcp-gate-release.json"
        or not stat.S_ISDIR(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or os.path.lexists(ready)
        or os.path.lexists(release)
    ):
        raise ValueError("gate barrier paths are not new private .runtime files")
    if os.path.realpath(runtime_dir) != runtime_dir:
        raise ValueError("gate barrier runtime directory is not canonical")
    return True


def write_gate_ready(path: str, supervisor_pid: int, host: subprocess.Popen[bytes], nonce: str) -> None:
    binding = process_binding(host.pid)
    payload = {
        "schema_version": 1,
        "supervisor_pid": supervisor_pid,
        "host_pid": binding["pid"],
        "host_start_time_ticks": binding["start_time_ticks"],
        "host_executable": binding["executable"],
        "nonce": nonce,
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("ascii") + b"\n"
    fd = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
        0o600,
    )
    try:
        if os.write(fd, encoded) != len(encoded):
            raise OSError("short gate barrier ready write")
        os.fsync(fd)
    finally:
        os.close(fd)


def wait_for_gate_release(
    host: subprocess.Popen[bytes], release_path: str, nonce: str, *, stop_requested: Callable[[], bool]
) -> bool:
    deadline = time.monotonic() + GATE_BARRIER_TIMEOUT_SECONDS
    while time.monotonic() < deadline and not stop_requested():
        if host.poll() is not None:
            return False
        try:
            fd = os.open(
                release_path,
                os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0),
            )
        except FileNotFoundError:
            time.sleep(0.025)
            continue
        except OSError:
            return False
        try:
            metadata = os.fstat(fd)
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or metadata.st_nlink != 1
                or stat.S_IMODE(metadata.st_mode) != 0o600
                or metadata.st_size > 8192
            ):
                return False
            contents = os.read(fd, 8193)
        finally:
            os.close(fd)
        if len(contents) > 8192:
            return False
        try:
            release = json.loads(contents)
        except (UnicodeDecodeError, json.JSONDecodeError):
            return False
        return (
            isinstance(release, dict)
            and release.get("schema_version") == 1
            and release.get("nonce") == nonce
            and isinstance(release.get("gate_operation_sha256"), str)
            and len(release["gate_operation_sha256"]) == 64
            and all(character in "0123456789abcdef" for character in release["gate_operation_sha256"])
        )
    return False


def is_shutdown_request(frame: bytes) -> bool:
    try:
        request = json.loads(frame)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return False
    return isinstance(request, dict) and "id" in request and request.get("method") == "shutdown"


def main() -> int:
    try:
        separator = sys.argv.index("--")
    except ValueError:
        print("usage: mission_012_mcp_supervisor.py -- HOST [ARGS...]", file=sys.stderr)
        return 64
    options = argparse.ArgumentParser(add_help=False)
    options.add_argument("--gate-barrier-ready")
    options.add_argument("--gate-barrier-release")
    try:
        parsed = options.parse_args(sys.argv[1:separator])
        gate_barrier_enabled = validate_gate_barrier_paths(
            parsed.gate_barrier_ready, parsed.gate_barrier_release
        )
    except (SystemExit, ValueError) as error:
        if isinstance(error, SystemExit):
            return 64
        print(str(error), file=sys.stderr)
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
    gate_barrier_nonce = secrets.token_hex(32) if gate_barrier_enabled else None
    gate_barrier_triggered = False
    gate_barrier_failed = False

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
                        if (
                            gate_barrier_enabled
                            and not gate_barrier_triggered
                            and is_shutdown_request(frame)
                        ):
                            gate_barrier_triggered = True
                            try:
                                write_gate_ready(
                                    parsed.gate_barrier_ready,
                                    os.getpid(),
                                    host,
                                    gate_barrier_nonce,
                                )
                                gate_barrier_failed = not wait_for_gate_release(
                                    host,
                                    parsed.gate_barrier_release,
                                    gate_barrier_nonce,
                                    stop_requested=lambda: shutdown_requested,
                                )
                            except (OSError, RuntimeError):
                                gate_barrier_failed = True
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
    if gate_barrier_enabled and (not gate_barrier_triggered or gate_barrier_failed):
        return GATE_BARRIER_TIMEOUT_EXIT_STATUS
    return 65 if input_failed else host_status


if __name__ == "__main__":
    raise SystemExit(main())
