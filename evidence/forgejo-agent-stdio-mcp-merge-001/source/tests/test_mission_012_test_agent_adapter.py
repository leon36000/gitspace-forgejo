from __future__ import annotations

from contextlib import redirect_stdout
import json
from pathlib import Path
import sys
from unittest import mock
import io
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import mission_012_test_agent_adapter as adapter  # noqa: E402


class FakeMCPClient:
    instances: list["FakeMCPClient"] = []

    def __init__(self, *_args: object, **_kwargs: object) -> None:
        self.instances.append(self)
        self.calls: list[tuple[str, dict[str, object]]] = []
        self.cargo_test_calls = 0
        self.mission_context_calls = 0

    def request(self, method: str, _params: dict[str, object] | None = None) -> dict[str, object]:
        if method == "initialize":
            return {"protocolVersion": "2025-11-25"}
        if method == "tools/list":
            return {"tools": [{"name": name} for name in adapter.MCP_TOOLS]}
        raise AssertionError(f"unexpected MCP request {method}")

    def notify(self, _method: str) -> None:
        return None

    def call_tool(
        self, name: str, arguments: dict[str, object]
    ) -> tuple[dict[str, object], bool]:
        self.calls.append((name, arguments))
        if name == "mission_context":
            self.mission_context_calls += 1
            return (
                {
                    "requirements_json": json.dumps(
                        {"request": "Clamp values above the maximum boundary."}
                    ),
                    "resume_checkpoint": None,
                },
                False,
            )
        if name == "cargo_test":
            self.cargo_test_calls += 1
            if self.cargo_test_calls == 1:
                return (
                    {
                        "exit_code": 101,
                        "expected_baseline_failure": True,
                        "stdout": "left: -2\nright: 10",
                        "stderr": "",
                    },
                    False,
                )
            return {"exit_code": 0, "verified_acceptance_pass": True}, False
        if name == "workspace_read_text" and arguments.get("path") == "src/lib.rs":
            return {"content": f"fn clamp() {{ {adapter.SOURCE_BUG} }}"}, False
        if name == "workspace_read_text" and arguments.get("path") == "tests/acceptance.rs":
            return {
                "content": "values_outside_the_range_clamp_to_the_nearest_boundary"
            }, False
        if name == "workspace_write_text":
            return {}, False
        raise AssertionError(f"unexpected MCP tool call {name}: {arguments}")

    def close(self) -> int:
        return 76

    def abort(self) -> None:
        return None


class Mission012TestAgentAdapterTests(unittest.TestCase):
    def test_optional_gate_barrier_occurs_after_green_acceptance(self) -> None:
        FakeMCPClient.instances.clear()
        with (
            mock.patch.object(adapter, "MCPClient", FakeMCPClient),
            mock.patch.object(FakeMCPClient, "close", return_value=0),
        ):
            result, interrupted = adapter.run(
                Path("/unused/config.json"), stdio_mcp=True, wait_at_gate_barrier=True
            )

        self.assertFalse(interrupted)
        self.assertIn("barrière gate libérée", result)
        calls = FakeMCPClient.instances[0].calls
        self.assertEqual(calls[-2:], [("cargo_test", {}), ("mission_context", {})])
        self.assertEqual(FakeMCPClient.instances[0].mission_context_calls, 2)
        self.assertEqual(FakeMCPClient.instances[0].cargo_test_calls, 2)

    def test_nonzero_supervisor_cleanup_status_becomes_adapter_error_after_verification(self) -> None:
        FakeMCPClient.instances.clear()
        output = io.StringIO()
        with (
            mock.patch.object(adapter, "MCPClient", FakeMCPClient),
            mock.patch.object(
                sys,
                "argv",
                ["mission_012_test_agent_adapter", "--mcp-config", "/unused/config.json"],
            ),
            redirect_stdout(output),
        ):
            status = adapter.main()

        self.assertEqual(status, 1)
        event = json.loads(output.getvalue())
        self.assertEqual(event["type"], "error")
        self.assertIn("verifier-scope cleanup", event["error"])
        self.assertNotEqual(event.get("subtype"), "success")
        self.assertNotEqual(event.get("subtype"), "interrupted")
        self.assertEqual(FakeMCPClient.instances[0].cargo_test_calls, 2)
        self.assertEqual(
            [name for name, _ in FakeMCPClient.instances[0].calls],
            [
                "mission_context",
                "cargo_test",
                "workspace_read_text",
                "workspace_read_text",
                "workspace_write_text",
                "cargo_test",
            ],
        )


if __name__ == "__main__":
    unittest.main()
