from __future__ import annotations

from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
import sys

sys.path.insert(0, str(ROOT / "scripts"))
import semantic_merge_canary as canary  # noqa: E402


class SemanticMergeCanaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.rustc = shutil.which("rustc")
        if self.rustc is None:
            self.skipTest("rustc is required for the semantic merge fixture")
        self.temporary = tempfile.TemporaryDirectory(prefix="semantic-merge-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def run_acceptance(self, source: bytes, test_source: bytes, name: str) -> subprocess.CompletedProcess[bytes]:
        candidate = self.root / f"{name}-candidate.rs"
        test = self.root / f"{name}-acceptance.rs"
        executable = self.root / f"{name}-acceptance"
        candidate.write_bytes(source)
        test.write_bytes(test_source.replace(b"@CANDIDATE@", str(candidate).encode()))
        built = subprocess.run(
            [self.rustc, "--edition=2024", "--test", str(test), "-o", str(executable)],
            cwd=self.root,
            capture_output=True,
            check=False,
        )
        self.assertEqual(built.returncode, 0, built.stdout + built.stderr)
        return subprocess.run([str(executable)], cwd=self.root, capture_output=True, check=False)

    def test_independent_changes_merge_cleanly_but_combined_acceptance_fails_then_recovers(self) -> None:
        sources = canary.build_semantic_canary_sources()

        lower = self.run_acceptance(
            sources["lower"], canary.semantic_acceptance_source("lower"), "lower"
        )
        upper = self.run_acceptance(
            sources["upper"], canary.semantic_acceptance_source("upper"), "upper"
        )
        self.assertEqual(lower.returncode, 0, lower.stdout + lower.stderr)
        self.assertEqual(upper.returncode, 0, upper.stdout + upper.stderr)

        combined_lower = self.run_acceptance(
            sources["combined"], canary.semantic_acceptance_source("lower"), "combined-lower"
        )
        combined_upper = self.run_acceptance(
            sources["combined"], canary.semantic_acceptance_source("upper"), "combined-upper"
        )
        protected_red = self.run_acceptance(
            sources["combined"], canary.semantic_acceptance_source("protected"), "protected-red"
        )
        self.assertEqual(combined_lower.returncode, 0, combined_lower.stdout + combined_lower.stderr)
        self.assertEqual(combined_upper.returncode, 0, combined_upper.stdout + combined_upper.stderr)
        self.assertNotEqual(protected_red.returncode, 0)
        self.assertIn(b"min > max", protected_red.stdout + protected_red.stderr)

        protected_green = self.run_acceptance(
            sources["corrected"], canary.semantic_acceptance_source("protected"), "protected-green"
        )
        self.assertEqual(protected_green.returncode, 0, protected_green.stdout + protected_green.stderr)


if __name__ == "__main__":
    unittest.main()
