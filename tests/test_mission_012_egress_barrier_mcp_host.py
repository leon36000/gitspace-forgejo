from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import mission_012_egress_barrier_mcp_host as mcp_host  # noqa: E402


class EgressBarrierMcpHostTests(unittest.TestCase):
    def test_second_context_read_is_stable_and_holds_only_after_acceptance(self) -> None:
        args = argparse.Namespace(
            barrier_on_context_call=2,
            barrier_ready=Path("/tmp/mission012-ready"),
            barrier_release=Path("/tmp/mission012-release"),
        )
        state: dict[str, bool | int] = {"written": True, "mission_context_calls": 0}

        with mock.patch.object(mcp_host, "write_barrier", return_value=True) as barrier:
            first = mcp_host.handle_tool("mission_context", {}, args, state)
            barrier.assert_not_called()
            second = mcp_host.handle_tool("mission_context", {}, args, state)

        barrier.assert_called_once_with(args.barrier_ready, args.barrier_release)
        self.assertEqual(first, second)

    def test_stdio_shutdown_returns_a_success_response_and_exits(self) -> None:
        with tempfile.TemporaryDirectory(prefix="mission012-egress-host-shutdown-") as directory:
            root = Path(directory)
            calls = root / "calls.jsonl"
            command = [
                sys.executable,
                str(Path(mcp_host.__file__).resolve()),
                "--workspace", str(root / "workspace"),
                "--calls-file", str(calls),
                "--barrier-ready", str(root / "ready"),
                "--barrier-release", str(root / "release"),
                "--cargo", "/usr/bin/cargo",
                "--toolchain-bin", "/usr/bin",
                "--cargo-home", str(root / "cargo-home"),
                "--home", str(root / "home"),
                "--baseline-log", str(root / "baseline.log"),
                "--corrected-log", str(root / "corrected.log"),
            ]
            completed = subprocess.run(
                command,
                input=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "shutdown", "params": {}}) + "\n",
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env={"PATH": "/usr/bin:/bin", "HOME": str(root)},
                timeout=5,
                check=False,
            )

            self.assertEqual(completed.returncode, 0, msg=completed.stderr)
            response = json.loads(completed.stdout)
            self.assertEqual(response.get("id"), 1)
            self.assertEqual(response.get("result"), {})
            self.assertNotIn("error", response)
            self.assertEqual(calls.read_text(encoding="utf-8"), "shutdown\n")


if __name__ == "__main__":
    unittest.main()
