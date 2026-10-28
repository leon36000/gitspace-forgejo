from __future__ import annotations

import json
import io
import os
from pathlib import Path
import selectors
import socket
import subprocess
import tempfile
import sys
import unittest
from contextlib import redirect_stdout
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "scripts/mission_012_sandboxed_test_adapter.py"
PYTHON = Path("/usr/bin/python3")
SOURCE_BUG = "minimum // BUG: values above the range should clamp to maximum."
sys.path.insert(0, str(ROOT / "scripts"))
import mission_012_test_agent_adapter as agent_adapter
import mission_012_sandboxed_test_adapter as sandbox_wrapper


FAKE_MCP_HOST = r'''#!/usr/bin/python3
import argparse
import json
from pathlib import Path
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--calls-file", required=True)
args = parser.parse_args()
calls_path = Path(args.calls_file)
calls = []
source = "fn clamp(value: i32) -> i32 { value.minimum // BUG: values above the range should clamp to maximum. }\n"
acceptance = "fn values_outside_the_range_clamp_to_the_nearest_boundary() {}\n"
written = False
cargo_runs = 0
tools = ["mission_context", "workspace_read_text", "workspace_write_text", "cargo_test"]

def send(value):
    sys.stdout.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")
    sys.stdout.flush()

def tool_text(value, is_error=False):
    return {"isError": is_error, "content": [{"type": "text", "text": json.dumps(value, sort_keys=True)}]}

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if "id" not in request:
        continue
    calls.append(method)
    params = request.get("params", {})
    if method == "initialize":
        result = {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": [{"name": name, "inputSchema": {"type": "object"}} for name in tools]}
    elif method == "tools/call":
        name = params.get("name")
        values = params.get("arguments", {})
        calls[-1] = "tools/call:" + str(name)
        if name == "mission_context":
            result = tool_text({"requirements_json": json.dumps({"request": "Clamp values above maximum to maximum."}), "resume_checkpoint": None})
        elif name == "cargo_test":
            cargo_runs += 1
            if not written:
                result = tool_text({"exit_code": 101, "expected_baseline_failure": True, "stdout": "left: -2", "stderr": "right: 10"})
            else:
                result = tool_text({"exit_code": 0, "verified_acceptance_pass": True, "stdout": "ok", "stderr": ""})
        elif name == "workspace_read_text" and values.get("path") == "src/lib.rs":
            result = tool_text({"content": source})
        elif name == "workspace_read_text" and values.get("path") == "tests/acceptance.rs":
            result = tool_text({"content": acceptance})
        elif name == "workspace_write_text" and values.get("path") == "src/lib.rs":
            wanted = source.replace("minimum // BUG: values above the range should clamp to maximum.", "maximum", 1)
            written = values.get("content") == wanted
            result = tool_text({"written": written}, not written)
        else:
            result = tool_text({"error": "unapproved fixture tool"}, True)
    else:
        result = {"error": {"code": -32601, "message": "method not found"}}
    send({"jsonrpc": "2.0", "id": request["id"], "result": result})

Path(args.calls_file).write_text("\n".join(calls) + "\n", encoding="utf-8")
'''


class SandboxedAdapterTests(unittest.TestCase):
    def test_live_route_probe_binds_to_the_current_repository_and_pr(self) -> None:
        probes = agent_adapter.forgejo_route_probe_specs(
            "forge-admin", "agent-forge-core", 2
        )
        self.assertEqual(
            probes,
            (
                ("api", "GET", "/api/v1/repos/forge-admin/agent-forge-core/pulls/2"),
                ("web", "GET", "/forge-admin/agent-forge-core/pulls/2"),
                ("git_receive_pack", "POST", "/forge-admin/agent-forge-core.git/git-receive-pack"),
            ),
        )

    def test_live_route_probe_rejects_ambiguous_repository_identity(self) -> None:
        with self.assertRaises(ValueError):
            agent_adapter.forgejo_route_probe_specs("forge-admin", "../other", 2)
        with self.assertRaises(ValueError):
            agent_adapter.forgejo_route_probe_specs("forge-admin", "agent-forge-core", 0)

    def test_stdio_task_targets_the_requested_live_forgejo_listener(self) -> None:
        with tempfile.TemporaryDirectory(prefix="mission012-live-route-target-") as directory:
            credential = Path(directory) / "synthetic-credential.txt"
            credential.write_text("SYNTHETIC-CREDENTIAL-CANARY-ONLY\n", encoding="utf-8")
            credential.chmod(0o600)
            command = sandbox_wrapper.bubblewrap_command(
                credential,
                49152,
                owner="forge-admin",
                repository="mission-012-clamp",
                pull_number=1,
                forgejo_probe_port=3130,
                credential_probe_path="/proc/4242/fd/9",
                wait_at_gate_barrier=True,
            )

        probe_port_index = command.index("--probe-port")
        self.assertEqual(command[probe_port_index + 1], "3130")
        credential_index = command.index("--probe-credential")
        self.assertEqual(command[credential_index + 1], "/proc/4242/fd/9")
        self.assertIn("--wait-at-gate-barrier", command)
        self.assertIn("--probe-owner", command)
        self.assertIn("--probe-repository", command)
        self.assertIn("--probe-pull-number", command)

    def test_probe_process_remains_isolated_until_reprobe_and_release(self) -> None:
        if not Path("/usr/bin/bwrap").is_file() or not PYTHON.is_file():
            self.skipTest("local Bubblewrap/Python runtime is unavailable")

        with tempfile.TemporaryDirectory(prefix="mission012-held-probe-") as directory:
            credential = Path(directory) / "synthetic-credential.txt"
            credential.write_text("SYNTHETIC-CREDENTIAL-CANARY-ONLY\n", encoding="utf-8")
            credential.chmod(0o600)
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen(4)
                listener.setblocking(False)
                port = listener.getsockname()[1]
                command = sandbox_wrapper.bubblewrap_command(
                    credential,
                    port,
                    owner="forge-admin",
                    repository="agent-forge-core",
                    pull_number=2,
                    probe_only=True,
                    probe_hold_open=True,
                )
                process = subprocess.Popen(
                    command,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    env=sandbox_wrapper.HOST_ENV,
                    bufsize=0,
                    close_fds=True,
                    start_new_session=True,
                )
                try:
                    self.assertIsNotNone(process.stdin)
                    self.assertIsNotNone(process.stdout)

                    def read_event() -> dict[str, object]:
                        assert process.stdout is not None
                        with selectors.DefaultSelector() as selector:
                            selector.register(process.stdout, selectors.EVENT_READ)
                            self.assertTrue(selector.select(10), "held sandbox probe timed out")
                        line = process.stdout.readline(64 * 1024)
                        self.assertTrue(line, "held sandbox probe exited before its event")
                        event = json.loads(line)
                        self.assertIsInstance(event, dict)
                        return event

                    first = read_event()
                    self.assertEqual(first.get("type"), "sandbox_egress_probe_ready")
                    self.assertEqual(first.get("sandbox_checks"), {
                        "credential_visible": False,
                        "network_connected": False,
                        "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
                        "open_fds": [0, 1, 2],
                    })
                    self.assertEqual(first.get("open_fds"), [0, 1, 2])
                    self.assertNotEqual(first.get("network_namespace"), os.readlink("/proc/self/ns/net"))
                    self.assertTrue(process.poll() is None, "probe exited before the gate could run")
                    self.assertEqual(first.get("route_probes"), [
                        {"name": "api", "method": "GET", "path": "/api/v1/repos/forge-admin/agent-forge-core/pulls/2"},
                        {"name": "web", "method": "GET", "path": "/forge-admin/agent-forge-core/pulls/2"},
                        {"name": "git_receive_pack", "method": "POST", "path": "/forge-admin/agent-forge-core.git/git-receive-pack"},
                    ])
                    process.stdin.write(b"reprobe\n")
                    process.stdin.flush()
                    second = read_event()
                    self.assertEqual(second.get("type"), "sandbox_egress_probe_after_gate")
                    self.assertEqual(second.get("sandbox_checks"), first.get("sandbox_checks"))
                    self.assertEqual(second.get("network_namespace"), first.get("network_namespace"))
                    self.assertTrue(process.poll() is None)
                    process.stdin.write(b"release\n")
                    process.stdin.flush()
                    released = read_event()
                    self.assertEqual(released, {"type": "sandbox_egress_probe_released"})
                    self.assertEqual(process.wait(timeout=5), 0)
                    self.assertEqual(process.stderr.read(), b"")
                    with self.assertRaises(BlockingIOError):
                        listener.accept()
                finally:
                    sandbox_wrapper.stop_process_group(process)
                    for stream in (process.stdin, process.stdout, process.stderr):
                        if stream is not None:
                            stream.close()

    def test_stdio_adapter_uses_only_trusted_mcp_pipe_and_refuses_host_network_and_file(self) -> None:
        if not Path("/usr/bin/bwrap").is_file() or not PYTHON.is_file():
            self.skipTest("local Bubblewrap/Python runtime is unavailable")

        with tempfile.TemporaryDirectory(prefix="mission012-sandbox-adapter-test-") as directory:
            root = Path(directory)
            host = root / "fake_mcp_host.py"
            host.write_text(FAKE_MCP_HOST, encoding="utf-8")
            calls_file = root / "mcp-calls.jsonl"
            config = root / "mcp-config.json"
            config.write_text(
                json.dumps({
                    "mcpServers": {
                        "mission012": {
                            "command": str(PYTHON),
                            "args": [str(host), "--calls-file", str(calls_file)],
                            "env": {"IGNORED_BY_WRAPPER": "true"},
                        }
                    }
                }),
                encoding="utf-8",
            )
            config.chmod(0o600)

            result = subprocess.run(
                [str(PYTHON), str(WRAPPER), "--mcp-config", str(config), "--capture-host-stderr"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8"},
                close_fds=True,
                timeout=45,
                check=False,
                text=True,
            )

            self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
            events = [json.loads(line) for line in result.stdout.splitlines() if line]
            self.assertEqual(len(events), 1)
            self.assertEqual(events[0].get("type"), "result")
            self.assertEqual(events[0].get("subtype"), "success")
            self.assertFalse(events[0].get("is_error"))
            sandbox_checks = events[0].get("sandbox_checks")
            self.assertIsInstance(sandbox_checks, dict)
            assert isinstance(sandbox_checks, dict)
            self.assertIsInstance(sandbox_checks.get("probe_target_port"), int)
            self.assertNotEqual(sandbox_checks.get("network_namespace"), os.readlink("/proc/self/ns/net"))
            self.assertNotEqual(sandbox_checks.get("pid_namespace"), os.readlink("/proc/self/ns/pid"))
            self.assertEqual(
                sandbox_checks.get("environment_keys"),
                ["HOME", "LANG", "LC_ALL", "PATH", "PWD", "PYTHONDONTWRITEBYTECODE"],
            )
            self.assertEqual(
                {
                    key: value for key, value in sandbox_checks.items()
                    if key not in {"probe_target_port", "network_namespace", "pid_namespace", "environment_keys"}
                },
                {
                    "credential_visible": False,
                    "network_connected": False,
                    "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
                    "open_fds": [0, 1, 2],
                    "supervisor_observed_route_connections": 0,
                },
            )
            self.assertIn("test protégé vert", events[0].get("result", ""))

            methods = calls_file.read_text(encoding="utf-8").splitlines()
            self.assertEqual(
                methods,
                [
                    "initialize",
                    "tools/list",
                    "tools/call:mission_context",
                    "tools/call:cargo_test",
                    "tools/call:workspace_read_text",
                    "tools/call:workspace_read_text",
                    "tools/call:workspace_write_text",
                    "tools/call:cargo_test",
                ],
            )
            self.assertEqual(result.stderr, "")

            interrupted = subprocess.run(
                [
                    str(PYTHON), str(WRAPPER), "--mcp-config", str(config),
                    "--interrupt-after-write",
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8"},
                close_fds=True,
                timeout=45,
                check=False,
                text=True,
            )
            self.assertEqual(interrupted.returncode, 75, msg=interrupted.stdout + interrupted.stderr)
            interrupted_event = json.loads(interrupted.stdout)
            self.assertEqual(interrupted_event.get("subtype"), "interrupted")
            interrupted_checks = interrupted_event.get("sandbox_checks")
            self.assertIsInstance(interrupted_checks, dict)
            assert isinstance(interrupted_checks, dict)
            self.assertIsInstance(interrupted_checks.get("probe_target_port"), int)
            self.assertNotEqual(interrupted_checks.get("network_namespace"), os.readlink("/proc/self/ns/net"))
            self.assertNotEqual(interrupted_checks.get("pid_namespace"), os.readlink("/proc/self/ns/pid"))
            self.assertEqual(
                {
                    key: value for key, value in interrupted_checks.items()
                    if key not in {"probe_target_port", "network_namespace", "pid_namespace", "environment_keys"}
                },
                {
                    "credential_visible": False,
                    "network_connected": False,
                    "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
                    "open_fds": [0, 1, 2],
                    "supervisor_observed_route_connections": 0,
                },
            )
            interrupted_methods = calls_file.read_text(encoding="utf-8").splitlines()
            self.assertEqual(interrupted_methods[-1], "tools/call:workspace_write_text")
            self.assertEqual(interrupted_methods.count("tools/call:cargo_test"), 1)

    def test_supervisor_cleanup_failure_survives_full_mcp_sandbox_path(self) -> None:
        if not Path("/usr/bin/bwrap").is_file() or not PYTHON.is_file():
            self.skipTest("local Bubblewrap/Python runtime is unavailable")

        with tempfile.TemporaryDirectory(prefix="mission012-supervisor-status76-") as directory:
            root = Path(directory)
            host = root / "hanging_mcp_host.py"
            host_source = FAKE_MCP_HOST.replace(
                "Path(args.calls_file).write_text(",
                "Path(args.calls_file).with_suffix('.pid').write_text("
                "str(__import__('os').getpid()), encoding='utf-8')\n"
                "Path(args.calls_file).write_text(",
            )
            host.write_text(
                host_source + "\nimport time\nwhile True:\n    time.sleep(60)\n",
                encoding="utf-8",
            )
            calls_file = root / "mcp-calls.txt"
            scope_marker = root / "injected-scope.txt"
            supervisor = ROOT / "scripts/mission_012_mcp_supervisor.py"
            shim = root / "supervisor_with_injected_cleanup_refusal.py"
            shim.write_text(
                "\n".join(
                    [
                        "import importlib.util",
                        "import os",
                        "from pathlib import Path",
                        "import sys",
                        f"supervisor_path = Path({str(supervisor)!r})",
                        "spec = importlib.util.spec_from_file_location('mission_012_mcp_supervisor', supervisor_path)",
                        "module = importlib.util.module_from_spec(spec)",
                        "assert spec.loader is not None",
                        "spec.loader.exec_module(module)",
                        "uid = os.getuid()",
                        "module.HOST_FINALIZATION_TIMEOUT_SECONDS = 0.15",
                        "module.HOST_GROUP_TERMINATION_GRACE_SECONDS = 0.05",
                        "module.verifier_scope_paths = lambda _pid: [f'/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/run-test.scope']",
                        f"scope_marker = Path({str(scope_marker)!r})",
                        "def reject_scope_kill(path):",
                        "    scope_marker.write_text(path, encoding='utf-8')",
                        "    return False",
                        "module.kill_verifier_scope = reject_scope_kill",
                        "sys.argv = [str(supervisor_path), '--', *sys.argv[3:]]",
                        "raise SystemExit(module.main())",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            config = root / "mcp-config.json"
            config.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "mission012": {
                                "command": str(PYTHON),
                                "args": [
                                    str(shim),
                                    str(supervisor),
                                    "--",
                                    str(PYTHON),
                                    str(host),
                                    "--calls-file",
                                    str(calls_file),
                                ],
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            config.chmod(0o600)

            output = io.StringIO()
            with (
                mock.patch.object(
                    sys,
                    "argv",
                    [str(WRAPPER), "--mcp-config", str(config)],
                ),
                redirect_stdout(output),
            ):
                status = sandbox_wrapper.main()

            self.assertEqual(status, 1)
            event = json.loads(output.getvalue())
            self.assertEqual(event["type"], "error")
            self.assertNotEqual(event.get("subtype"), "success")
            self.assertIn("exit 76", event["error"])
            self.assertIn("verifier-scope cleanup", event["error"])
            self.assertIn("inconclusive", event["error"])
            self.assertTrue(scope_marker.is_file(), "the injected cleanup refusal was not reached")
            self.assertEqual(
                scope_marker.read_text(encoding="utf-8"),
                f"/user.slice/user-{os.getuid()}.slice/user@{os.getuid()}.service/app.slice/run-test.scope",
            )
            methods = calls_file.read_text(encoding="utf-8").splitlines()
            self.assertEqual(methods.count("tools/call:cargo_test"), 2)
            self.assertEqual(methods[-1], "tools/call:cargo_test")
            host_pid = int(calls_file.with_suffix(".pid").read_text(encoding="ascii"))
            with self.assertRaises(ProcessLookupError):
                os.kill(host_pid, 0)

    def test_wrapper_rejects_non_private_mcp_config_before_spawning_host(self) -> None:
        with tempfile.TemporaryDirectory(prefix="mission012-sandbox-config-test-") as directory:
            config = Path(directory) / "mcp-config.json"
            config.write_text('{"mcpServers":{}}', encoding="utf-8")
            config.chmod(0o644)
            result = subprocess.run(
                [str(PYTHON), str(WRAPPER), "--mcp-config", str(config)],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8"},
                close_fds=True,
                timeout=10,
                check=False,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("private regular file", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
