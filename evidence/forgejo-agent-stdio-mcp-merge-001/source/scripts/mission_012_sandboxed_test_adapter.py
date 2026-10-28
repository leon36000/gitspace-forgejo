#!/usr/bin/env python3
"""Run the explicit MISSION-012 fixture adapter behind a Bubblewrap boundary.

The trusted wrapper owns the MCP process and connects its stdio directly to
the sandboxed adapter. The adapter receives no MCP configuration, workspace,
credential, or inherited descriptor beyond its protocol streams.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
ADAPTER = ROOT / "scripts/mission_012_test_agent_adapter.py"
BWRAP = Path("/usr/bin/bwrap")
PYTHON = Path("/usr/bin/python3")
HOST_ENV = {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"}
SANDBOX_ENV = {
    "PATH": "/usr/bin:/bin",
    "HOME": "/tmp",
    "PWD": "/tmp",
    "LANG": "C.UTF-8",
    "LC_ALL": "C.UTF-8",
    "PYTHONDONTWRITEBYTECODE": "1",
}
SYNTHETIC_CREDENTIAL = b"SYNTHETIC-CREDENTIAL-CANARY-ONLY\n"
MAX_CONFIG_BYTES = 64 * 1024
MAX_ADAPTER_OUTPUT_BYTES = 1024 * 1024
ADAPTER_TIMEOUT_SECONDS = 540
HOST_SHUTDOWN_SECONDS = 20
VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS = 76


def private_regular_file(path: Path, description: str) -> bytes:
    metadata = path.lstat()
    if path.is_symlink() or not path.is_file() or metadata.st_mode & 0o7777 != 0o600:
        raise RuntimeError(f"{description} must be a private regular file")
    if metadata.st_uid != os.geteuid() or metadata.st_size > MAX_CONFIG_BYTES:
        raise RuntimeError(f"{description} has unsafe ownership or size")
    return path.read_bytes()


def validate_credential_probe_path(value: str | None) -> None:
    if value is None:
        return
    if not isinstance(value, str):
        raise ValueError("credential probe override must be a string path")
    parts = Path(value).parts
    if (
        len(value) > 4096
        or "\x00" in value
        or len(parts) != 5
        or parts[0] != "/"
        or parts[1] != "proc"
        or not parts[2].isdigit()
        or parts[3] != "fd"
        or not parts[4].isdigit()
    ):
        raise ValueError("credential probe override must name a bounded host /proc/<pid>/fd/<fd> path")


def supervisor_command(config_path: Path) -> list[str]:
    config = json.loads(private_regular_file(config_path, "MCP config"))
    servers = config.get("mcpServers") if isinstance(config, dict) else None
    server = servers.get("mission012") if isinstance(servers, dict) else None
    if not isinstance(server, dict):
        raise RuntimeError("the mission012 MCP server is missing")
    command = server.get("command")
    arguments = server.get("args")
    if (
        not isinstance(command, str)
        or not Path(command).is_absolute()
        or not isinstance(arguments, list)
        or any(not isinstance(argument, str) or "\x00" in argument for argument in arguments)
    ):
        raise RuntimeError("the trusted MCP server command is invalid")
    if Path(command).resolve(strict=True) != PYTHON.resolve(strict=True) or not arguments:
        raise RuntimeError("the trusted MCP server must be launched by the pinned local Python runtime")
    entrypoint = Path(arguments[0])
    try:
        entrypoint_info = entrypoint.lstat()
    except OSError:
        raise RuntimeError("the trusted MCP server entrypoint is unavailable") from None
    if (
        not entrypoint.is_absolute()
        or entrypoint.is_symlink()
        or not entrypoint.is_file()
        or entrypoint_info.st_uid != os.geteuid()
        or entrypoint_info.st_mode & 0o002
        or entrypoint_info.st_size > 2 * 1024 * 1024
    ):
        raise RuntimeError(
            "the trusted MCP server entrypoint has unsafe metadata "
            f"(absolute={entrypoint.is_absolute()}, symlink={entrypoint.is_symlink()}, "
            f"regular={entrypoint.is_file()}, owner={entrypoint_info.st_uid}, "
            f"mode={entrypoint_info.st_mode & 0o7777:o}, size={entrypoint_info.st_size})"
        )
    return [command, *arguments]


def bubblewrap_command(
    credential_path: Path,
    loopback_port: int,
    *,
    owner: str = "forge-admin",
    repository: str = "mission-012-clamp",
    pull_number: int = 1,
    forgejo_probe_port: int | None = None,
    credential_probe_path: str | None = None,
    probe_only: bool = False,
    probe_hold_open: bool = False,
    wait_at_gate_barrier: bool = False,
) -> list[str]:
    if probe_hold_open and not probe_only:
        raise ValueError("a held probe requires probe-only mode")
    if wait_at_gate_barrier and probe_only:
        raise ValueError("the gate barrier requires the stdio MCP task adapter")
    if forgejo_probe_port is not None and (
        not isinstance(forgejo_probe_port, int)
        or isinstance(forgejo_probe_port, bool)
        or not 1 <= forgejo_probe_port <= 65535
    ):
        raise ValueError("Forgejo probe port must be between 1 and 65535")
    validate_credential_probe_path(credential_probe_path)
    adapter_path = Path("/opt/mission_012_test_agent_adapter.py")
    probe_port = forgejo_probe_port if forgejo_probe_port is not None else loopback_port
    command = [
        str(BWRAP),
        "--die-with-parent",
        "--unshare-net",
        "--unshare-pid",
        "--new-session",
        "--ro-bind", "/usr", "/usr",
        "--dir", "/etc",
        "--ro-bind", "/etc/alternatives", "/etc/alternatives",
        "--symlink", "usr/lib", "/lib",
        "--symlink", "usr/bin", "/bin",
        "--tmpfs", "/tmp",
        "--tmpfs", "/home",
        "--dev", "/dev",
        "--proc", "/proc",
        "--dir", "/opt",
        "--ro-bind", str(ADAPTER), str(adapter_path),
        "--chdir", "/tmp",
        "--clearenv",
        *[item for key, value in SANDBOX_ENV.items() for item in ("--setenv", key, value)],
        str(PYTHON), str(adapter_path),
        "--probe-harness" if probe_only else "--stdio-mcp",
        "--probe-credential", credential_probe_path if credential_probe_path is not None else str(credential_path),
        "--probe-port", str(probe_port),
        "--probe-owner", owner,
        "--probe-repository", repository,
        "--probe-pull-number", str(pull_number),
    ]
    if probe_hold_open:
        command.append("--probe-hold-open")
    if wait_at_gate_barrier:
        command.append("--wait-at-gate-barrier")
    return command


def stop_process_group(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=2)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass


def parse_adapter_event(stderr: bytes) -> dict[str, object]:
    if len(stderr) > MAX_ADAPTER_OUTPUT_BYTES:
        raise RuntimeError("sandbox adapter diagnostics exceeded the output limit")
    lines = [line for line in stderr.splitlines() if line]
    if len(lines) != 1:
        raise RuntimeError("sandbox adapter did not emit exactly one bounded status event")
    event = json.loads(lines[0])
    if not isinstance(event, dict):
        raise RuntimeError("sandbox adapter status event is not an object")
    return event


def run(
    config_path: Path,
    *,
    interrupt_after_write: bool,
    capture_host_stderr: bool,
    forgejo_probe_port: int | None = None,
    credential_probe_path: str | None = None,
    owner: str = "forge-admin",
    repository: str = "mission-012-clamp",
    pull_number: int = 1,
    wait_at_gate_barrier: bool = False,
) -> tuple[dict[str, object], int]:
    if forgejo_probe_port is not None and (
        not isinstance(forgejo_probe_port, int)
        or isinstance(forgejo_probe_port, bool)
        or not 1 <= forgejo_probe_port <= 65535
    ):
        raise ValueError("Forgejo probe port must be between 1 and 65535")
    validate_credential_probe_path(credential_probe_path)
    if not BWRAP.is_file() or not PYTHON.is_file() or ADAPTER.is_symlink() or not ADAPTER.is_file():
        raise RuntimeError("the local Bubblewrap, Python, or test adapter is unavailable")
    command = supervisor_command(config_path)
    if not command:
        raise RuntimeError("the trusted MCP command is empty")

    with tempfile.TemporaryDirectory(prefix="mission012-sandbox-", dir="/tmp") as temporary:
        credential_path = Path(temporary) / "synthetic-credential.txt"
        if credential_probe_path is None:
            credential_path.write_bytes(SYNTHETIC_CREDENTIAL)
            credential_path.chmod(0o600)
        else:
            credential_path = Path(credential_probe_path)

        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(("127.0.0.1", 0))
        listener.listen(4)
        listener.setblocking(False)
        loopback_port = listener.getsockname()[1]

        host = subprocess.Popen(
            command,
            cwd="/",
            env=HOST_ENV,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=None if capture_host_stderr else subprocess.DEVNULL,
            bufsize=0,
            close_fds=True,
            start_new_session=True,
        )
        assert host.stdin is not None and host.stdout is not None
        adapter_command = bubblewrap_command(
            credential_path,
            loopback_port,
            owner=owner,
            repository=repository,
            pull_number=pull_number,
            forgejo_probe_port=forgejo_probe_port,
            credential_probe_path=credential_probe_path,
            wait_at_gate_barrier=wait_at_gate_barrier,
        )
        if interrupt_after_write:
            adapter_command.append("--interrupt-after-write")

        adapter: subprocess.Popen[bytes] | None = None
        try:
            adapter = subprocess.Popen(
                adapter_command,
                stdin=host.stdout,
                stdout=host.stdin,
                stderr=subprocess.PIPE,
                env=HOST_ENV,
                bufsize=0,
                close_fds=True,
                start_new_session=True,
            )
        except BaseException:
            host.stdout.close()
            host.stdin.close()
            listener.close()
            stop_process_group(host)
            raise
        finally:
            if adapter is not None:
                # The wrapper must not keep either MCP pipe open after the child is started.
                host.stdout.close()
                host.stdin.close()
        assert adapter is not None

        try:
            adapter_stderr = adapter.communicate(timeout=ADAPTER_TIMEOUT_SECONDS)[1]
        except subprocess.TimeoutExpired:
            stop_process_group(adapter)
            stop_process_group(host)
            listener.close()
            raise RuntimeError("sandbox adapter exceeded its bounded runtime") from None

        try:
            host_status = host.wait(timeout=HOST_SHUTDOWN_SECONDS)
        except subprocess.TimeoutExpired:
            stop_process_group(host)
            listener.close()
            raise RuntimeError("trusted MCP host did not finalize after adapter EOF") from None

        observed_route_requests: list[bytes] = []
        while True:
            try:
                unexpected, _ = listener.accept()
            except BlockingIOError:
                break
            unexpected.settimeout(0.2)
            try:
                observed_route_requests.append(unexpected.recv(512))
            except TimeoutError:
                observed_route_requests.append(b"")
            finally:
                unexpected.close()
        if observed_route_requests:
            listener.close()
            raise RuntimeError("sandbox adapter reached a host listener through a direct Forgejo route probe")
        listener.close()

        event = parse_adapter_event(adapter_stderr)
        if event.get("type") == "error":
            raise RuntimeError(f"sandbox adapter rejected its MCP session: {event.get('error')}")
        checks = event.get("sandbox_checks")
        expected_checks = {
            "credential_visible": False,
            "network_connected": False,
            "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
            "open_fds": [0, 1, 2],
        }
        probe_target_port = forgejo_probe_port if forgejo_probe_port is not None else loopback_port
        if (
            not isinstance(checks, dict)
            or {
                key: value
                for key, value in checks.items()
                if key not in {"probe_target_port", "network_namespace", "pid_namespace", "environment_keys"}
            }
            != expected_checks
            or checks.get("probe_target_port") != probe_target_port
            or not isinstance(checks.get("network_namespace"), str)
            or checks.get("network_namespace") == os.readlink("/proc/self/ns/net")
            or not isinstance(checks.get("pid_namespace"), str)
            or checks.get("pid_namespace") == os.readlink("/proc/self/ns/pid")
            or checks.get("environment_keys") != sorted(SANDBOX_ENV)
        ):
            raise RuntimeError(f"sandbox adapter capability probe was incomplete: {checks!r}")
        reported_checks = {
            **checks,
            "supervisor_observed_route_connections": len(observed_route_requests),
        }
        if adapter.returncode not in (0, 75):
            return ({"type": "error", "error": "sandbox adapter failed; see bounded local diagnostics"}, 1)
        if host_status == VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS:
            raise RuntimeError(
                "trusted MCP supervisor could not confirm verifier-scope cleanup "
                "(exit 76); the candidate result is inconclusive"
            )
        if host_status != 0:
            raise RuntimeError(f"trusted MCP host failed to finalize cleanly (exit {host_status})")
        expected_subtype = "interrupted" if adapter.returncode == 75 else "success"
        if event.get("type") != "result" or event.get("subtype") != expected_subtype or event.get("is_error") is not False:
            raise RuntimeError("sandbox adapter status does not match its exit code")
        return (
            {
                "type": "result",
                "subtype": expected_subtype,
                "is_error": False,
                "result": event.get("result"),
                "sandbox_checks": reported_checks,
                "host_network_namespace": os.readlink("/proc/self/ns/net"),
                "host_pid_namespace": os.readlink("/proc/self/ns/pid"),
            },
            adapter.returncode,
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mcp-config", required=True, type=Path)
    parser.add_argument("--interrupt-after-write", action="store_true")
    parser.add_argument("--capture-host-stderr", action="store_true")
    parser.add_argument("--forgejo-probe-port", type=int)
    parser.add_argument("--credential-probe-path")
    parser.add_argument("--probe-owner", default="forge-admin")
    parser.add_argument("--probe-repository", default="mission-012-clamp")
    parser.add_argument("--probe-pull-number", type=int, default=1)
    parser.add_argument("--wait-at-gate-barrier", action="store_true")
    args = parser.parse_args()
    try:
        event, status = run(
            args.mcp_config,
            interrupt_after_write=args.interrupt_after_write,
            capture_host_stderr=args.capture_host_stderr,
            forgejo_probe_port=args.forgejo_probe_port,
            credential_probe_path=args.credential_probe_path,
            owner=args.probe_owner,
            repository=args.probe_repository,
            pull_number=args.probe_pull_number,
            wait_at_gate_barrier=args.wait_at_gate_barrier,
        )
    except Exception as error:
        event = {"type": "error", "error": str(error)}
        status = 1
    print(json.dumps(event, sort_keys=True, separators=(",", ":")))
    return status


if __name__ == "__main__":
    raise SystemExit(main())
