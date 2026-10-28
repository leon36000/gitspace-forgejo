from __future__ import annotations

import argparse
from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import run_mission_012_supervised_private_netns_e2e as runner  # noqa: E402


class SupervisedMission012RunnerTests(unittest.TestCase):
    def test_success_path_keeps_same_host_gate_barrier(self) -> None:
        args = argparse.Namespace(
            state_dir="/tmp/state",
            evidence_dir="/repo/evidence/mission-012-live-agent-webhook-success",
            execution_id="live-mcp-223",
            allocation_id="mission012-success",
        )

        command = runner._composed_runner_arguments(
            args, "net:[123]", Path("/run/source-git/common")
        )

        self.assertIn("--same-host-gate-barrier", command)
        self.assertNotIn("--test-host-restart", command)

    def test_host_restart_mode_omits_incompatible_gate_barrier(self) -> None:
        args = argparse.Namespace(
            state_dir="/tmp/state",
            evidence_dir="/repo/evidence/mission-012-live-agent-webhook-restart",
            execution_id="live-mcp-224",
            allocation_id="mission012-restart",
            test_host_restart=True,
        )

        command = runner._composed_runner_arguments(
            args, "net:[123]", Path("/run/source-git/common")
        )

        self.assertIn("--test-host-restart", command)
        self.assertNotIn("--same-host-gate-barrier", command)
        self.assertIn("--reuse-attested-parent-scope", command)
        self.assertIn("--sandbox-test-adapter", command)

    def test_host_restart_verdict_requires_exact_checkpoint_and_new_fence(self) -> None:
        valid = {
            "host_restart_recovery": {
                "crash_point": "after_effect",
                "first_attempt_exit_code": 137,
                "first_attempt_timed_out": False,
                "first_attempt_output_exceeded": False,
                "unresolved_intents_before_restart": 1,
                "results_before_restart": 0,
                "results_after_restart": 1,
                "resume_checkpoint_status": "unverified",
                "old_fencing_token": 4,
                "new_fencing_token": 5,
                "initial_lease_id": "mission012-initial",
                "recovery_lease_id": "mission012-recovery",
                "initial_receipt_count": 1,
                "recovery_receipt_count": 2,
                "candidate_source_sha256_after_effect": "a" * 64,
                "initial_receipt_sha256": "b" * 64,
                "recovery_receipt_sha256": "c" * 64,
                "combined_receipt_sha256": "d" * 64,
            }
        }
        self.assertTrue(runner._host_restart_recovery_is_verified(valid))

        for field, value in (
            ("resume_checkpoint_status", "verified"),
            ("results_before_restart", 1),
            ("results_after_restart", 2),
            ("new_fencing_token", 4),
            ("recovery_lease_id", "mission012-initial"),
            ("combined_receipt_sha256", "not-a-digest"),
        ):
            with self.subTest(field=field):
                invalid = {"host_restart_recovery": dict(valid["host_restart_recovery"])}
                invalid["host_restart_recovery"][field] = value
                self.assertFalse(runner._host_restart_recovery_is_verified(invalid))


if __name__ == "__main__":
    unittest.main()
