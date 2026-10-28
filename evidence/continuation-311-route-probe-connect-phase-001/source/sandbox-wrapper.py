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
import re
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time

import mission_012_test_agent_adapter as agent_adapter


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
PRIVATE_PROBE_LABELS = {
    "admission_source",
    "forgejo_read_token",
    "host_admission",
    "invalid_forgejo_read_token",
    "mission_journal",
    "webhook_inbox",
    "webhook_attestation_seed",
}
WEBHOOK_HANDOFF_PROBE_LABELS = PRIVATE_PROBE_LABELS - {"webhook_attestation_seed"}
TEST_ONLY_PROBE_LABELS = {"host_systemd_bus"}
PRIVATE_WRITE_PROBE_LABELS = {"webhook_inbox"}


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


def validate_webhook_attestation_seed_probe(path: Path) -> None:
    try:
        resolved = path.resolve(strict=True)
        metadata = path.lstat()
        parent = path.parent.lstat()
    except OSError:
        raise RuntimeError("webhook attestation seed probe file is unavailable") from None
    if (
        not path.is_absolute()
        or not path.is_relative_to(Path("/tmp"))
        or path.name != "webhook-attestation.seed"
        or resolved != path
        or path.parent.is_symlink()
        or not stat.S_ISDIR(parent.st_mode)
        or parent.st_uid != os.geteuid()
        or stat.S_IMODE(parent.st_mode) != 0o700
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.geteuid()
        or metadata.st_nlink != 1
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_size != 32
    ):
        raise RuntimeError("webhook attestation seed probe file is unsafe")


def supervisor_command(
    config_path: Path,
    *,
    gate_barrier_ready_file: Path | None = None,
    gate_barrier_release_file: Path | None = None,
) -> list[str]:
    if (gate_barrier_ready_file is None) != (gate_barrier_release_file is None):
        raise ValueError("gate barrier ready and release paths must be supplied together")
    barrier_arguments: list[str] = []
    if gate_barrier_ready_file is not None and gate_barrier_release_file is not None:
        ready = gate_barrier_ready_file
        release = gate_barrier_release_file
        runtime_dir = ready.parent
        if (
            not ready.is_absolute()
            or not release.is_absolute()
            or ready.name != "mcp-gate-ready.json"
            or release.name != "mcp-gate-release.json"
            or ready.parent != release.parent
            or runtime_dir.name != ".runtime"
            or not runtime_dir.is_relative_to(Path("/tmp"))
            or runtime_dir.resolve(strict=True) != runtime_dir
        ):
            raise ValueError("gate barrier paths must be canonical private .runtime files")
        metadata = runtime_dir.lstat()
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or stat.S_IMODE(metadata.st_mode) != 0o700
            or ready.exists()
            or ready.is_symlink()
            or release.exists()
            or release.is_symlink()
        ):
            raise ValueError("gate barrier files must be new and owned by the private runtime directory")
        barrier_arguments = [
            "--gate-barrier-ready", str(ready),
            "--gate-barrier-release", str(release),
        ]
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
    separators = [index for index, argument in enumerate(arguments) if argument == "--"]
    if len(separators) != 1:
        raise RuntimeError("the trusted MCP supervisor command must have exactly one host separator")
    separator = separators[0]
    if any(argument.startswith("--gate-barrier-") for argument in arguments[:separator]):
        raise RuntimeError("MCP configuration cannot supply trusted gate barrier paths")
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
    return [command, *arguments[:separator], *barrier_arguments, *arguments[separator:]]


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
    private_file_probes: list[tuple[str, Path]] | None = None,
    private_file_write_probes: list[tuple[str, Path]] | None = None,
) -> list[str]:
    if probe_hold_open and not probe_only:
        raise ValueError("a held probe requires probe-only mode")
    if wait_at_gate_barrier and probe_only:
        raise ValueError("the gate barrier requires the stdio MCP task adapter")
    probes = private_file_probes or []
    allowed_probe_labels = PRIVATE_PROBE_LABELS | (
        TEST_ONLY_PROBE_LABELS if probe_only else set()
    )
    if len(probes) > len(allowed_probe_labels) or len({label for label, _ in probes}) != len(probes):
        raise ValueError("private runtime file probes must use unique bounded labels")
    for label, path in probes:
        if (
            label not in allowed_probe_labels
            or not path.is_absolute()
            or len(str(path)) > 4096
            or "\x00" in str(path)
        ):
            raise ValueError("private runtime file probe must identify an absolute private file")
    write_probes = private_file_write_probes or []
    if write_probes and not probe_only:
        raise ValueError("private runtime write probes are available only in probe-only mode")
    if (
        len(write_probes) > len(PRIVATE_WRITE_PROBE_LABELS)
        or len({label for label, _ in write_probes}) != len(write_probes)
    ):
        raise ValueError("private runtime write probes must use unique bounded labels")
    for label, path in write_probes:
        if (
            label not in PRIVATE_WRITE_PROBE_LABELS
            or not path.is_absolute()
            or ".." in path.parts
            or not path.is_relative_to(Path("/tmp"))
            or len(str(path)) > 4096
            or "\x00" in str(path)
        ):
            raise ValueError("private runtime write probe must identify a synthetic canary under /tmp")
        try:
            agent_adapter.validate_private_webhook_inbox_write_probe(path)
        except RuntimeError as error:
            raise ValueError(str(error)) from error
        try:
            resolved = path.resolve(strict=True)
            runtime_metadata = path.parent.lstat()
            metadata = path.lstat()
        except OSError as error:
            raise ValueError("private runtime write probe canary must already exist") from error
        if (
            resolved != path
            or path.parent.is_symlink()
            or not path.parent.is_dir()
            or runtime_metadata.st_mode & 0o7777 != 0o700
            or runtime_metadata.st_uid != os.geteuid()
            or path.is_symlink()
            or not path.is_file()
            or metadata.st_mode & 0o7777 != 0o600
            or metadata.st_uid != os.geteuid()
            or metadata.st_size > MAX_CONFIG_BYTES
        ):
            raise ValueError("private runtime write probe canary must be a bounded private regular file")
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
        "--tmpfs", "/run",
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
    for label, path in probes:
        command.extend(("--probe-private-file", label, str(path)))
    for label, path in write_probes:
        command.extend(("--probe-private-write-file", label, str(path)))
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


def _read_proc_stat(pid: int) -> tuple[int, int]:
    raw = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    close_paren = raw.rfind(")")
    if close_paren < 0:
        raise RuntimeError("process stat record is malformed")
    fields = raw[close_paren + 2 :].split()
    if len(fields) < 20:
        raise RuntimeError("process stat record is incomplete")
    return int(fields[1]), int(fields[19])


def _read_proc_process(pid: int) -> dict[str, object]:
    first_parent, first_start = _read_proc_stat(pid)
    status = Path(f"/proc/{pid}/status").read_text(encoding="ascii")
    uid_line = next((line for line in status.splitlines() if line.startswith("Uid:")), None)
    if uid_line is None:
        raise RuntimeError("process status omitted its UID")
    uid_fields = uid_line.split()
    if len(uid_fields) != 5:
        raise RuntimeError("process status UID fields are malformed")
    pid_namespace = os.readlink(f"/proc/{pid}/ns/pid")
    network_namespace = os.readlink(f"/proc/{pid}/ns/net")
    mount_namespace = os.readlink(f"/proc/{pid}/ns/mnt")
    user_namespace = os.readlink(f"/proc/{pid}/ns/user")
    executable = os.readlink(f"/proc/{pid}/exe")
    if executable.endswith(" (deleted)"):
        raise RuntimeError("sandbox process executable was deleted")
    open_fds = sorted(
        int(name) for name in os.listdir(f"/proc/{pid}/fd") if name.isascii() and name.isdigit()
    )
    cgroup_lines = [
        line for line in Path(f"/proc/{pid}/cgroup").read_text(encoding="ascii").splitlines()
        if line.startswith("0::")
    ]
    if len(cgroup_lines) != 1 or not cgroup_lines[0][3:].startswith("/"):
        raise RuntimeError("process cgroup record is missing or ambiguous")
    second_parent, second_start = _read_proc_stat(pid)
    if (first_parent, first_start) != (second_parent, second_start):
        raise RuntimeError("process identity changed during parent observation")
    return {
        "pid": pid,
        "parent_pid": first_parent,
        "start_time_ticks": first_start,
        "effective_uid": int(uid_fields[2]),
        "pid_namespace": pid_namespace,
        "network_namespace": network_namespace,
        "mount_namespace": mount_namespace,
        "user_namespace": user_namespace,
        "cgroup_path": cgroup_lines[0][3:],
        "executable": executable,
        "open_fds": open_fds,
    }


def _sandbox_process_tree_snapshot(root_pid: int) -> tuple[dict[str, object], list[dict[str, object]]]:
    process_stats: dict[int, tuple[int, int]] = {}
    for name in os.listdir("/proc"):
        if not name.isascii() or not name.isdigit():
            continue
        pid = int(name)
        try:
            process_stats[pid] = _read_proc_stat(pid)
        except FileNotFoundError:
            continue
    root_stat = process_stats.get(root_pid)
    if root_stat is None:
        raise RuntimeError("sandbox root process disappeared before parent observation")

    children: dict[int, list[int]] = {}
    for pid, (parent_pid, _) in process_stats.items():
        children.setdefault(parent_pid, []).append(pid)
    descendant_pids: list[int] = []
    pending = list(children.get(root_pid, []))
    seen = {root_pid}
    while pending:
        pid = pending.pop()
        if pid in seen:
            raise RuntimeError("process tree contains a duplicate or cycle")
        seen.add(pid)
        descendant_pids.append(pid)
        pending.extend(children.get(pid, []))

    root = _read_proc_process(root_pid)
    if (root["parent_pid"], root["start_time_ticks"]) != root_stat:
        raise RuntimeError("sandbox root changed during parent observation")
    descendants: list[dict[str, object]] = []
    for pid in sorted(descendant_pids):
        process = _read_proc_process(pid)
        if process_stats.get(pid) != (process["parent_pid"], process["start_time_ticks"]):
            raise RuntimeError("sandbox descendant changed during parent observation")
        descendants.append(process)

    for pid in [root_pid, *descendant_pids]:
        if process_stats.get(pid) != _read_proc_stat(pid):
            raise RuntimeError("sandbox process tree changed during parent observation")
    return root, descendants


def supervisor_observe_sandbox_process_tree(root_pid: int) -> dict[str, object]:
    """Independently read the live Bubblewrap descendants through host procfs."""
    if not isinstance(root_pid, int) or isinstance(root_pid, bool) or root_pid < 1:
        raise ValueError("sandbox process root PID is invalid")
    host_pid_namespace = os.readlink("/proc/self/ns/pid")
    host_network_namespace = os.readlink("/proc/self/ns/net")
    host_mount_namespace = os.readlink("/proc/self/ns/mnt")
    host_user_namespace = os.readlink("/proc/self/ns/user")
    expected_bwrap = str(BWRAP.resolve(strict=True))
    expected_python = str(PYTHON.resolve(strict=True))
    deadline = time.monotonic() + 3
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            root, descendants = _sandbox_process_tree_snapshot(root_pid)
            if root["executable"] != expected_bwrap:
                raise RuntimeError("sandbox root executable differs from the pinned Bubblewrap binary")
            if (
                root["pid_namespace"] != host_pid_namespace
                or root["network_namespace"] != host_network_namespace
                or root["mount_namespace"] != host_mount_namespace
                or root["user_namespace"] != host_user_namespace
            ):
                raise RuntimeError("Bubblewrap launcher root is not in the supervisor namespaces")
            if not descendants:
                raise RuntimeError("sandbox process tree has no live descendants")
            sandbox_pid_namespaces = {item["pid_namespace"] for item in descendants}
            sandbox_network_namespaces = {item["network_namespace"] for item in descendants}
            sandbox_mount_namespaces = {item["mount_namespace"] for item in descendants}
            sandbox_user_namespaces = {item["user_namespace"] for item in descendants}
            sandbox_cgroups = {item["cgroup_path"] for item in descendants}
            if (
                len(sandbox_pid_namespaces) != 1
                or len(sandbox_network_namespaces) != 1
                or len(sandbox_mount_namespaces) != 1
                or len(sandbox_user_namespaces) != 1
                or len(sandbox_cgroups) != 1
                or host_pid_namespace in sandbox_pid_namespaces
                or host_network_namespace in sandbox_network_namespaces
                or host_mount_namespace in sandbox_mount_namespaces
                or host_user_namespace in sandbox_user_namespaces
                or root["cgroup_path"] not in sandbox_cgroups
            ):
                raise RuntimeError("sandbox descendants do not share the expected namespaces and cgroup")
            if any(item["effective_uid"] != os.geteuid() for item in descendants):
                raise RuntimeError("sandbox descendant unexpectedly changed its effective UID")
            python_agents = [item for item in descendants if item["executable"] == expected_python]
            if len(python_agents) != 1 or python_agents[0]["open_fds"] != [0, 1, 2]:
                raise RuntimeError("sandbox Python agent identity or descriptor set is unexpected")
            time.sleep(0.01)
            root_again, descendants_again = _sandbox_process_tree_snapshot(root_pid)
            first_identity = [
                (item["pid"], item["parent_pid"], item["start_time_ticks"], item["executable"])
                for item in [root, *descendants]
            ]
            second_identity = [
                (item["pid"], item["parent_pid"], item["start_time_ticks"], item["executable"])
                for item in [root_again, *descendants_again]
            ]
            if first_identity != second_identity:
                raise RuntimeError("sandbox process tree changed between parent observations")
            if any(
                item["pid_namespace"] != next(iter(sandbox_pid_namespaces))
                or item["network_namespace"] != next(iter(sandbox_network_namespaces))
                or item["mount_namespace"] != next(iter(sandbox_mount_namespaces))
                or item["user_namespace"] != next(iter(sandbox_user_namespaces))
                or item["cgroup_path"] not in sandbox_cgroups
                or item["effective_uid"] != os.geteuid()
                for item in descendants_again
            ):
                raise RuntimeError("sandbox process namespace or UID changed between parent observations")
            python_again = [item for item in descendants_again if item["executable"] == expected_python]
            if len(python_again) != 1 or python_again[0]["open_fds"] != [0, 1, 2]:
                raise RuntimeError("sandbox Python agent descriptors changed during parent observation")
            return {
                "schema_version": 1,
                "source": "parent_procfs",
                "host_pid_namespace": host_pid_namespace,
                "host_network_namespace": host_network_namespace,
                "host_mount_namespace": host_mount_namespace,
                "host_user_namespace": host_user_namespace,
                "sandbox_pid_namespace": next(iter(sandbox_pid_namespaces)),
                "sandbox_network_namespace": next(iter(sandbox_network_namespaces)),
                "sandbox_mount_namespace": next(iter(sandbox_mount_namespaces)),
                "sandbox_user_namespace": next(iter(sandbox_user_namespaces)),
                "all_descendants_share_root_cgroup": True,
                "sandbox_descendant_count": len(descendants),
                "agent_python_process_count": len(python_agents),
                "agent_python_open_fds": python_agents[0]["open_fds"],
            }
        except (OSError, RuntimeError, ValueError) as error:
            last_error = error
            if "unexpectedly changed" in str(error) or "do not share isolated" in str(error):
                raise RuntimeError(f"host-side sandbox process observation failed: {error}") from None
            time.sleep(0.02)
    raise RuntimeError(
        "host-side sandbox process observation was inconclusive"
        + (f": {last_error}" if last_error is not None else "")
    ) from None


def _wait_for_gate_barrier_ready(
    adapter: subprocess.Popen[bytes],
    host: subprocess.Popen[bytes],
    ready_file: Path,
    deadline: float,
) -> None:
    while time.monotonic() < deadline:
        try:
            ready_file.lstat()
        except FileNotFoundError:
            if adapter.poll() is not None:
                raise RuntimeError("sandbox adapter exited before its trusted gate barrier")
            time.sleep(0.025)
            continue
        except OSError:
            raise RuntimeError("trusted gate barrier observation was inconclusive") from None
        payload = private_regular_file(ready_file, "trusted gate readiness receipt")
        try:
            ready = json.loads(payload)
        except (UnicodeDecodeError, json.JSONDecodeError):
            raise RuntimeError("trusted gate readiness receipt is malformed") from None
        if (
            not isinstance(ready, dict)
            or ready.get("schema_version") != 1
            or ready.get("supervisor_pid") != host.pid
            or not isinstance(ready.get("host_pid"), int)
            or isinstance(ready.get("host_pid"), bool)
            or ready["host_pid"] < 1
            or not isinstance(ready.get("host_start_time_ticks"), int)
            or isinstance(ready.get("host_start_time_ticks"), bool)
            or ready["host_start_time_ticks"] < 1
            or not isinstance(ready.get("host_executable"), str)
            or not ready["host_executable"].startswith("/")
            or ready["host_executable"].endswith(" (deleted)")
            or not isinstance(ready.get("nonce"), str)
            or re.fullmatch(r"[0-9a-f]{64}", ready["nonce"]) is None
        ):
            raise RuntimeError("trusted gate readiness receipt does not bind the running MCP supervisor")
        return
    raise RuntimeError("trusted gate barrier was not reached before the sandbox timeout")


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


def test_adapter_work_item_observation(event: dict[str, object]) -> bool:
    observed = event.get("test_adapter_untrusted_work_item_observed")
    if type(observed) is not bool:
        raise RuntimeError("sandbox adapter omitted its test-adapter issue observation")
    return observed


def validate_direct_route_probe_errors(value: object) -> None:
    """Require all configured direct routes to fail before a connection exists."""
    expected_names = ["api", "web", "git_receive_pack"]
    if not isinstance(value, list) or len(value) != len(expected_names):
        raise RuntimeError("route probe diagnostics must cover all direct routes")
    expected_keys = {"name", "phase", "error_category", "errno"}
    for expected_name, diagnostic in zip(expected_names, value):
        if (
            not isinstance(diagnostic, dict)
            or set(diagnostic) != expected_keys
            or diagnostic.get("name") != expected_name
        ):
            raise RuntimeError("route probe diagnostics must identify all direct routes")
        if diagnostic.get("phase") != "connect":
            raise RuntimeError("direct route must be blocked at connect stage")
        code = diagnostic.get("errno")
        if (
            diagnostic.get("error_category") != "os_error"
            or type(code) is not int
            or code <= 0
        ):
            raise RuntimeError("connect-stage refusal must include a positive OS error number")


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
    gate_barrier_ready_file: Path | None = None,
    gate_barrier_release_file: Path | None = None,
    private_file_probes: list[tuple[str, Path]] | None = None,
) -> tuple[dict[str, object], int]:
    if forgejo_probe_port is not None and (
        not isinstance(forgejo_probe_port, int)
        or isinstance(forgejo_probe_port, bool)
        or not 1 <= forgejo_probe_port <= 65535
    ):
        raise ValueError("Forgejo probe port must be between 1 and 65535")
    validate_credential_probe_path(credential_probe_path)
    if (
        gate_barrier_ready_file is not None
        and gate_barrier_release_file is not None
        and not wait_at_gate_barrier
    ):
        raise ValueError("gate barrier requires the adapter's post-acceptance wait")
    probes = private_file_probes or []
    if probes:
        labels = {label for label, _ in probes}
        if len(labels) != len(probes) or not labels <= PRIVATE_PROBE_LABELS:
            raise ValueError("webhook sandbox private file probe labels are invalid")
        seed_probes = [(label, path) for label, path in probes if label == "webhook_attestation_seed"]
        for _, path in seed_probes:
            validate_webhook_attestation_seed_probe(path)
        handoff_probes = [(label, path) for label, path in probes if label != "webhook_attestation_seed"]
        if handoff_probes and {label for label, _ in handoff_probes} != WEBHOOK_HANDOFF_PROBE_LABELS:
            raise ValueError("webhook sandbox requires the complete host-admission file probe set")
        runtime_dirs = {path.parent for _, path in handoff_probes}
        if len(runtime_dirs) > 1:
            raise ValueError("private webhook handoff files must share one runtime directory")
        if handoff_probes:
            runtime_dir = next(iter(runtime_dirs))
            try:
                canonical_runtime_dir = runtime_dir.resolve(strict=True)
            except OSError:
                raise RuntimeError("private webhook runtime directory is unavailable") from None
            runtime_metadata = runtime_dir.lstat()
            if (
                runtime_dir.name != ".runtime"
                or not runtime_dir.is_relative_to(Path("/tmp"))
                or not runtime_dir.is_absolute()
                or runtime_dir != canonical_runtime_dir
                or not stat.S_ISDIR(runtime_metadata.st_mode)
                or runtime_metadata.st_uid != os.geteuid()
                or stat.S_IMODE(runtime_metadata.st_mode) != 0o700
            ):
                raise RuntimeError("private webhook runtime directory is unsafe")
        for label, path in handoff_probes:
            if path.parent != runtime_dir or not path.is_absolute():
                raise RuntimeError(f"private webhook runtime file path is unsafe: {label}")
            metadata = path.lstat()
            if (
                not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or metadata.st_nlink != 1
                or stat.S_IMODE(metadata.st_mode) != 0o600
                or metadata.st_size > 16 * 1024 * 1024
                or label == "host_admission" and path.name != "host-admission.json"
                or label == "webhook_inbox" and path.name != "forgejo-webhook-inbox.jsonl"
                or label == "admission_source"
                and re.fullmatch(r"mission-host-admission-source-[0-9a-f]{64}\.json", path.name) is None
                or label == "forgejo_read_token" and path.name != "forgejo-read.token"
                or label == "invalid_forgejo_read_token" and path.name != "invalid-forgejo-read.token"
                or label == "mission_journal" and path.name != "mission-execution.jsonl"
            ):
                raise RuntimeError(f"private webhook runtime file is unsafe: {label}")
    if not BWRAP.is_file() or not PYTHON.is_file() or ADAPTER.is_symlink() or not ADAPTER.is_file():
        raise RuntimeError("the local Bubblewrap, Python, or test adapter is unavailable")
    command = supervisor_command(
        config_path,
        gate_barrier_ready_file=gate_barrier_ready_file,
        gate_barrier_release_file=gate_barrier_release_file,
    )
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
            private_file_probes=probes,
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
        adapter_deadline = time.monotonic() + ADAPTER_TIMEOUT_SECONDS
        host_observed_process_tree: dict[str, object] | None = None
        if gate_barrier_ready_file is not None and gate_barrier_release_file is not None:
            try:
                _wait_for_gate_barrier_ready(
                    adapter,
                    host,
                    gate_barrier_ready_file,
                    adapter_deadline,
                )
                host_observed_process_tree = supervisor_observe_sandbox_process_tree(adapter.pid)
            except BaseException:
                stop_process_group(adapter)
                stop_process_group(host)
                listener.close()
                raise

        try:
            remaining_adapter_time = adapter_deadline - time.monotonic()
            if remaining_adapter_time <= 0:
                raise subprocess.TimeoutExpired(adapter.args, ADAPTER_TIMEOUT_SECONDS)
            adapter_stderr = adapter.communicate(timeout=remaining_adapter_time)[1]
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
        issue_context_observed = test_adapter_work_item_observation(event)
        checks = event.get("sandbox_checks")
        expected_checks = {
            "credential_visible": False,
            "host_systemd_bus_visible": False,
            "network_connected": False,
            "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
            "open_fds": [0, 1, 2],
        }
        if not isinstance(checks, dict):
            raise RuntimeError("sandbox adapter capability probe was incomplete")
        validate_direct_route_probe_errors(checks.get("direct_forgejo_route_probe_errors"))
        if probes:
            expected_checks["private_runtime_files_not_visible"] = sorted(
                label for label, _ in probes
            )
        probe_target_port = forgejo_probe_port if forgejo_probe_port is not None else loopback_port
        if (
                {
                key: value
                for key, value in checks.items()
                if key not in {
                    "probe_target_port", "network_namespace", "pid_namespace", "environment_keys",
                    "direct_forgejo_route_probe_errors",
                }
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
            "route_probe_specs": [
                {"name": name, "method": method, "path": path}
                for name, method, path in agent_adapter.forgejo_route_probe_specs(
                    owner, repository, pull_number
                )
            ],
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
                "test_adapter_untrusted_work_item_observed": issue_context_observed,
                "sandbox_checks": reported_checks,
                "host_network_namespace": os.readlink("/proc/self/ns/net"),
                "host_pid_namespace": os.readlink("/proc/self/ns/pid"),
                **(
                    {"host_observed_agent_process_tree": host_observed_process_tree}
                    if host_observed_process_tree is not None
                    else {}
                ),
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
    parser.add_argument("--probe-private-file", action="append", nargs=2, default=[], metavar=("LABEL", "PATH"))
    parser.add_argument("--wait-at-gate-barrier", action="store_true")
    parser.add_argument("--gate-barrier-ready-file", type=Path)
    parser.add_argument("--gate-barrier-release-file", type=Path)
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
            gate_barrier_ready_file=args.gate_barrier_ready_file,
            gate_barrier_release_file=args.gate_barrier_release_file,
            private_file_probes=(
                [(label, Path(path)) for label, path in args.probe_private_file]
                if args.probe_private_file
                else None
            ),
        )
    except Exception as error:
        event = {"type": "error", "error": str(error)}
        status = 1
    print(json.dumps(event, sort_keys=True, separators=(",", ":")))
    return status


if __name__ == "__main__":
    raise SystemExit(main())
