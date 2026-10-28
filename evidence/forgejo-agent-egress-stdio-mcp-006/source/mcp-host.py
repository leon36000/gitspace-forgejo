#!/usr/bin/env python3
"""Local-only MCP host fixture for the isolated task/live-gate composition."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import time


TOOLS = ("mission_context", "workspace_read_text", "workspace_write_text", "cargo_test")
MAX_TOOL_OUTPUT_BYTES = 128 * 1024


def append_call(path: Path, value: str) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "ab") as stream:
        stream.write(value.encode("utf-8") + b"\n")
        stream.flush()


def tool_result(value: dict[str, object], *, is_error: bool = False) -> dict[str, object]:
    return {
        "isError": is_error,
        "content": [{"type": "text", "text": json.dumps(value, sort_keys=True)}],
    }


def write_barrier(ready: Path, release: Path) -> bool:
    fd = os.open(ready, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(f"pid={os.getpid()}\n".encode("ascii"))
        stream.flush()
        os.fsync(stream.fileno())
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if release.exists():
            return True
        time.sleep(0.02)
    return False


def run_cargo_test(args: argparse.Namespace, *, baseline: bool) -> dict[str, object]:
    environment = {
        "PATH": f"{args.toolchain_bin}:/usr/bin:/bin",
        "HOME": str(args.home),
        "CARGO_HOME": str(args.cargo_home),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "CARGO_TERM_COLOR": "never",
        "RUST_BACKTRACE": "0",
    }
    completed = subprocess.run(
        [str(args.cargo), "test", "--offline", "--locked"],
        cwd=args.workspace,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=35,
        check=False,
    )
    if len(completed.stdout) + len(completed.stderr) > MAX_TOOL_OUTPUT_BYTES:
        return {"exit_code": completed.returncode, "stdout": "", "stderr": "test output exceeded the fixture limit"}
    stdout = completed.stdout.decode("utf-8", errors="replace")
    stderr = completed.stderr.decode("utf-8", errors="replace")
    log_path = args.baseline_log if baseline else args.corrected_log
    fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(completed.stdout)
        if completed.stderr:
            stream.write(b"\n--- stderr ---\n")
            stream.write(completed.stderr)
        stream.flush()
        os.fsync(stream.fileno())
    return {
        "exit_code": completed.returncode,
        "stdout": stdout,
        "stderr": stderr,
        "expected_baseline_failure": completed.returncode == 101 and "left: -2" in stdout + stderr and "right: 10" in stdout + stderr,
        "verified_acceptance_pass": completed.returncode == 0,
    }


def handle_tool(name: object, values: object, args: argparse.Namespace, state: dict[str, bool]) -> dict[str, object]:
    if not isinstance(name, str) or not isinstance(values, dict):
        return tool_result({"error": "invalid fixture tool request"}, is_error=True)
    if name == "mission_context":
        if not write_barrier(args.barrier_ready, args.barrier_release):
            return tool_result({"error": "gate barrier release deadline expired"}, is_error=True)
        return tool_result(
            {
                "requirements_json": json.dumps(
                    {"request": "For values above the maximum, clamp the result to the maximum."},
                    sort_keys=True,
                ),
                "resume_checkpoint": None,
            }
        )
    if name == "cargo_test":
        return tool_result(run_cargo_test(args, baseline=not state["written"]))
    if name == "workspace_read_text":
        relative = values.get("path")
        permitted = {"src/lib.rs": args.workspace / "src/lib.rs", "tests/acceptance.rs": args.workspace / "tests/acceptance.rs"}
        path = permitted.get(relative) if isinstance(relative, str) else None
        if path is None:
            return tool_result({"error": "workspace path is outside the fixture allowlist"}, is_error=True)
        try:
            content = path.read_text(encoding="utf-8")
        except OSError:
            return tool_result({"error": "fixture file could not be read"}, is_error=True)
        if len(content.encode("utf-8")) > MAX_TOOL_OUTPUT_BYTES:
            return tool_result({"error": "fixture file exceeds the tool limit"}, is_error=True)
        return tool_result({"content": content})
    if name == "workspace_write_text":
        if values.get("path") != "src/lib.rs" or not isinstance(values.get("content"), str):
            return tool_result({"error": "fixture write is outside the source allowlist"}, is_error=True)
        content = values["content"]
        if len(content.encode("utf-8")) > MAX_TOOL_OUTPUT_BYTES:
            return tool_result({"error": "fixture write exceeds the tool limit"}, is_error=True)
        (args.workspace / "src/lib.rs").write_text(content, encoding="utf-8")
        state["written"] = True
        return tool_result({"written": True})
    return tool_result({"error": "unapproved fixture tool"}, is_error=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workspace", required=True, type=Path)
    parser.add_argument("--calls-file", required=True, type=Path)
    parser.add_argument("--barrier-ready", required=True, type=Path)
    parser.add_argument("--barrier-release", required=True, type=Path)
    parser.add_argument("--cargo", required=True, type=Path)
    parser.add_argument("--toolchain-bin", required=True, type=Path)
    parser.add_argument("--cargo-home", required=True, type=Path)
    parser.add_argument("--home", required=True, type=Path)
    parser.add_argument("--baseline-log", required=True, type=Path)
    parser.add_argument("--corrected-log", required=True, type=Path)
    args = parser.parse_args()

    state = {"written": False}
    for line in __import__("sys").stdin:
        request = json.loads(line)
        method = request.get("method")
        if "id" not in request:
            continue
        params = request.get("params", {})
        name = params.get("name") if method == "tools/call" and isinstance(params, dict) else None
        append_call(args.calls_file, f"tools/call:{name}" if method == "tools/call" else str(method))
        if method == "initialize":
            result = {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "local-egress-fixture", "version": "1"}}
        elif method == "tools/list":
            result = {"tools": [{"name": name, "inputSchema": {"type": "object"}} for name in TOOLS]}
        elif method == "tools/call" and isinstance(params, dict):
            result = handle_tool(name, params.get("arguments", {}), args, state)
        else:
            result = None
        response = {"jsonrpc": "2.0", "id": request["id"]}
        if result is None:
            response["error"] = {"code": -32601, "message": "method not found"}
        else:
            response["result"] = result
        __import__("sys").stdout.write(json.dumps(response, sort_keys=True, separators=(",", ":")) + "\n")
        __import__("sys").stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
