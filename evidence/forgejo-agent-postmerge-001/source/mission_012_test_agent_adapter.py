#!/usr/bin/env python3
"""Explicit no-LLM adapter for exercising the MISSION-012 MCP contract.

This deterministic fixture consumes the admitted context, observes the
protected red test, makes the one authorized source edit through MCP, and
observes the green test. It does not call a model or access the workspace
directly.
"""

from __future__ import annotations

import argparse
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
MCP_TOOLS = {
    "mission_context",
    "workspace_read_text",
    "workspace_write_text",
    "cargo_test",
}
SOURCE_BUG = "minimum // BUG: values above the range should clamp to maximum."


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
            os.close(self.read_fd)
            return 0
        self.process.stdin.close()
        return self.process.wait(timeout=20)

    def abort(self) -> None:
        if self.process is None:
            try:
                os.close(self.read_fd)
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


def run(
    config_path: Path | None,
    interrupt_after_write: bool = False,
    capture_host_stderr: bool = False,
    stdio_mcp: bool = False,
) -> tuple[str, bool]:
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
        fixed_source = source_text.replace(SOURCE_BUG, "maximum", 1)
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
                raise RuntimeError(f"MCP host did not durably checkpoint and release the interrupted attempt ({exit_code})")
            return (
                "ADAPTATEUR DE TEST LOCAL SANS LLM: contexte admis consommé; échec rouge observé; "
                "écriture MCP terminée; arrêt injecté avant toute vérification verte.",
                True,
            )

        candidate, candidate_error = client.call_tool("cargo_test", {})
        if (
            candidate_error
            or candidate.get("exit_code") != 0
            or candidate.get("verified_acceptance_pass") is not True
        ):
            raise RuntimeError("the corrected candidate did not pass the protected acceptance test")
        exit_code = client.close()
        if exit_code != 0:
            raise RuntimeError(f"MCP supervisor exited with status {exit_code}")
    except BaseException:
        client.abort()
        raise
    return (
        "ADAPTATEUR DE TEST LOCAL SANS LLM: contexte admis consommé; échec rouge observé; "
        + (
            f"checkpoint `unverified` {resume['checkpoint_sha256']} réappliqué par MCP; "
            if resume is not None
            else ""
        )
        + "src/lib.rs corrigé par MCP; test protégé vert; hôte finalisé avec reçu.",
        False,
    )


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
) -> dict[str, object]:
    if not (1 <= loopback_port <= 65535):
        raise RuntimeError("sandbox loopback probe port is invalid")
    try:
        credential_path.read_bytes()
    except OSError:
        credential_visible = False
    else:
        credential_visible = True
    if credential_visible:
        raise RuntimeError("synthetic credential was visible inside the sandbox")

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
    return {
        "credential_visible": False,
        "network_connected": False,
        "direct_forgejo_routes_blocked": blocked_routes,
    }


def sandbox_probe_harness(
    credential_path: Path,
    loopback_port: int,
    *,
    owner: str,
    repository: str,
    pull_number: int,
    hold_open: bool,
) -> int:
    checks = probe_sandbox_capabilities(
        credential_path,
        loopback_port,
        owner=owner,
        repository=repository,
        pull_number=pull_number,
    )
    routes = [
        {"name": name, "method": method, "path": path}
        for name, method, path in forgejo_route_probe_specs(owner, repository, pull_number)
    ]
    directory_fd = os.open("/proc/self/fd", os.O_RDONLY | os.O_DIRECTORY)
    try:
        open_fds = sorted(
            int(entry)
            for entry in os.listdir(directory_fd)
            if entry.isdigit()
            and int(entry) != directory_fd
            and _descriptor_is_open(int(entry))
        )
    finally:
        os.close(directory_fd)
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
    parser.add_argument("--probe-hold-open", action="store_true")
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

    if args.probe_harness:
        try:
            return sandbox_probe_harness(
                args.probe_credential,
                args.probe_port,
                owner=args.probe_owner,
                repository=args.probe_repository,
                pull_number=args.probe_pull_number,
                hold_open=args.probe_hold_open,
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
            sandbox_checks = probe_sandbox_capabilities(args.probe_credential, args.probe_port)
        result, interrupted = run(
            args.mcp_config,
            args.interrupt_after_write,
            args.capture_host_stderr,
            args.stdio_mcp,
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
