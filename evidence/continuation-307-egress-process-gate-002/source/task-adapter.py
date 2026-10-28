#!/usr/bin/env python3
"""Explicit no-LLM adapter for exercising the MISSION-012 MCP contract.

This deterministic fixture consumes the admitted context, observes the
protected red test, makes the one authorized source edit through MCP, and
observes the green test. It does not call a model or access the workspace
directly.
"""

from __future__ import annotations

import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import sys
import time


MAX_FRAME_BYTES = 128 * 1024
VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS = 76
MCP_TOOLS = {
    "mission_context",
    "workspace_read_text",
    "workspace_write_text",
    "cargo_test",
}
SOURCE_BUG = "minimum // BUG: values above the range should clamp to maximum."
SOURCE_FIX = "maximum"
UNTRUSTED_WORK_ITEM_OBSERVED_MARKER = (
    "contexte d’issue non fiable observé séparément des exigences"
)
UNTRUSTED_WORK_ITEM_FIELDS = {
    "schema_version",
    "source",
    "trust",
    "origin",
    "owner",
    "repository",
    "issue_number",
    "source_url",
    "updated_at",
    "title",
    "body",
    "content_sha256",
}
PRIVATE_WRITE_PROBE_LABELS = {"webhook_inbox"}
PRIVATE_WRITE_PROBE_MARKER = b"FORGEJO_WEBHOOK_INBOX_WRITE_PROBE\n"


def observe_untrusted_work_item(value: object) -> bool:
    """Recognize the bounded issue envelope without using it as a criterion."""
    if value is None:
        return False
    if not isinstance(value, dict) or set(value) != UNTRUSTED_WORK_ITEM_FIELDS:
        raise RuntimeError("untrusted issue context does not match its versioned envelope")
    schema_version = value.get("schema_version")
    if (
        not isinstance(schema_version, int)
        or isinstance(schema_version, bool)
        or schema_version != 1
        or value.get("source") != "current_forgejo_issue_api"
        or value.get("trust") != "untrusted_user_content"
    ):
        raise RuntimeError("untrusted issue context has an unexpected version, source, or trust label")

    origin = value.get("origin")
    owner = value.get("owner")
    repository = value.get("repository")
    issue_number = value.get("issue_number")
    if (
        not isinstance(origin, str)
        or not re.fullmatch(r"https?://[^/\s?#]+", origin)
        or not isinstance(owner, str)
        or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,99}", owner)
        or owner in {".", ".."}
        or not isinstance(repository, str)
        or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,99}", repository)
        or repository in {".", ".."}
        or not isinstance(issue_number, int)
        or isinstance(issue_number, bool)
        or issue_number < 1
    ):
        raise RuntimeError("untrusted issue context has an invalid repository identity")
    expected_source_url = (
        f"{origin}/api/v1/repos/{owner}/{repository}/issues/{issue_number}"
    )
    if value.get("source_url") != expected_source_url:
        raise RuntimeError("untrusted issue context route does not match its repository identity")

    updated_at = value.get("updated_at")
    title = value.get("title")
    if (
        not isinstance(updated_at, str)
        or not updated_at
        or len(updated_at.encode("utf-8")) > 128
        or any(ord(character) < 32 for character in updated_at)
        or not isinstance(title, str)
        or not title.strip()
        or len(title.encode("utf-8")) > 1_024
        or any(ord(character) < 32 for character in title)
    ):
        raise RuntimeError("untrusted issue context timestamp or title is invalid")

    body = value.get("body")
    if not isinstance(body, dict):
        raise RuntimeError("untrusted issue context body has an invalid tagged shape")
    if body == {"kind": "explicit_null"}:
        pass
    elif set(body) == {"kind", "text"} and body.get("kind") == "text":
        body_text = body.get("text")
        if (
            not isinstance(body_text, str)
            or len(body_text.encode("utf-8")) > 4_096
            or any(
                ord(character) < 32 and character not in "\n\r\t"
                for character in body_text
            )
        ):
            raise RuntimeError("untrusted issue context body is invalid or exceeds its bound")
    else:
        raise RuntimeError("untrusted issue context body has an invalid tagged shape")

    digest = value.get("content_sha256")
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise RuntimeError("untrusted issue context digest is malformed")
    return True


def supervisor_shutdown_error(exit_code: int, operation: str) -> str:
    if exit_code == VERIFIER_KILL_REQUEST_FAILED_EXIT_STATUS:
        return (
            "MCP supervisor verifier-scope cleanup request failed (exit 76); "
            f"{operation} is inconclusive"
        )
    return f"{operation} (MCP supervisor exit {exit_code})"


class MCPClient:
    def __init__(
        self,
        config_path: Path | None = None,
        *,
        capture_host_stderr: bool = False,
        stdio_mcp: bool = False,
    ) -> None:
        self.stdio_mcp = stdio_mcp
        self.process: subprocess.Popen[bytes] | None = None
        if stdio_mcp:
            if config_path is not None:
                raise RuntimeError("stdio MCP mode must not receive a host configuration path")
            self.read_fd = sys.stdin.fileno()
            self.write_fd = sys.stdout.fileno()
            self.next_id = 1
            self.buffer = bytearray()
            return
        if config_path is None:
            raise RuntimeError("the trusted MCP host configuration is required outside stdio mode")
        metadata = config_path.lstat()
        if config_path.is_symlink() or not config_path.is_file() or metadata.st_mode & 0o7777 != 0o600:
            raise RuntimeError("MCP config must be a private regular file")
        config = json.loads(config_path.read_bytes())
        server = config.get("mcpServers", {}).get("mission012")
        if not isinstance(server, dict):
            raise RuntimeError("the mission012 MCP server is missing")
        command = server.get("command")
        arguments = server.get("args")
        if not isinstance(command, str) or not Path(command).is_absolute() or not isinstance(arguments, list):
            raise RuntimeError("the mission012 MCP command is invalid")
        if any(not isinstance(argument, str) for argument in arguments):
            raise RuntimeError("the mission012 MCP arguments are invalid")
        self.process = subprocess.Popen(
            [command, *arguments],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=None if capture_host_stderr else subprocess.DEVNULL,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
            close_fds=True,
            bufsize=0,
        )
        assert self.process.stdin is not None and self.process.stdout is not None
        self.read_fd = self.process.stdout.fileno()
        self.write_fd = self.process.stdin.fileno()
        self.next_id = 1
        self.buffer = bytearray()

    def _read_line(self, timeout_seconds: float = 180.0) -> dict[str, object]:
        deadline = time.monotonic() + timeout_seconds
        while True:
            newline = self.buffer.find(b"\n")
            if newline >= 0:
                if newline > MAX_FRAME_BYTES:
                    raise RuntimeError("MCP response exceeded the frame limit")
                raw = bytes(self.buffer[:newline])
                del self.buffer[: newline + 1]
                value = json.loads(raw)
                if not isinstance(value, dict):
                    raise RuntimeError("MCP response must be a JSON object")
                return value
            if len(self.buffer) >= MAX_FRAME_BYTES:
                raise RuntimeError("MCP response exceeded the frame limit")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("MCP response deadline expired")
            ready, _, _ = select.select([self.read_fd], [], [], min(remaining, 0.25))
            if not ready:
                if self.process is not None and self.process.poll() is not None:
                    raise RuntimeError("MCP host exited before replying")
                continue
            chunk = os.read(self.read_fd, 65536)
            if not chunk:
                raise RuntimeError("MCP host closed its response stream")
            self.buffer.extend(chunk)

    def _write(self, value: dict[str, object]) -> None:
        payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"
        if len(payload) > MAX_FRAME_BYTES:
            raise RuntimeError("MCP request exceeded the frame limit")
        view = memoryview(payload)
        while view:
            written = os.write(self.write_fd, view)
            if written <= 0:
                raise OSError("short MCP request write")
            view = view[written:]

    def request(self, method: str, params: dict[str, object] | None = None) -> dict[str, object]:
        request_id = self.next_id
        self.next_id += 1
        request: dict[str, object] = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            request["params"] = params
        self._write(request)
        response = self._read_line()
        if response.get("id") != request_id or response.get("jsonrpc") != "2.0":
            raise RuntimeError("MCP response does not match its request")
        if "error" in response:
            raise RuntimeError(f"MCP request failed: {response['error']}")
        result = response.get("result")
        if not isinstance(result, dict):
            raise RuntimeError("MCP response has no result object")
        return result

    def notify(self, method: str) -> None:
        self._write({"jsonrpc": "2.0", "method": method})

    def call_tool(self, name: str, arguments: dict[str, object]) -> tuple[dict[str, object], bool]:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        content = result.get("content")
        if not isinstance(content, list) or not content or not isinstance(content[0], dict):
            raise RuntimeError(f"MCP tool {name} returned no text result")
        text = content[0].get("text")
        if not isinstance(text, str):
            raise RuntimeError(f"MCP tool {name} returned a non-text result")
        decoded = json.loads(text)
        if not isinstance(decoded, dict):
            raise RuntimeError(f"MCP tool {name} returned a non-object result")
        return decoded, result.get("isError") is True

    def close(self) -> int:
        if self.process is None:
            try:
                # Send an explicit MCP shutdown request so the trusted
                # supervisor can hold the Rust host at its gate barrier. EOF
                # alone would finalize the host without exposing that boundary.
                self.request("shutdown", {})
            finally:
                os.close(self.read_fd)
                self.read_fd = -1
            return 0
        self.process.stdin.close()
        return self.process.wait(timeout=20)

    def abort(self) -> None:
        if self.process is None:
            try:
                if self.read_fd >= 0:
                    os.close(self.read_fd)
                    self.read_fd = -1
            except OSError:
                pass
            return
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
        try:
            self.process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            if self.process.poll() is None:
                self.process.kill()
            self.process.wait(timeout=5)


def run_with_evidence(
    config_path: Path | None,
    interrupt_after_write: bool = False,
    capture_host_stderr: bool = False,
    stdio_mcp: bool = False,
    wait_at_gate_barrier: bool = False,
) -> tuple[str, bool, bool]:
    if wait_at_gate_barrier and not stdio_mcp:
        raise RuntimeError("the held gate barrier is available only to the isolated stdio MCP harness")
    client = MCPClient(
        config_path,
        capture_host_stderr=capture_host_stderr,
        stdio_mcp=stdio_mcp,
    )
    try:
        initialized = client.request(
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "mission012-local-test-adapter", "version": "1.0"},
            },
        )
        if initialized.get("protocolVersion") != "2025-11-25":
            raise RuntimeError("MCP host negotiated an unexpected protocol version")
        client.notify("notifications/initialized")
        listed = client.request("tools/list").get("tools")
        if not isinstance(listed, list) or {item.get("name") for item in listed if isinstance(item, dict)} != MCP_TOOLS:
            raise RuntimeError("MCP host did not expose the exact four mission tools")

        context, context_error = client.call_tool("mission_context", {})
        if context_error:
            raise RuntimeError("admitted mission context was refused")
        issue_context_observed = observe_untrusted_work_item(
            context.get("untrusted_work_item")
        )
        requirement_text = context.get("requirements_json")
        if not isinstance(requirement_text, str):
            raise RuntimeError("admitted requirements are missing from mission context")
        requirement = json.loads(requirement_text)
        request_text = requirement.get("request") if isinstance(requirement, dict) else None
        if not isinstance(request_text, str) or "maximum" not in request_text.casefold():
            raise RuntimeError("admitted request does not describe the maximum boundary")

        baseline, baseline_error = client.call_tool("cargo_test", {})
        baseline_output = str(baseline.get("stdout")) + "\n" + str(baseline.get("stderr"))
        if (
            baseline_error
            or baseline.get("exit_code") != 101
            or baseline.get("expected_baseline_failure") is not True
            or "left: -2" not in baseline_output
            or "right: 10" not in baseline_output
        ):
            raise RuntimeError("the unchanged candidate did not show its actionable upper-bound failure")

        source, source_error = client.call_tool("workspace_read_text", {"path": "src/lib.rs"})
        acceptance, acceptance_error = client.call_tool(
            "workspace_read_text", {"path": "tests/acceptance.rs"}
        )
        if source_error or acceptance_error:
            raise RuntimeError("the authorized source or acceptance read was refused")
        source_text = source.get("content")
        acceptance_text = acceptance.get("content")
        if not isinstance(source_text, str) or not isinstance(acceptance_text, str):
            raise RuntimeError("mission source or acceptance file is missing")
        if "values_outside_the_range_clamp_to_the_nearest_boundary" not in acceptance_text:
            raise RuntimeError("the protected acceptance suite differs from the test adapter fixture")
        if source_text.count(SOURCE_BUG) != 1:
            raise RuntimeError("the source no longer matches the pinned test adapter fixture")
        fixed_source = source_text.replace(SOURCE_BUG, SOURCE_FIX, 1)
        resume = context.get("resume_checkpoint")
        if resume is not None:
            if (
                not isinstance(resume, dict)
                or resume.get("schema_version") != 1
                or resume.get("status") != "unverified"
                or resume.get("source_path") != "src/lib.rs"
                or resume.get("source_content") != fixed_source
                or resume.get("source_size_bytes") != len(str(resume.get("source_content", "")).encode())
                or resume.get("source_sha256")
                != hashlib.sha256(str(resume.get("source_content", "")).encode()).hexdigest()
            ):
                raise RuntimeError("host did not provide a bound, unverified source checkpoint")
            fixed_source = resume["source_content"]
        _, write_error = client.call_tool(
            "workspace_write_text", {"path": "src/lib.rs", "content": fixed_source}
        )
        if write_error:
            raise RuntimeError("the authorized source edit was refused")

        if interrupt_after_write:
            exit_code = client.close()
            if exit_code != 0:
                raise RuntimeError(
                    supervisor_shutdown_error(
                        exit_code,
                        "durable checkpoint and release of the interrupted attempt",
                    )
                )
            return (
                "ADAPTATEUR DE TEST LOCAL SANS LLM: contexte admis consommé; échec rouge observé; "
                + (
                    f"{UNTRUSTED_WORK_ITEM_OBSERVED_MARKER}; "
                    if issue_context_observed
                    else ""
                )
                + "écriture MCP terminée; arrêt injecté avant toute vérification verte.",
                True,
                issue_context_observed,
            )

        candidate, candidate_error = client.call_tool("cargo_test", {})
        if (
            candidate_error
            or candidate.get("exit_code") != 0
            or candidate.get("verified_acceptance_pass") is not True
        ):
            raise RuntimeError("the corrected candidate did not pass the protected acceptance test")
        if wait_at_gate_barrier:
            gate_context, gate_context_error = client.call_tool("mission_context", {})
            if gate_context_error or gate_context != context:
                raise RuntimeError("admitted mission context changed across the protected acceptance run")
        exit_code = client.close()
        if exit_code != 0:
            raise RuntimeError(
                supervisor_shutdown_error(
                    exit_code, "clean host finalization and candidate result"
                )
            )
    except BaseException:
        client.abort()
        raise
    return (
        "ADAPTATEUR DE TEST LOCAL SANS LLM: contexte admis consommé; échec rouge observé; "
        + (
            f"{UNTRUSTED_WORK_ITEM_OBSERVED_MARKER}; "
            if issue_context_observed
            else ""
        )
        + (
            f"checkpoint `unverified` {resume['checkpoint_sha256']} réappliqué par MCP; "
            if resume is not None
            else ""
        )
        + "src/lib.rs corrigé par MCP; test protégé vert; "
        + ("barrière gate libérée; " if wait_at_gate_barrier else "")
        + "hôte finalisé avec reçu.",
        False,
        issue_context_observed,
    )


def run(
    config_path: Path | None,
    interrupt_after_write: bool = False,
    capture_host_stderr: bool = False,
    stdio_mcp: bool = False,
    wait_at_gate_barrier: bool = False,
) -> tuple[str, bool]:
    """Compatibility helper for tests that only inspect the adapter message."""
    message, interrupted, _ = run_with_evidence(
        config_path,
        interrupt_after_write,
        capture_host_stderr,
        stdio_mcp,
        wait_at_gate_barrier,
    )
    return message, interrupted


def forgejo_route_probe_specs(
    owner: str,
    repository: str,
    pull_number: int,
) -> tuple[tuple[str, str, str], ...]:
    """Build unambiguous API, Web, and receive-pack paths for one PR target."""
    component = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$")
    if (
        not isinstance(owner, str)
        or not component.fullmatch(owner)
        or owner in {".", ".."}
        or not isinstance(repository, str)
        or not component.fullmatch(repository)
        or repository in {".", ".."}
        or not isinstance(pull_number, int)
        or isinstance(pull_number, bool)
        or pull_number < 1
    ):
        raise ValueError("Forgejo route probe target is invalid")
    return (
        ("api", "GET", f"/api/v1/repos/{owner}/{repository}/pulls/{pull_number}"),
        ("web", "GET", f"/{owner}/{repository}/pulls/{pull_number}"),
        ("git_receive_pack", "POST", f"/{owner}/{repository}.git/git-receive-pack"),
    )


def probe_sandbox_capabilities(
    credential_path: Path,
    loopback_port: int,
    *,
    owner: str = "forge-admin",
    repository: str = "mission-012-clamp",
    pull_number: int = 1,
    private_file_probes: list[tuple[str, Path]] | None = None,
    private_file_write_probes: list[tuple[str, Path]] | None = None,
) -> dict[str, object]:
    if not (1 <= loopback_port <= 65535):
        raise RuntimeError("sandbox loopback probe port is invalid")
    try:
        credential_path.read_bytes()
    except OSError as error:
        if error.errno not in {errno.ENOENT, errno.EACCES, errno.EPERM, errno.ENOTDIR}:
            raise RuntimeError("synthetic credential probe failed inconclusively") from None
        credential_visible = False
    else:
        credential_visible = True
    if credential_visible:
        raise RuntimeError("synthetic credential was visible inside the sandbox")

    host_runtime_dir = Path("/run/user")
    try:
        host_runtime_entries = list(host_runtime_dir.iterdir())
    except FileNotFoundError:
        host_runtime_entries = []
    except OSError:
        raise RuntimeError("host systemd bus visibility probe was inconclusive") from None
    for runtime_entry in host_runtime_entries:
        try:
            (runtime_entry / "bus").lstat()
        except FileNotFoundError:
            continue
        except OSError:
            raise RuntimeError("host systemd bus visibility probe was inconclusive") from None
        raise RuntimeError("host systemd bus path was visible inside the sandbox")
    host_systemd_bus_visible = False

    private_files_not_visible: list[str] = []
    seen_labels: set[str] = set()
    for label, path in private_file_probes or []:
        if (
            label not in {
                "admission_source",
                "forgejo_read_token",
                "host_admission",
                "host_systemd_bus",
                "invalid_forgejo_read_token",
                "mission_journal",
                "webhook_inbox",
                "webhook_attestation_seed",
            }
            or label in seen_labels
            or not path.is_absolute()
        ):
            raise RuntimeError("private runtime file probe is invalid")
        seen_labels.add(label)
        if label == "host_systemd_bus":
            try:
                path.lstat()
            except FileNotFoundError:
                private_files_not_visible.append(label)
                continue
            except OSError:
                raise RuntimeError("host systemd bus path probe was inconclusive") from None
            raise RuntimeError("host systemd bus path was visible inside the sandbox")
        try:
            descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        except OSError as error:
            if error.errno not in {errno.ENOENT, errno.EACCES, errno.EPERM, errno.ENOTDIR}:
                raise RuntimeError("private runtime file probe failed inconclusively") from None
            private_files_not_visible.append(label)
        else:
            try:
                os.read(descriptor, 1)
            finally:
                os.close(descriptor)
            raise RuntimeError("private host file was visible inside the sandbox")

    private_file_writes_denied: list[str] = []
    seen_write_labels: set[str] = set()
    for label, path in private_file_write_probes or []:
        if (
            label not in PRIVATE_WRITE_PROBE_LABELS
            or label in seen_write_labels
            or not path.is_absolute()
            or not path.is_relative_to(Path("/tmp"))
        ):
            raise RuntimeError("private runtime write probe is invalid")
        seen_write_labels.add(label)
        if probe_private_file_write_access(path):
            raise RuntimeError("private runtime webhook inbox was writable inside the sandbox")
        private_file_writes_denied.append(label)

    import socket

    forgejo_routes = forgejo_route_probe_specs(owner, repository, pull_number)
    blocked_routes: list[str] = []
    for name, method, path in forgejo_routes:
        request = (
            f"{method} {path} HTTP/1.1\r\n"
            "Host: forgejo.test\r\n"
            "Connection: close\r\n"
            "Content-Length: 0\r\n\r\n"
        ).encode("ascii")
        try:
            with socket.create_connection(("127.0.0.1", loopback_port), timeout=0.5) as connection:
                connection.sendall(request)
        except OSError:
            blocked_routes.append(name)
    if len(blocked_routes) != len(forgejo_routes):
        raise RuntimeError("sandbox reached a host listener through a direct Forgejo route probe")
    checks: dict[str, object] = {
        "credential_visible": False,
        "host_systemd_bus_visible": host_systemd_bus_visible,
        "network_connected": False,
        "direct_forgejo_routes_blocked": blocked_routes,
        "open_fds": open_file_descriptors(),
    }
    if private_file_probes is not None:
        checks["private_runtime_files_not_visible"] = sorted(private_files_not_visible)
    if private_file_write_probes is not None:
        checks["private_runtime_file_writes_denied"] = sorted(private_file_writes_denied)
    return checks


def probe_private_file_write_access(path: Path) -> bool:
    """Try to append a fixed canary marker; true means the fixture was writable."""
    validate_private_webhook_inbox_write_probe(path)
    try:
        descriptor = os.open(
            path,
            os.O_WRONLY | os.O_APPEND | os.O_CLOEXEC | os.O_NOFOLLOW,
        )
    except OSError as error:
        if error.errno in {errno.ENOENT, errno.EACCES, errno.EPERM, errno.ENOTDIR}:
            return False
        raise RuntimeError("private runtime write probe failed inconclusively") from None

    try:
        offset = 0
        while offset < len(PRIVATE_WRITE_PROBE_MARKER):
            written = os.write(descriptor, PRIVATE_WRITE_PROBE_MARKER[offset:])
            if written == 0:
                raise RuntimeError("private runtime write probe made no progress")
            offset += written
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    return True


def validate_private_webhook_inbox_write_probe(path: Path) -> None:
    """Accept only the fixed synthetic ``/tmp/.runtime`` inbox probe path."""
    if (
        not path.is_absolute()
        or ".." in path.parts
        or path.name != "webhook-inbox.jsonl"
        or path.parent.name != ".runtime"
        or not path.is_relative_to(Path("/tmp"))
    ):
        raise RuntimeError("private runtime write probe path is invalid")


def sandbox_probe_harness(
    credential_path: Path,
    loopback_port: int,
    *,
    owner: str,
    repository: str,
    pull_number: int,
    hold_open: bool,
    private_file_probes: list[tuple[str, Path]] | None = None,
    private_file_write_probes: list[tuple[str, Path]] | None = None,
) -> int:
    checks = probe_sandbox_capabilities(
        credential_path,
        loopback_port,
        owner=owner,
        repository=repository,
        pull_number=pull_number,
        private_file_probes=private_file_probes,
        private_file_write_probes=private_file_write_probes,
    )
    routes = [
        {"name": name, "method": method, "path": path}
        for name, method, path in forgejo_route_probe_specs(owner, repository, pull_number)
    ]
    open_fds = open_file_descriptors()
    network_namespace = os.readlink("/proc/self/ns/net")

    def emit(value: dict[str, object]) -> None:
        sys.stdout.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")
        sys.stdout.flush()

    if not hold_open:
        emit(
            {
                "type": "sandbox_egress_probe",
                "sandbox_checks": checks,
                "route_probes": routes,
                "network_namespace": network_namespace,
                "open_fds": open_fds,
            }
        )
        return 0

    emit(
        {
            "type": "sandbox_egress_probe_ready",
            "sandbox_checks": checks,
            "route_probes": routes,
            "network_namespace": network_namespace,
            "open_fds": open_fds,
        }
    )
    command = sys.stdin.buffer.readline(129)
    if command != b"reprobe\n":
        emit({"type": "sandbox_egress_probe_error", "reason": "expected_reprobe"})
        return 1
    after_checks = probe_sandbox_capabilities(
        credential_path,
        loopback_port,
        owner=owner,
        repository=repository,
        pull_number=pull_number,
        private_file_probes=private_file_probes,
        private_file_write_probes=private_file_write_probes,
    )
    if after_checks != checks:
        emit({"type": "sandbox_egress_probe_error", "reason": "probe_changed_after_gate"})
        return 1
    emit(
        {
            "type": "sandbox_egress_probe_after_gate",
            "sandbox_checks": after_checks,
            "route_probes": routes,
            "network_namespace": os.readlink("/proc/self/ns/net"),
        }
    )
    command = sys.stdin.buffer.readline(129)
    if command != b"release\n":
        emit({"type": "sandbox_egress_probe_error", "reason": "expected_release"})
        return 1
    emit({"type": "sandbox_egress_probe_released"})
    return 0


def _descriptor_is_open(descriptor: int) -> bool:
    try:
        os.fstat(descriptor)
    except OSError:
        return False
    return True


def open_file_descriptors() -> list[int]:
    """Return the live descriptor set without counting the /proc scan handle."""
    directory_fd = os.open("/proc/self/fd", os.O_RDONLY | os.O_DIRECTORY)
    try:
        return sorted(
            int(entry)
            for entry in os.listdir(directory_fd)
            if entry.isdigit()
            and int(entry) != directory_fd
            and _descriptor_is_open(int(entry))
        )
    finally:
        os.close(directory_fd)


def main() -> int:
    parser = argparse.ArgumentParser()
    transport = parser.add_mutually_exclusive_group(required=True)
    transport.add_argument("--mcp-config", type=Path)
    transport.add_argument(
        "--stdio-mcp",
        action="store_true",
        help="use stdin/stdout only for MCP frames; reserved for the Bubblewrap test harness",
    )
    transport.add_argument(
        "--probe-harness",
        action="store_true",
        help="run only the bounded sandbox egress probe protocol",
    )
    parser.add_argument("--interrupt-after-write", action="store_true")
    parser.add_argument("--probe-credential", type=Path)
    parser.add_argument("--probe-port", type=int)
    parser.add_argument("--probe-owner", default="forge-admin")
    parser.add_argument("--probe-repository", default="mission-012-clamp")
    parser.add_argument("--probe-pull-number", type=int, default=1)
    parser.add_argument("--probe-private-file", action="append", nargs=2, default=[], metavar=("LABEL", "PATH"))
    parser.add_argument("--probe-private-write-file", action="append", nargs=2, default=[], metavar=("LABEL", "PATH"))
    parser.add_argument("--probe-hold-open", action="store_true")
    parser.add_argument("--wait-at-gate-barrier", action="store_true")
    parser.add_argument(
        "--capture-host-stderr",
        action="store_true",
        help="capture local MCP host diagnostics for isolated restart tests",
    )
    args = parser.parse_args()
    probe_mode = args.stdio_mcp or args.probe_harness
    if probe_mode and (args.probe_credential is None or args.probe_port is None):
        parser.error("sandbox probe mode requires both synthetic probe arguments")
    if not probe_mode and (
        args.probe_credential is not None
        or args.probe_port is not None
        or args.probe_hold_open
    ):
        parser.error("sandbox probe arguments are available only with a probe mode")
    if args.probe_hold_open and not args.probe_harness:
        parser.error("held probes require --probe-harness")
    if args.probe_private_write_file and not args.probe_harness:
        parser.error("private runtime write probes require --probe-harness")
    if args.wait_at_gate_barrier and not args.stdio_mcp:
        parser.error("the gate barrier requires --stdio-mcp")

    if args.probe_harness:
        try:
            return sandbox_probe_harness(
                args.probe_credential,
                args.probe_port,
                owner=args.probe_owner,
                repository=args.probe_repository,
                pull_number=args.probe_pull_number,
                hold_open=args.probe_hold_open,
                private_file_probes=(
                    [(label, Path(path)) for label, path in args.probe_private_file]
                    if args.probe_private_file
                    else None
                ),
                private_file_write_probes=(
                    [(label, Path(path)) for label, path in args.probe_private_write_file]
                    if args.probe_private_write_file
                    else None
                ),
            )
        except Exception as error:
            print(
                json.dumps({"type": "sandbox_egress_probe_error", "reason": str(error)}, separators=(",", ":")),
                flush=True,
            )
            return 1

    sandbox_checks: dict[str, object] | None = None
    try:
        if args.stdio_mcp:
            sandbox_checks = probe_sandbox_capabilities(
                args.probe_credential,
                args.probe_port,
                private_file_probes=(
                    [(label, Path(path)) for label, path in args.probe_private_file]
                    if args.probe_private_file
                    else None
                ),
            )
            sandbox_checks["probe_target_port"] = args.probe_port
            sandbox_checks["network_namespace"] = os.readlink("/proc/self/ns/net")
            sandbox_checks["pid_namespace"] = os.readlink("/proc/self/ns/pid")
            sandbox_checks["environment_keys"] = sorted(os.environ)
        result, interrupted, issue_context_observed = run_with_evidence(
            args.mcp_config,
            args.interrupt_after_write,
            args.capture_host_stderr,
            args.stdio_mcp,
            args.wait_at_gate_barrier,
        )
    except Exception as error:
        print(
            json.dumps({"type": "error", "error": str(error)}, sort_keys=True),
            file=sys.stderr if args.stdio_mcp else sys.stdout,
        )
        return 1
    event: dict[str, object] = {
        "type": "result",
        "subtype": "interrupted" if interrupted else "success",
        "is_error": False,
        "result": result,
        "test_adapter_untrusted_work_item_observed": issue_context_observed,
    }
    if sandbox_checks is not None:
        event["sandbox_checks"] = sandbox_checks
    print(
        json.dumps(event, sort_keys=True),
        file=sys.stderr if args.stdio_mcp else sys.stdout,
    )
    return 75 if interrupted else 0


if __name__ == "__main__":
    raise SystemExit(main())
