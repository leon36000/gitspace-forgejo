from __future__ import annotations

import argparse
import sys
from pathlib import Path
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


if __name__ == "__main__":
    unittest.main()
