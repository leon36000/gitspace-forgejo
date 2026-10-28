#!/usr/bin/env python3
"""Run the admitted Forgejo fixture through the bounded MCP host and test adapter.

The input state is a private output of run_mission_012_live_admission_e2e.py.
Only its non-secret admission DTO and claim journal are handed to the host.
Real provider execution is refused until provider egress and filesystem
isolation can be attested independently of the agent process.
"""

from __future__ import annotations

import argparse
from decimal import Decimal, InvalidOperation
import hashlib
import io
import json
import os
from pathlib import Path
import re
import selectors
import signal
import shutil
import subprocess
import sys
import tarfile
import time

from mission_012_test_agent_adapter import SOURCE_BUG
from mission_012_sandboxed_test_adapter import validate_direct_route_probe_errors
from mission_webhook_handoff import validate_agent_handoff_files
from process_group_lifecycle import (  # noqa: E402
    child_exit_is_waitable,
    finish_exited_process_group,
)
from systemd_process_scope import (  # noqa: E402
    current_attested_process_scope,
    current_attested_process_scope_snapshot,
    ProcessScopeUnavailable,
    finish_process_scope,
    launch_scoped_process,
    prepare_process_scope,
    process_scope_snapshot_matches,
)

EXPECTED_ORIGIN = "http://127.0.0.1:3132"
EXPECTED_OWNER = "forge-admin"
EXPECTED_REPOSITORY = "mission-012-clamp"
EXPECTED_FORGEJO_PROBE_PORT = 3132
EXPECTED_PULL_REQUEST_NUMBER = 2
EXPECTED_BRANCH = "main"
EXPECTED_REQUIREMENT_SHA256 = "3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431"
EXPECTED_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    "README.md",
    "src/lib.rs",
    "tests/acceptance.rs",
    "requirements/MISSION-012-CLAMP-001.json",
}
MAX_AGENT_OUTPUT_BYTES = 1024 * 1024
REAL_PROVIDER_EXECUTION_REFUSAL = (
    "Claude Code execution is disabled: provider egress and filesystem isolation are not attested"
)


def validate_sandbox_route_probe_observations(value: object) -> None:
    """Require trusted diagnostics before treating direct Forgejo routes as blocked."""
    if not isinstance(value, dict):
        raise RuntimeError("sandboxed test adapter omitted its network observations")
    validate_direct_route_probe_errors(value.get("direct_forgejo_route_probe_errors"))


def validate_sandbox_agent_observations(
    response: dict[str, object],
    admission_source_binding: object,
) -> dict[str, object]:
    """Require current namespace, route, credential, and host-bus refusal evidence."""
    sandbox_checks = response.get("sandbox_checks")
    validate_sandbox_route_probe_observations(sandbox_checks)
    assert isinstance(sandbox_checks, dict)
    expected_sandbox_checks: dict[str, object] = {
        "credential_visible": False,
        "network_connected": False,
        "host_systemd_bus_visible": False,
        "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
        "open_fds": [0, 1, 2],
        "route_probe_specs": [
            {
                "name": "api",
                "method": "GET",
                "path": f"/api/v1/repos/{EXPECTED_OWNER}/{EXPECTED_REPOSITORY}/pulls/{EXPECTED_PULL_REQUEST_NUMBER}",
            },
            {
                "name": "web",
                "method": "GET",
                "path": f"/{EXPECTED_OWNER}/{EXPECTED_REPOSITORY}/pulls/{EXPECTED_PULL_REQUEST_NUMBER}",
            },
            {
                "name": "git_receive_pack",
                "method": "POST",
                "path": f"/{EXPECTED_OWNER}/{EXPECTED_REPOSITORY}.git/git-receive-pack",
            },
        ],
        "supervisor_observed_route_connections": 0,
    }
    if admission_source_binding is not None:
        expected_sandbox_checks["private_runtime_files_not_visible"] = [
            "admission_source",
            "forgejo_read_token",
            "host_admission",
            "invalid_forgejo_read_token",
            "mission_journal",
            "webhook_inbox",
        ]
    extra_check_fields = {
        "probe_target_port",
        "network_namespace",
        "pid_namespace",
        "environment_keys",
        "route_probe_specs",
        "supervisor_observed_route_connections",
        "private_runtime_files_not_visible",
        "direct_forgejo_route_probe_errors",
    }
    if (
        {
            key: value
            for key, value in sandbox_checks.items()
            if key not in extra_check_fields
        }
        != {
            key: value
            for key, value in expected_sandbox_checks.items()
            if key not in extra_check_fields
        }
        or sandbox_checks.get("route_probe_specs")
        != expected_sandbox_checks["route_probe_specs"]
        or sandbox_checks.get("probe_target_port") != EXPECTED_FORGEJO_PROBE_PORT
        or not isinstance(sandbox_checks.get("network_namespace"), str)
        or sandbox_checks.get("network_namespace")
        == response.get("host_network_namespace")
        or not isinstance(sandbox_checks.get("pid_namespace"), str)
        or sandbox_checks.get("pid_namespace") == response.get("host_pid_namespace")
        or sandbox_checks.get("environment_keys")
        != ["HOME", "LANG", "LC_ALL", "PATH", "PWD", "PYTHONDONTWRITEBYTECODE"]
        or sandbox_checks.get("private_runtime_files_not_visible")
        != expected_sandbox_checks.get("private_runtime_files_not_visible")
    ):
        raise RuntimeError("sandboxed test adapter did not prove the credential and network refusals")
    return sandbox_checks


def validate_host_observed_agent_process_tree(
    response: dict[str, object], sandbox_checks: dict[str, object]
) -> dict[str, object]:
    """Require host-procfs namespace and FD observations for a gated sandbox run."""
    observation = response.get("host_observed_agent_process_tree")
    expected_fields = {
        "schema_version",
        "source",
        "host_pid_namespace",
        "host_network_namespace",
        "host_mount_namespace",
        "host_user_namespace",
        "sandbox_pid_namespace",
        "sandbox_network_namespace",
        "sandbox_mount_namespace",
        "sandbox_user_namespace",
        "all_descendants_share_root_cgroup",
        "sandbox_descendant_count",
        "agent_python_process_count",
        "agent_python_open_fds",
    }
    if not isinstance(observation, dict) or set(observation) != expected_fields:
        raise RuntimeError("gated sandbox result omitted its parent process observation")
    host_pid_namespace = response.get("host_pid_namespace")
    host_network_namespace = response.get("host_network_namespace")
    sandbox_pid_namespace = sandbox_checks.get("pid_namespace")
    sandbox_network_namespace = sandbox_checks.get("network_namespace")
    if (
        observation.get("schema_version") != 1
        or observation.get("source") != "parent_procfs"
        or not isinstance(host_pid_namespace, str)
        or not isinstance(host_network_namespace, str)
        or not isinstance(sandbox_pid_namespace, str)
        or not isinstance(sandbox_network_namespace, str)
        or observation.get("host_pid_namespace") != host_pid_namespace
        or observation.get("host_network_namespace") != host_network_namespace
        or observation.get("sandbox_pid_namespace") != sandbox_pid_namespace
        or observation.get("sandbox_network_namespace") != sandbox_network_namespace
        or host_pid_namespace == sandbox_pid_namespace
        or host_network_namespace == sandbox_network_namespace
        or not isinstance(observation.get("host_mount_namespace"), str)
        or not isinstance(observation.get("sandbox_mount_namespace"), str)
        or observation.get("host_mount_namespace") == observation.get("sandbox_mount_namespace")
        or not isinstance(observation.get("host_user_namespace"), str)
        or not isinstance(observation.get("sandbox_user_namespace"), str)
        or observation.get("sandbox_user_namespace") == observation.get("host_user_namespace")
        or observation.get("all_descendants_share_root_cgroup") is not True
    ):
        raise RuntimeError("parent process observation conflicts with sandbox namespace observations")
    for field in ("sandbox_descendant_count", "agent_python_process_count"):
        count = observation.get(field)
        if not isinstance(count, int) or isinstance(count, bool) or count < 1:
            raise RuntimeError("parent process observation has no stable sandbox descendants")
    if (
        observation.get("agent_python_process_count") != 1
        or observation.get("agent_python_open_fds") != [0, 1, 2]
    ):
        raise RuntimeError("parent process observation found an unexpected agent descriptor set")
    return observation


def safe_local_adapter_failure_detail(agent_adapter: str, output: bytes) -> str | None:
    """Extract one bounded, redacted error field from the deterministic adapter wrapper."""
    if agent_adapter != "local-test-adapter":
        return None
    try:
        value = json.loads(output)
    except (json.JSONDecodeError, UnicodeDecodeError):
        return None
    if not isinstance(value, dict) or value.get("type") != "error":
        return None
    detail = value.get("error")
    if not isinstance(detail, str) or not detail:
        return None
    detail = re.sub(r"[\x00-\x1f\x7f]+", " ", detail)
    detail = re.sub(
        r"(?i)(token|secret|credential|password|api[_-]?key)(\s*[=:]\s*)[^\s,;]+",
        r"\1\2[redacted]",
        detail,
    )
    detail = re.sub(r"\b[A-Za-z0-9_-]{32,}\b", "[redacted-long-value]", detail)
    return detail[:512]


def private_file(path: Path, data: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def _terminate_cli_group(process: subprocess.Popen[bytes]) -> str:
    """Stop Claude and its relay group, allowing bounded MCP host finalization first."""
    if child_exit_is_waitable(process):
        return finish_exited_process_group(process, grace_seconds=18)
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    # The MCP supervisor handles this signal by closing host stdin and waiting
    # for its separately grouped host to write its final receipt (up to 15 s).
    deadline = time.monotonic() + 18
    while time.monotonic() < deadline:
        if child_exit_is_waitable(process):
            return finish_exited_process_group(process, grace_seconds=18)
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.025)
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline and not child_exit_is_waitable(process):
        time.sleep(0.025)
    if child_exit_is_waitable(process):
        return finish_exited_process_group(process, grace_seconds=0)
    return "unconfirmed"


def run_bounded_cli(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    timeout: float,
    byte_limit: int = MAX_AGENT_OUTPUT_BYTES,
    reuse_attested_parent_scope: bool = False,
) -> tuple[int, bytes, bytes, bool, bool]:
    """Run the external CLI only when its owning stage is in an attested cgroup scope."""
    if reuse_attested_parent_scope:
        parent_snapshot = current_attested_process_scope_snapshot()
        if parent_snapshot is None:
            return (
                125,
                b"",
                b"refusing CLI start without an observable attested parent cgroup scope\n",
                False,
                False,
            )
        return _run_bounded_cli_process_group(
            command,
            cwd=cwd,
            env=env,
            timeout=timeout,
            byte_limit=byte_limit,
            parent_scope_snapshot=parent_snapshot,
        )
    owner_scope = current_attested_process_scope()
    if owner_scope is None:
        return (
            125,
            b"",
            b"refusing CLI start without an attested systemd process scope\n",
            False,
            False,
        )
    try:
        scope = prepare_process_scope(command, env, timeout, owner_unit=owner_scope.unit)
    except ProcessScopeUnavailable:
        return (
            125,
            b"",
            b"refusing CLI start because a nested systemd process scope is unavailable\n",
            False,
            False,
        )
    return _run_bounded_cli_process_group(
        command,
        cwd=cwd,
        env=env,
        timeout=timeout,
        byte_limit=byte_limit,
        process_scope=scope,
    )


def _run_bounded_cli_process_group(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    timeout: float,
    byte_limit: int = MAX_AGENT_OUTPUT_BYTES,
    process_scope=None,
    parent_scope_snapshot=None,
) -> tuple[int, bytes, bytes, bool, bool]:
    """Low-level capture primitive; production callers must pass the public scope guard."""
    if timeout <= 0 or byte_limit <= 0:
        raise ValueError("Claude capture timeout and byte limit must be positive")
    captures = {"stdout": bytearray(), "stderr": bytearray()}
    deadline = time.monotonic() + timeout
    captured_bytes = 0
    timed_out = False
    exceeded = False
    cancelled_status: int | None = None
    cancelled_signal: int | None = None
    previous_sigterm = signal.getsignal(signal.SIGTERM)
    previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM})
    group_cleanup: str | None = None
    scope_cleanup: str | None = None
    scope_cgroup: str | None = None

    def parent_cancelled(signum: int, _frame: object) -> None:
        # Never raise asynchronously: Python can deliver this between any two
        # bytecodes, including inside finally while cleanup is in progress.
        nonlocal cancelled_signal
        cancelled_signal = signum

    signal.signal(signal.SIGTERM, parent_cancelled)
    process: subprocess.Popen[bytes] | None = None
    selector: selectors.BaseSelector | None = None
    try:
        if process_scope is None:
            # Private process-group-only path retained for synthetic unit tests.
            child_unblock = (
                "import os,signal,sys; "
                "signal.pthread_sigmask(signal.SIG_UNBLOCK,{signal.SIGTERM}); "
                "os.execvpe(sys.argv[1],sys.argv[1:],os.environ)"
            )
            process = subprocess.Popen(
                [sys.executable, "-c", child_unblock, *command],
                cwd=cwd,
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                bufsize=0,
                close_fds=True,
                start_new_session=True,
            )
        else:
            started = launch_scoped_process(
                process_scope,
                command,
                env,
                cwd=cwd,
                timeout=timeout,
                merge_stderr=False,
            )
            process = started.process
            scope_cgroup = started.cgroup_path
            if process is None or scope_cgroup is None:
                return (
                    125,
                    started.startup_stdout,
                    started.startup_stderr + b"refusing CLI start: nested cgroup scope could not be attested\n",
                    False,
                    False,
                )
        assert process.stdout is not None and process.stderr is not None
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        # The caller may have entered with SIGTERM blocked. Keep the temporary
        # cancellation handler reachable until cleanup has completed.
        signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGTERM})
        while selector.get_map():
            if cancelled_signal is not None:
                timed_out = True
                cancelled_status = 128 + cancelled_signal
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                break
            for key, _ in selector.select(min(remaining, 0.25)):
                if cancelled_signal is not None:
                    timed_out = True
                    cancelled_status = 128 + cancelled_signal
                    break
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                available = byte_limit - captured_bytes
                accepted = chunk[:available]
                captures[key.data].extend(accepted)
                captured_bytes += len(accepted)
                if len(accepted) != len(chunk):
                    exceeded = True
                    break
            if exceeded:
                break
            if cancelled_signal is not None:
                timed_out = True
                cancelled_status = 128 + cancelled_signal
                break
        if not timed_out and not exceeded and not child_exit_is_waitable(process):
            while not child_exit_is_waitable(process):
                if cancelled_signal is not None:
                    timed_out = True
                    cancelled_status = 128 + cancelled_signal
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    break
                time.sleep(min(remaining, 0.25))
    finally:
        try:
            # Drain any same-group helpers even after normal CLI exit. MCP hosts
            # are isolated in their own group and finalize through the relay.
            if process is not None:
                group_cleanup = _terminate_cli_group(process)
                if process_scope is not None and scope_cgroup is not None:
                    scope_cleanup = finish_process_scope(process_scope, scope_cgroup)
                elif parent_scope_snapshot is not None:
                    scope_cleanup = (
                        "clean"
                        if process_scope_snapshot_matches(parent_scope_snapshot)
                        else "parent_membership_changed"
                    )
        finally:
            # Keep the non-throwing handler active through group cleanup. A
            # signal delivered anywhere in this finally block only records
            # cancellation; it cannot unwind resource or process cleanup.
            if cancelled_signal is not None:
                timed_out = True
                cancelled_status = 128 + cancelled_signal
            try:
                if selector is not None:
                    selector.close()
            finally:
                try:
                    if process is not None:
                        for stream in (process.stdout, process.stderr):
                            if stream is not None:
                                try:
                                    stream.close()
                                except OSError:
                                    pass
                finally:
                    try:
                        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM})
                    finally:
                        try:
                            if cancelled_signal is not None:
                                timed_out = True
                                cancelled_status = 128 + cancelled_signal
                            signal.signal(signal.SIGTERM, previous_sigterm)
                        finally:
                            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
    assert process is not None
    if cancelled_status is not None:
        status = cancelled_status
    elif exceeded:
        status = 125
    elif timed_out:
        status = 124
    elif (
        group_cleanup != "clean"
        or (process_scope is not None and scope_cleanup != "clean")
        or (parent_scope_snapshot is not None and scope_cleanup != "clean")
    ):
        status = 125
    else:
        status = process.returncode
    if parent_scope_snapshot is not None and scope_cleanup != "clean":
        captures["stderr"].extend(
            b"refusing adapter success because parent cgroup membership changed during cleanup\n"
        )
    return status if status is not None else 124, bytes(captures["stdout"]), bytes(captures["stderr"]), timed_out, exceeded


def git(args: list[str], *, timeout: int = 30) -> bytes:
    env = {
        "PATH": "/usr/bin:/bin",
        "HOME": "/nonexistent",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
    }
    result = subprocess.run(
        ["/usr/bin/git", *args],
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"trusted local Git operation refused (exit {result.returncode})")
    return result.stdout


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def validate_private_file(path: Path) -> None:
    metadata = path.lstat()
    if (
        path.is_symlink()
        or not path.is_file()
        or metadata.st_uid != os.geteuid()
        or metadata.st_nlink != 1
        or metadata.st_mode & 0o7777 != 0o600
    ):
        raise RuntimeError(f"host admission state is not a private regular file: {path.name}")


def trusted_webhook_attestation_public_keys(
    state: Path, *, webhook_delivery: bool
) -> list[str]:
    """Load the host-owned receiver trust anchor, never from agent admission data."""
    if type(webhook_delivery) is not bool:
        raise ValueError("webhook delivery mode must be an explicit boolean")
    if not webhook_delivery:
        return []
    path = state / "mission-admission-attestation.public"
    validate_private_file(path)
    raw = path.read_bytes()
    try:
        key = raw.decode("ascii", errors="strict")
    except UnicodeDecodeError:
        raise RuntimeError("receiver attestation public key is not canonical Ed25519 public key hex") from None
    if (
        len(key) != 64
        or any(character not in "0123456789abcdef" for character in key)
    ):
        raise RuntimeError("receiver attestation public key is not canonical Ed25519 public key hex")
    return [key]


def validate_test_adapter_work_item_observation(
    response: object,
    *,
    expected: bool,
) -> bool:
    if type(expected) is not bool or not isinstance(response, dict):
        raise RuntimeError("test-adapter observation does not match the admitted issue context")
    observed = response.get("test_adapter_untrusted_work_item_observed")
    if type(observed) is not bool or observed is not expected:
        raise RuntimeError("test-adapter observation does not match the admitted issue context")
    return observed


def materialize_snapshot(git_dir: Path, commit: str, destination: Path) -> None:
    destination.mkdir(mode=0o700)
    archive = subprocess.run(
        ["/usr/bin/git", "-C", str(git_dir), "archive", commit],
        env={
            "PATH": "/usr/bin:/bin",
            "HOME": "/nonexistent",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
        },
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=20,
        check=False,
    )
    if archive.returncode or len(archive.stdout) > 2 * 1024 * 1024:
        raise RuntimeError("could not read a bounded archive from the admitted Git commit")
    with tarfile.open(fileobj=io.BytesIO(archive.stdout), mode="r:") as tar:
        members = tar.getmembers()
        files = {member.name for member in members if member.isfile()}
        directories = {member.name for member in members if member.isdir()}
        if (
            files != EXPECTED_FILES
            or directories != {"requirements", "src", "tests"}
            or any(member.issym() or member.islnk() or not (member.isfile() or member.isdir()) for member in members)
            or any(member.size > 1024 * 1024 for member in members)
        ):
            raise RuntimeError("admitted repository tree differs from the bounded mission fixture")
        tar.extractall(destination, filter="data")


def build_agent_command(
    agent_adapter: str,
    mcp_config: Path,
    budget_cap: Decimal,
    prompt: str,
    *,
    capture_test_host_stderr: bool = False,
    sandbox_test_adapter: bool = False,
    wait_at_gate_barrier: bool = False,
    gate_barrier_ready_file: Path | None = None,
    gate_barrier_release_file: Path | None = None,
    private_file_probes: list[tuple[str, Path]] | None = None,
) -> list[str]:
    barrier_paths_supplied = gate_barrier_ready_file is not None or gate_barrier_release_file is not None
    if (gate_barrier_ready_file is None) != (gate_barrier_release_file is None):
        raise RuntimeError("gate barrier ready and release paths must be supplied together")
    if (wait_at_gate_barrier or barrier_paths_supplied) and (
        agent_adapter != "local-test-adapter" or not sandbox_test_adapter
    ):
        raise RuntimeError("gate barrier is only available for the sandboxed local-test-adapter")
    if barrier_paths_supplied and not wait_at_gate_barrier:
        raise RuntimeError("gate barrier paths require the post-acceptance adapter wait")
    if agent_adapter == "local-test-adapter":
        adapter_name = (
            "mission_012_sandboxed_test_adapter.py"
            if sandbox_test_adapter
            else "mission_012_test_agent_adapter.py"
        )
        adapter = Path(__file__).with_name(adapter_name)
        if adapter.is_symlink() or not adapter.is_file():
            raise RuntimeError("the explicit local MISSION-012 test adapter is unavailable")
        command = [sys.executable, str(adapter), "--mcp-config", str(mcp_config)]
        if capture_test_host_stderr:
            command.append("--capture-host-stderr")
        if sandbox_test_adapter:
            command.extend(
                [
                    "--forgejo-probe-port",
                    str(EXPECTED_FORGEJO_PROBE_PORT),
                    "--probe-owner",
                    EXPECTED_OWNER,
                    "--probe-repository",
                    EXPECTED_REPOSITORY,
                    "--probe-pull-number",
                    str(EXPECTED_PULL_REQUEST_NUMBER),
                ]
            )
            for label, path in private_file_probes or []:
                command.extend(["--probe-private-file", label, str(path)])
            if wait_at_gate_barrier:
                command.append("--wait-at-gate-barrier")
            if gate_barrier_ready_file is not None and gate_barrier_release_file is not None:
                command.extend([
                    "--gate-barrier-ready-file", str(gate_barrier_ready_file),
                    "--gate-barrier-release-file", str(gate_barrier_release_file),
                ])
        return command
    if sandbox_test_adapter:
        raise RuntimeError("sandbox mode is only available for local-test-adapter")
    if agent_adapter != "claude":
        raise RuntimeError("agent adapter must be claude or the explicit local-test-adapter")
    raise RuntimeError(REAL_PROVIDER_EXECUTION_REFUSAL)


def mcp_server_config(supervisor: Path, host: Path, host_config: Path) -> dict[str, object]:
    return {
        "mcpServers": {
            "mission012": {
                "command": "/usr/bin/python3",
                "args": [
                    str(supervisor),
                    "--",
                    str(host),
                    "--config",
                    str(host_config),
                ],
                "env": {},
            }
        }
    }


def journal_records(path: Path) -> list[dict[str, object]]:
    records = []
    for raw in path.read_bytes().splitlines():
        if not raw:
            continue
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise RuntimeError("execution journal contains a non-object record")
        records.append(value)
    return records


def validate_test_crash_host(state: Path, evidence_dir: Path) -> tuple[Path, str]:
    evidence_root = (Path(__file__).resolve().parents[1] / "evidence").resolve(strict=True)
    if (
        evidence_dir.is_symlink()
        or evidence_dir.resolve(strict=True).parent != evidence_root
        or evidence_dir.name.endswith("-admission") is False
    ):
        raise RuntimeError("crash-host provenance must be a direct local admission evidence directory")
    provenance_path = evidence_dir / "provenance.json"
    metadata = provenance_path.lstat()
    if provenance_path.is_symlink() or not provenance_path.is_file() or metadata.st_mode & 0o7777 != 0o600:
        raise RuntimeError("crash-host provenance file has unsafe metadata")
    provenance = json.loads(provenance_path.read_bytes())
    hashes = provenance.get("built_executable_sha256") if isinstance(provenance, dict) else None
    expected = hashes.get("mission-012-mcp-host-test-crash") if isinstance(hashes, dict) else None
    selected = hashes.get("mission-012-mcp-host") if isinstance(hashes, dict) else None
    if (
        not isinstance(provenance, dict)
        or provenance.get("test_crash_injection_host_built") is not True
        or not isinstance(expected, str)
        or expected != selected
        or len(expected) != 64
        or any(character not in "0123456789abcdef" for character in expected)
    ):
        raise RuntimeError("admission provenance does not bind the debug crash-injection host")
    host = state / "build/host-artifacts/mission-012-mcp-host"
    test_host = state / "build/host-artifacts/mission-012-mcp-host-test-crash"
    for path in (host, test_host):
        info = path.lstat()
        if path.is_symlink() or not path.is_file() or info.st_mode & 0o7777 != 0o700:
            raise RuntimeError("built crash-injection host artifact has unsafe metadata")
    if sha256(host) != expected or sha256(test_host) != expected:
        raise RuntimeError("crash-injection host artifact differs from admission build provenance")
    return host, expected


def run(args: argparse.Namespace) -> dict[str, object]:
    if getattr(args, "agent_adapter", None) == "claude":
        raise RuntimeError(REAL_PROVIDER_EXECUTION_REFUSAL)
    state = Path(args.state_dir).resolve(strict=True)
    test_host_restart = bool(getattr(args, "test_host_restart", False))
    sandbox_test_adapter = bool(getattr(args, "sandbox_test_adapter", False))
    reuse_attested_parent_scope = bool(
        getattr(args, "reuse_attested_parent_scope", False)
    )
    gate_barrier_ready_file = getattr(args, "gate_barrier_ready_file", None)
    gate_barrier_release_file = getattr(args, "gate_barrier_release_file", None)
    if (gate_barrier_ready_file is None) != (gate_barrier_release_file is None):
        raise RuntimeError("gate barrier ready and release paths must be supplied together")
    gate_barrier_enabled = gate_barrier_ready_file is not None
    if gate_barrier_enabled and (
        args.agent_adapter != "local-test-adapter"
        or not sandbox_test_adapter
        or test_host_restart
    ):
        raise RuntimeError("gate barrier is restricted to the non-restarting sandboxed local-test-adapter")
    if sandbox_test_adapter and args.agent_adapter != "local-test-adapter":
        raise RuntimeError("sandbox mode is only available for local-test-adapter")
    if reuse_attested_parent_scope and (
        args.agent_adapter != "local-test-adapter"
        or not sandbox_test_adapter
        or current_attested_process_scope_snapshot() is None
    ):
        raise RuntimeError("parent-scope reuse requires the sandboxed local-test-adapter in an attested scope")
    if test_host_restart and args.agent_adapter != "local-test-adapter":
        raise RuntimeError("test host restart is restricted to the explicit local-test-adapter")
    if test_host_restart and not getattr(args, "admission_evidence_dir", None):
        raise RuntimeError("test host restart requires its admission build provenance")
    project = state / "project"
    runtime = project / ".runtime"
    admission_file = Path(args.admission_file) if args.admission_file else runtime / "host-admission.json"
    journal = Path(args.journal_file) if args.journal_file else runtime / "mission-execution.jsonl"
    token_file = runtime / "forgejo-read.token"
    try:
        budget_cap = Decimal(args.budget_cap_usd)
    except InvalidOperation:
        raise RuntimeError("budget cap must be a decimal amount no higher than USD 0.20") from None
    if not budget_cap.is_finite() or budget_cap <= 0 or budget_cap > Decimal("0.20"):
        raise RuntimeError("budget cap must be a positive decimal amount no higher than USD 0.20")
    if (
        not admission_file.is_absolute()
        or not journal.is_absolute()
        or admission_file.parent != runtime
        or journal.parent != runtime
    ):
        raise RuntimeError("admission DTO and execution journal must remain in the project .runtime directory")
    for path in (admission_file, journal, token_file):
        validate_private_file(path)
    if state.parent != Path("/tmp") or state.stat().st_uid != os.geteuid() or state.stat().st_mode & 0o777 != 0o700:
        raise RuntimeError("admission state must be an owner-only direct child of /tmp")
    dto = json.loads(admission_file.read_bytes())
    if (
        dto.get("schema_version") != 1
        or dto.get("scope") != {
            "origin": EXPECTED_ORIGIN,
            "owner": EXPECTED_OWNER,
            "repository": EXPECTED_REPOSITORY,
            "branch": EXPECTED_BRANCH,
        }
        or dto.get("issue_number") != 1
        or dto.get("requirement_sha256") != EXPECTED_REQUIREMENT_SHA256
        or type(dto.get("allocation", {}).get("execution_generation")) is not int
        or dto.get("allocation", {}).get("execution_generation", 0) < 1
        or dto.get("request", {}).get("capabilities", {}).get("write_paths") != ["src/lib.rs"]
    ):
        raise RuntimeError("admission DTO does not match the pinned MISSION-012 profile")

    webhook_inbox_argument = getattr(args, "webhook_inbox_file", None)
    admission_source_argument = getattr(args, "admission_source_file", None)
    if (webhook_inbox_argument is None) != (admission_source_argument is None):
        raise RuntimeError("webhook inbox and admission source receipt must be supplied together")
    admission_source_binding: dict[str, str] | None = None
    webhook_inbox_file: Path | None = None
    admission_source_file: Path | None = None
    if webhook_inbox_argument is not None and admission_source_argument is not None:
        webhook_inbox_file = Path(webhook_inbox_argument)
        admission_source_file = Path(admission_source_argument)
        admission_source_binding = validate_agent_handoff_files(
            runtime,
            admission_file,
            webhook_inbox_file,
            admission_source_file,
            dto,
        )

    base_commit = dto.get("base_commit")
    if not isinstance(base_commit, str) or len(base_commit) != 40:
        raise RuntimeError("admission DTO has no full Git base commit")
    forgejo_git = state / "repositories" / EXPECTED_OWNER / f"{EXPECTED_REPOSITORY}.git"
    if forgejo_git.is_symlink() or not forgejo_git.is_dir():
        raise RuntimeError("private Forgejo test repository is unavailable")

    if not re.fullmatch(r"live-mcp-[0-9]{3}", args.execution_id):
        raise RuntimeError("execution ID must match the private live-mcp-NNN format")
    execution_root = runtime / args.execution_id
    execution_root.mkdir(mode=0o700)
    # The MCP host invokes `git -C <trusted_repo>`, so this must be a trusted
    # working clone rather than a bare mirror. Keep it separate from the
    # candidate workspace, which is writable by the model through MCP.
    trusted_git = execution_root / "trusted-repository"
    git(["clone", "--", str(forgejo_git), str(trusted_git)], timeout=60)
    head = git(["-C", str(trusted_git), "rev-parse", f"refs/heads/{EXPECTED_BRANCH}"]).decode().strip()
    if head != base_commit:
        raise RuntimeError("trusted Forgejo Git head differs from the admitted base commit")

    workspace_root = execution_root / "workspace"
    workspace_root.mkdir(mode=0o700)
    base_dir = workspace_root / "base"
    candidate_dir = workspace_root / "candidate"
    target_dir = workspace_root / "target"
    materialize_snapshot(trusted_git, base_commit, base_dir)
    shutil.copytree(base_dir, candidate_dir)
    target_dir.mkdir(mode=0o700)

    sysroot = Path("/home/pc6/.rustup/toolchains/1.95.0-aarch64-unknown-linux-gnu").resolve(strict=True)
    host = state / "build/host-artifacts/mission-012-mcp-host"
    if host.is_symlink() or not host.is_file():
        raise RuntimeError("the isolated current MISSION-012 MCP host binary is missing")
    supervisor_path = Path(__file__).with_name("mission_012_mcp_supervisor.py")
    if supervisor_path.is_symlink() or not supervisor_path.is_file():
        raise RuntimeError("the local MCP shutdown supervisor is unavailable")
    supervisor = supervisor_path.resolve(strict=True)
    if test_host_restart:
        host, host_hash = validate_test_crash_host(
            state, Path(args.admission_evidence_dir)
        )
    else:
        host_hash = sha256(host)
    host_config = execution_root / "host-config.json"
    receipt_file = execution_root / "host-receipts.jsonl"
    initial_receipt_file = (
        execution_root / "host-receipts-before-restart.jsonl"
        if test_host_restart
        else receipt_file
    )
    config = {
        "schema_version": 5,
        "admission_source_mode": (
            "webhook_delivery" if admission_source_binding is not None else "direct_local"
        ),
        "webhook_inbox_file": (
            str(webhook_inbox_file) if webhook_inbox_file is not None else None
        ),
        "admission_source_file": (
            str(admission_source_file) if admission_source_file is not None else None
        ),
        "trusted_webhook_attestation_public_keys_hex": trusted_webhook_attestation_public_keys(
            state, webhook_delivery=admission_source_binding is not None
        ),
        "workspace": {
            "base_dir": str(base_dir),
            "candidate_dir": str(candidate_dir),
            "target_dir": str(target_dir),
            "rust_sysroot": str(sysroot),
            "trusted_git_dir": str(trusted_git),
        },
        "receipt_file": str(initial_receipt_file),
        "journal_file": str(journal),
        "admission_file": str(admission_file),
        "lease_id": f"mission012-{args.execution_id}",
        "holder_id": "mission012-host-001",
        "lease_ttl_ms": 10_000 if test_host_restart else 300000,
    }
    if test_host_restart:
        config["workspace_write_crash_point_for_test"] = "after_effect"
    private_file(host_config, json.dumps(config, sort_keys=True, separators=(",", ":")).encode() + b"\n")
    mcp_config = execution_root / "claude-mcp.json"
    private_file(
        mcp_config,
        json.dumps(
            mcp_server_config(supervisor, host, host_config),
            sort_keys=True,
            separators=(",", ":"),
        ).encode() + b"\n",
    )

    prompt = (
        "Tu es l’agent d’exécution de la mission MISSION-012. Utilise uniquement les outils MCP "
        "mission012 exposés; les outils intégrés et l’accès au système sont désactivés. "
        "Appelle mission_context pour consommer la demande et le contexte admis. Lis les fichiers "
        "autorisés nécessaires. Lance cargo_test avant toute modification, identifie l’assertion "
        "rouge et sa cause. Corrige ensuite uniquement src/lib.rs pour que les valeurs au-dessus "
        "de maximum retournent maximum, tout en préservant les autres cas. Relance cargo_test "
        "jusqu’à réussite. N’essaie pas de lire les credentials hôte, d’écrire les tests ou les "
        "critères, d’appeler le réseau, d’utiliser une autre capacité ou de fusionner. Termine "
        "par un résumé factuel du défaut, du changement et du résultat."
    )
    start = time.monotonic()
    agent_env = {
        "PATH": f"{Path.home() / '.local/bin'}:/usr/bin:/bin",
        "HOME": str(Path.home()),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
    }
    private_file_probes = (
        [
            ("admission_source", admission_source_file),
            ("forgejo_read_token", runtime / "forgejo-read.token"),
            ("host_admission", admission_file),
            ("invalid_forgejo_read_token", runtime / "invalid-forgejo-read.token"),
            ("mission_journal", journal),
            ("webhook_inbox", webhook_inbox_file),
        ]
        if sandbox_test_adapter
        and admission_source_binding is not None
        and admission_file is not None
        and admission_source_file is not None
        and webhook_inbox_file is not None
        else None
    )
    restart_recovery: dict[str, object] | None = None
    if test_host_restart:
        crash_command = build_agent_command(
            args.agent_adapter,
            mcp_config,
            budget_cap,
            prompt,
            capture_test_host_stderr=True,
            sandbox_test_adapter=sandbox_test_adapter,
            private_file_probes=private_file_probes,
        )
        crash_status, crash_stdout, crash_stderr, crash_timed_out, crash_output_exceeded = (
            run_bounded_cli(
                crash_command,
                cwd=project,
                env=agent_env,
                timeout=600,
                reuse_attested_parent_scope=reuse_attested_parent_scope,
            )
        )
        private_file(execution_root / "local-test-adapter-crash-output.stream.jsonl", crash_stdout)
        private_file(execution_root / "local-test-adapter-crash-stderr.log", crash_stderr)
        if crash_timed_out or crash_output_exceeded or crash_status == 0:
            raise RuntimeError("the injected host SIGKILL did not interrupt the first adapter attempt")

        source_path = base_dir / "src/lib.rs"
        base_source = source_path.read_bytes()
        bug_bytes = SOURCE_BUG.encode("utf-8")
        if base_source.count(bug_bytes) != 1:
            raise RuntimeError("base source differs from the pinned restart-adapter fixture")
        fixed_source = base_source.replace(bug_bytes, b"maximum", 1)
        expected_after_sha256 = hashlib.sha256(fixed_source).hexdigest()
        candidate_source = (candidate_dir / "src/lib.rs").read_bytes()
        if candidate_source != fixed_source:
            raise RuntimeError("after-effect SIGKILL did not leave the exact intended candidate bytes")

        events_before_restart = journal_records(journal)
        intents_before_restart = [
            event for event in events_before_restart
            if event.get("kind") == "mission_tool_operation_intent_v1"
        ]
        results_before_restart = [
            event for event in events_before_restart
            if event.get("kind") == "mission_tool_operation_result_v1"
        ]
        if len(intents_before_restart) != 1 or results_before_restart:
            raise RuntimeError("SIGKILL did not leave exactly one unresolved durable write intent")
        intent_payload = intents_before_restart[0].get("payload")
        if (
            not isinstance(intent_payload, dict)
            or intent_payload.get("path") != "src/lib.rs"
            or intent_payload.get("after_source_sha256") != expected_after_sha256
            or not isinstance(intent_payload.get("fencing_token"), int)
        ):
            raise RuntimeError("unresolved write intent is not bound to the observed candidate")

        lease_events = [
            event for event in events_before_restart
            if event.get("kind") == "mission_execution_lease_acquired_v1"
        ]
        initial_lease_payload = (
            lease_events[0].get("payload") if len(lease_events) == 1 else None
        )
        if not isinstance(initial_lease_payload, dict):
            raise RuntimeError("initial execution lease is missing from the durable journal")
        issued_at = initial_lease_payload.get("issued_at_unix_ms")
        runtime_timeout = initial_lease_payload.get("runtime_timeout_ms")
        if (
            not isinstance(issued_at, int)
            or isinstance(issued_at, bool)
            or not isinstance(runtime_timeout, int)
            or isinstance(runtime_timeout, bool)
            or runtime_timeout <= 0
        ):
            raise RuntimeError("initial execution lease has no valid absolute runtime deadline")

        wait_started = time.monotonic()
        time.sleep(10.25)
        lease_expiry_wait_ms = int((time.monotonic() - wait_started) * 1000)
        remaining_runtime_ms = issued_at + runtime_timeout - time.time_ns() // 1_000_000
        recovery_lease_ttl_ms = min(120_000, remaining_runtime_ms - 1_000)
        if recovery_lease_ttl_ms < 10_000:
            raise RuntimeError(
                "insufficient admitted runtime remains for a bounded host recovery lease"
            )
        recovery_receipt_file = execution_root / "host-receipts-after-restart.jsonl"
        recovery_config = dict(config)
        recovery_config["receipt_file"] = str(recovery_receipt_file)
        recovery_config["lease_id"] = f"mission012-{args.execution_id}-recovery"
        recovery_config["holder_id"] = config["holder_id"]
        recovery_config["lease_ttl_ms"] = recovery_lease_ttl_ms
        recovery_config["workspace_write_crash_point_for_test"] = None
        recovery_host_config = execution_root / "host-config-after-restart.json"
        private_file(
            recovery_host_config,
            json.dumps(recovery_config, sort_keys=True, separators=(",", ":")).encode() + b"\n",
        )
        recovery_mcp_config = execution_root / "mcp-after-restart.json"
        private_file(
            recovery_mcp_config,
            json.dumps(
                mcp_server_config(supervisor, host, recovery_host_config),
                sort_keys=True,
                separators=(",", ":"),
            ).encode() + b"\n",
        )
        recovery_command = build_agent_command(
            args.agent_adapter,
            recovery_mcp_config,
            budget_cap,
            prompt,
            capture_test_host_stderr=True,
            sandbox_test_adapter=sandbox_test_adapter,
            private_file_probes=private_file_probes,
        )
        agent_status, agent_stdout, agent_stderr, agent_timed_out, agent_output_exceeded = (
            run_bounded_cli(
                recovery_command,
                cwd=project,
                env=agent_env,
                timeout=600,
                reuse_attested_parent_scope=reuse_attested_parent_scope,
            )
        )
        private_file(
            execution_root / "local-test-adapter-recovery-output.stream.jsonl",
            agent_stdout,
        )
        private_file(
            execution_root / "local-test-adapter-recovery-stderr.log",
            agent_stderr,
        )
        if agent_timed_out or agent_output_exceeded or agent_status != 0:
            raise RuntimeError(
                "local adapter did not finish the recovered host attempt "
                f"(exit={agent_status}, timeout={agent_timed_out}, "
                f"output_exceeded={agent_output_exceeded})"
            )

        events_after_restart = journal_records(journal)
        intents_after_restart = [
            event for event in events_after_restart
            if event.get("kind") == "mission_tool_operation_intent_v1"
        ]
        results_after_restart = [
            event for event in events_after_restart
            if event.get("kind") == "mission_tool_operation_result_v1"
        ]
        if len(intents_after_restart) != 1 or len(results_after_restart) != 1:
            raise RuntimeError("restart did not resolve the original intent exactly once")
        result_payload = results_after_restart[0].get("payload")
        old_fence = intent_payload["fencing_token"]
        new_fence = result_payload.get("fencing_token") if isinstance(result_payload, dict) else None
        recovery_lease_events = [
            event for event in events_after_restart
            if event.get("kind") == "mission_execution_lease_acquired_v1"
            and isinstance(event.get("payload"), dict)
            and event["payload"].get("fencing_token") == new_fence
        ]
        if (
            not isinstance(result_payload, dict)
            or result_payload.get("operation_id") != intent_payload.get("operation_id")
            or not isinstance(new_fence, int)
            or new_fence <= old_fence
            or len(recovery_lease_events) != 1
            or recovery_lease_events[0]["payload"].get("ttl_ms") != recovery_lease_ttl_ms
        ):
            raise RuntimeError("recovery result is not bound to the unresolved operation under a newer fence")

        if (
            not initial_receipt_file.is_file()
            or not recovery_receipt_file.is_file()
        ):
            raise RuntimeError("one of the host processes produced no durable tool receipts")
        receipt_file = execution_root / "host-receipts.jsonl"
        initial_receipt = initial_receipt_file.read_bytes()
        recovered_receipt = recovery_receipt_file.read_bytes()
        if not initial_receipt or not recovered_receipt:
            raise RuntimeError("restart receipt segment is empty")
        combined_receipts = initial_receipt + recovered_receipt
        private_file(receipt_file, combined_receipts)
        restart_recovery = {
            "crash_point": "after_effect",
            "first_attempt_exit_code": crash_status,
            "first_attempt_timed_out": crash_timed_out,
            "first_attempt_output_exceeded": crash_output_exceeded,
            "unresolved_intents_before_restart": len(intents_before_restart),
            "results_before_restart": len(results_before_restart),
            "candidate_source_sha256_after_effect": expected_after_sha256,
            "operation_id": intent_payload["operation_id"],
            "old_fencing_token": old_fence,
            "new_fencing_token": new_fence,
            "initial_lease_id": config["lease_id"],
            "recovery_lease_id": recovery_config["lease_id"],
            "initial_runtime_timeout_ms": runtime_timeout,
            "remaining_runtime_ms_at_recovery": remaining_runtime_ms,
            "recovery_lease_ttl_ms": recovery_lease_ttl_ms,
            "lease_expiry_wait_ms": lease_expiry_wait_ms,
            "results_after_restart": len(results_after_restart),
            "initial_receipt_sha256": sha256(initial_receipt_file),
            "recovery_receipt_sha256": sha256(recovery_receipt_file),
            "combined_receipt_sha256": sha256(receipt_file),
            "initial_receipt_count": len(
                [line for line in initial_receipt.splitlines() if line]
            ),
            "recovery_receipt_count": len(
                [line for line in recovered_receipt.splitlines() if line]
            ),
        }
    else:
        command = build_agent_command(
            args.agent_adapter,
            mcp_config,
            budget_cap,
            prompt,
            sandbox_test_adapter=sandbox_test_adapter,
            wait_at_gate_barrier=gate_barrier_enabled,
            gate_barrier_ready_file=gate_barrier_ready_file,
            gate_barrier_release_file=gate_barrier_release_file,
            private_file_probes=private_file_probes,
        )
        agent_status, agent_stdout, agent_stderr, agent_timed_out, agent_output_exceeded = (
            run_bounded_cli(
                command,
                cwd=project,
                env=agent_env,
                timeout=600,
                reuse_attested_parent_scope=reuse_attested_parent_scope,
            )
        )
    duration_ms = int((time.monotonic() - start) * 1000)
    agent_label = "Claude Code" if args.agent_adapter == "claude" else "local test adapter"
    private_file(execution_root / f"{args.agent_adapter}-output.stream.jsonl", agent_stdout)
    private_file(execution_root / f"{args.agent_adapter}-stderr.log", agent_stderr)
    if agent_timed_out:
        raise RuntimeError(f"bounded {agent_label} execution timed out after 600 seconds")
    if agent_output_exceeded:
        raise RuntimeError(f"bounded {agent_label} output exceeded the combined 1 MiB capture limit")
    if agent_status:
        detail = safe_local_adapter_failure_detail(args.agent_adapter, agent_stdout)
        suffix = f": {detail}" if detail is not None else ""
        raise RuntimeError(
            f"bounded {agent_label} execution exited with status {agent_status}{suffix}"
        )
    try:
        events = [json.loads(line) for line in agent_stdout.splitlines() if line]
    except json.JSONDecodeError:
        raise RuntimeError(f"{agent_label} output was not bounded JSONL") from None
    result_events = [event for event in events if isinstance(event, dict) and event.get("type") == "result"]
    response = result_events[-1] if result_events else {}
    if response.get("is_error") is True or response.get("subtype") not in (None, "success"):
        raise RuntimeError(f"{agent_label} did not report a successful run")
    admitted_request = dto.get("request")
    if not isinstance(admitted_request, dict):
        raise RuntimeError("admission DTO has no request object")
    test_adapter_issue_context_observed = None
    if args.agent_adapter == "local-test-adapter":
        test_adapter_issue_context_observed = validate_test_adapter_work_item_observation(
            response,
            expected=admitted_request.get("untrusted_work_item") is not None,
        )
    sandbox_checks = None
    host_observed_process_tree = None
    if sandbox_test_adapter:
        sandbox_checks = validate_sandbox_agent_observations(
            response,
            admission_source_binding,
        )
        if gate_barrier_enabled:
            host_observed_process_tree = validate_host_observed_agent_process_tree(
                response,
                sandbox_checks,
            )
    if test_host_restart:
        resume_text = response.get("result")
        if not isinstance(resume_text, str) or "checkpoint `unverified`" not in resume_text:
            raise RuntimeError("restarted adapter did not consume the unverified checkpoint")
        if (candidate_dir / "src/lib.rs").read_bytes() != fixed_source:
            raise RuntimeError("recovered adapter did not restore the exact authorized source change")
        assert restart_recovery is not None
        restart_recovery["resume_checkpoint_status"] = "unverified"

    changed_paths = []
    expected_after = {}
    for path in EXPECTED_FILES:
        before = (base_dir / path).read_bytes()
        after = (candidate_dir / path).read_bytes()
        if before != after:
            changed_paths.append(path)
            expected_after[path] = hashlib.sha256(after).hexdigest()
    if len(changed_paths) != 1 or set(changed_paths) != {"src/lib.rs"}:
        raise RuntimeError("candidate did not change exactly the authorized source path")
    if not receipt_file.is_file():
        raise RuntimeError("MCP host produced no durable tool receipts")
    receipt_lines = receipt_file.read_bytes().splitlines()
    receipts = [json.loads(line) for line in receipt_lines if line]
    calls = [item for item in receipts if item.get("event") == "tool_call"]
    tool_names = sorted({item["tool_id"] for item in calls if isinstance(item.get("tool_id"), str)})
    expected_tools = {"mission.context", "workspace.read_text", "workspace.write_text", "cargo.test"}
    test_calls = [item for item in calls if item.get("tool_id") == "cargo.test"]
    write_indices = [index for index, item in enumerate(calls) if item.get("tool_id") == "workspace.write_text"]
    test_indices = [index for index, item in enumerate(calls) if item.get("tool_id") == "cargo.test"]
    finalization = next(
        (item for item in reversed(receipts) if item.get("event") == "host_finalization"),
        None,
    )
    if (
        not expected_tools.issubset(set(tool_names))
        or len(test_calls) < 2
        or not write_indices
        or not test_indices
        or test_indices[0] >= write_indices[0]
        or test_indices[-1] <= write_indices[-1]
        or finalization is None
        or finalization.get("outcome") != "candidate_ready"
    ):
        raise RuntimeError("host receipts do not show context, test-fix-test, and candidate_ready")
    if admission_source_binding is not None:
        assert webhook_inbox_file is not None and admission_source_file is not None
        final_handoff = validate_agent_handoff_files(
            runtime,
            admission_file,
            webhook_inbox_file,
            admission_source_file,
            dto,
        )
        if final_handoff != admission_source_binding:
            raise RuntimeError("webhook admission source or inbox changed during agent execution")
    result_text = response.get("result")
    if not isinstance(result_text, str):
        result_text = ""
    is_model_run = args.agent_adapter == "claude"
    result = {
        "schema_version": 1,
        "status": "candidate_ready_for_independent_verification",
        "agent_adapter": args.agent_adapter,
        "model": "claude-opus-5-5" if is_model_run else "local-test-adapter",
        "effort": "high" if is_model_run else "deterministic",
        "model_invoked_during_run": is_model_run,
        "test_adapter_untrusted_work_item_observed": test_adapter_issue_context_observed,
        "execution_sandbox": "bubblewrap_stdio_mcp" if sandbox_test_adapter else "host_process_unisolated",
        **({"sandbox_checks": sandbox_checks} if sandbox_checks is not None else {}),
        **(
            {"host_observed_agent_process_tree": host_observed_process_tree}
            if host_observed_process_tree is not None
            else {}
        ),
        "budget_cap_usd": float(budget_cap),
        "execution_generation": dto["allocation"]["execution_generation"],
        "reported_cost_usd": (
            response.get("total_cost_usd", response.get("cost_usd")) if is_model_run else 0.0
        ),
        "input_tokens": response.get("usage", {}).get("input_tokens") if is_model_run and isinstance(response.get("usage"), dict) else None,
        "output_tokens": response.get("usage", {}).get("output_tokens") if is_model_run and isinstance(response.get("usage"), dict) else None,
        "duration_ms": duration_ms,
        "forgejo_scope": dto["scope"],
        "issue_number": dto["issue_number"],
        "base_commit": base_commit,
        "admission_sha256": sha256(admission_file),
        "host_binary_sha256": host_hash,
        "admission_source_mode": (
            "webhook_delivery" if admission_source_binding is not None else "direct_local"
        ),
        "admission_source_delivery_id": (
            admission_source_binding.get("delivery_id")
            if admission_source_binding is not None
            else None
        ),
        "admission_source_sha256": (
            admission_source_binding.get("admission_source_sha256")
            if admission_source_binding is not None
            else None
        ),
        "webhook_inbox_sha256": (
            admission_source_binding.get("webhook_inbox_sha256")
            if admission_source_binding is not None
            else None
        ),
        "receipt_sha256": sha256(receipt_file),
        "receipt_count": len(receipts),
        "tool_names_from_receipts": tool_names,
        "changed_paths": changed_paths,
        "candidate_file_sha256": expected_after,
        "host_receipt_path": str(receipt_file),
        "private_execution_state": str(execution_root),
        "agent_summary": result_text[:2000],
        **({"restart_recovery": restart_recovery} if restart_recovery is not None else {}),
    }
    private_file(execution_root / "agent-run-summary.json", json.dumps(
        result, sort_keys=True, separators=(",", ":")
    ).encode() + b"\n")
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--execution-id", default="live-mcp-005")
    parser.add_argument("--admission-file")
    parser.add_argument("--journal-file")
    parser.add_argument("--webhook-inbox-file")
    parser.add_argument("--admission-source-file")
    parser.add_argument("--gate-barrier-ready-file", type=Path)
    parser.add_argument("--gate-barrier-release-file", type=Path)
    parser.add_argument("--budget-cap-usd", default="0.20")
    parser.add_argument(
        "--agent-adapter",
        choices=("claude", "local-test-adapter"),
        default="claude",
        help="Claude mode fails closed until provider egress and filesystem isolation are attested",
    )
    parser.add_argument(
        "--sandbox-test-adapter",
        action="store_true",
        help="run only the explicit no-LLM adapter in Bubblewrap with MCP over stdio and probe the live Forgejo routes",
    )
    parser.add_argument(
        "--reuse-attested-parent-scope",
        action="store_true",
        help="run the test adapter in its owning outer scope and refuse any residual child process",
    )
    parser.add_argument("--admission-evidence-dir", type=Path)
    parser.add_argument(
        "--test-host-restart",
        action="store_true",
        help="use the debug-only crash host to SIGKILL and resume the explicit local test adapter",
    )
    args = parser.parse_args()
    try:
        result = run(args)
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
