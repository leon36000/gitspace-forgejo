"""Keep the webhook process runner on the allowlisted sandbox path."""

from __future__ import annotations

import ast
from pathlib import Path
import unittest


RUNNER = Path(__file__).with_name("run_webhook_reconcile_cli_e2e.py")


def call_names(source: str) -> set[str]:
    tree = ast.parse(source)
    names: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            names.add(ast.unparse(node.func))
    return names


class WebhookRunnerSandboxTests(unittest.TestCase):
    def test_webhook_runner_delegates_build_and_execution_to_allowlisted_runner(self) -> None:
        source = RUNNER.read_text(encoding="utf-8")
        calls = call_names(source)

        self.assertIn("sandbox.copy_source_tree", source)
        self.assertIn("sandbox.copy_locked_registry", source)
        self.assertIn("sandbox.compile_targets", calls)
        self.assertIn("sandbox.isolated_command", calls)
        self.assertIn("sandbox.run", calls)
        self.assertNotIn('"--ro-bind"', source)
        self.assertNotIn("run([cargo", source)


if __name__ == "__main__":
    unittest.main()
