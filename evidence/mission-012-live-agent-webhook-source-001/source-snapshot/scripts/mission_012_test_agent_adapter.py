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
    def __init__(self, config_path: Path) -> None:
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
            stderr=subprocess.DEVNULL,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
            close_fds=True,
            bufsize=0,
        )
        assert self.process.stdin is not None and self.process.stdout is not None
        self.next_id = 1
        self.buffer = bytearray()

    def _read_line(self, timeout_seconds: float = 180.0) -> dict[str, object]:
        assert self.process.stdout is not None
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
            ready, _, _ = select.select([self.process.stdout.fileno()], [], [], min(remaining, 0.25))
            if not ready:
                if self.process.poll() is not None:
                    raise RuntimeError("MCP host exited before replying")
                continue
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise RuntimeError("MCP host closed its response stream")
            self.buffer.extend(chunk)

    def _write(self, value: dict[str, object]) -> None:
        assert self.process.stdin is not None
        payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"
        if len(payload) > MAX_FRAME_BYTES:
            raise RuntimeError("MCP request exceeded the frame limit")
        self.process.stdin.write(payload)
        self.process.stdin.flush()

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
        assert self.process.stdin is not None
        self.process.stdin.close()
        return self.process.wait(timeout=20)

    def abort(self) -> None:
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
        try:
            self.process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            if self.process.poll() is None:
                self.process.kill()
            self.process.wait(timeout=5)


def run(config_path: Path, interrupt_after_write: bool = False) -> tuple[str, bool]:
    client = MCPClient(config_path)
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mcp-config", required=True, type=Path)
    parser.add_argument("--interrupt-after-write", action="store_true")
    args = parser.parse_args()
    try:
        result, interrupted = run(args.mcp_config, args.interrupt_after_write)
    except Exception as error:
        print(json.dumps({"type": "error", "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps({
        "type": "result",
        "subtype": "interrupted" if interrupted else "success",
        "is_error": False,
        "result": result,
    }, sort_keys=True))
    return 75 if interrupted else 0


if __name__ == "__main__":
    raise SystemExit(main())
