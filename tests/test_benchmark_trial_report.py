import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

from benchmark_trial_report import build_report


def trial(pair_number: int, arm: str, *, elapsed: float, met: int = 3) -> dict:
    pair_id = f"task-{pair_number:02d}-rep-1"
    trial_id = f"{pair_id}-{arm}"
    arm_number = 0 if arm == "conventional" else 1
    candidate_commit = f"{pair_number * 2 + arm_number + 1:040x}"
    merged_commit = f"{pair_number * 2 + arm_number + 10001:040x}" if met == 4 else None
    base_commit = "1" * 40
    pre_merge_result = "pass" if met == 4 else "fail"
    return {
        "schema_version": 1,
        "campaign_id": "campaign-test",
        "protocol_sha256": "a" * 64,
        "trial_id": trial_id,
        "pair_id": pair_id,
        "task_id": f"task-{pair_number:02d}",
        "replicate": 1,
        "arm": arm,
        "order": "first" if arm == "conventional" else "second",
        "controls": {
            "base_commit": base_commit,
            "request_sha256": "b" * 64,
            "acceptance_oracle_sha256": "c" * 64,
            "agent_runtime_id": f"{arm}-runtime-v1",
            "model_id": "model-v1",
            "model_config_sha256": "d" * 64,
            "model_budget_tokens": 4096,
            "model_budget_usd": 1.0,
        },
        "outcome": {
            "candidate_commit": candidate_commit,
            "merged_commit": merged_commit,
            "human_interventions": 1,
            "elapsed_seconds": elapsed,
            "delivery_state": "merged" if met == 4 else "not_merged",
        },
        "independent_verification": {
            "pre_merge": {
                "base_commit": base_commit,
                "checked_commit": candidate_commit,
                "acceptance_criteria_met": met,
                "acceptance_criteria_total": 4,
                "escaped_regressions": 0,
                "executed": True,
                "verifier_id": "protected-oracle-v1",
                "result": pre_merge_result,
                "run_id": f"pre-{trial_id}",
                "result_sha256": "e" * 64,
            },
            "post_merge": (
                {
                    "base_commit": base_commit,
                    "checked_commit": merged_commit,
                    "acceptance_criteria_met": 4,
                    "acceptance_criteria_total": 4,
                    "escaped_regressions": 0,
                    "executed": True,
                    "verifier_id": "protected-oracle-v1",
                    "result": "pass",
                    "run_id": f"post-{trial_id}",
                    "result_sha256": "a" * 64,
                }
                if met == 4
                else None
            ),
        },
        "usage": {
            "model_invoked": True,
            "input_tokens": 100,
            "output_tokens": 25,
            "cost_usd": 0.02,
            "cost_source": "provider_billing_export",
            "usage_evidence_ref": f"usage/{trial_id}.json",
            "usage_evidence_sha256": "f" * 64,
        },
    }


class BenchmarkTrialReportTests(unittest.TestCase):
    def test_reports_paired_deltas_without_claiming_a_gain(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0, met=2)
        agent_layer = trial(1, "agent_layer", elapsed=90.0, met=3)

        report = build_report([conventional, agent_layer])

        self.assertEqual(report["observed_pairs"], 1)
        self.assertEqual(report["pilot_status"], "incomplete")
        self.assertFalse(report["gain_claim_supported"])
        self.assertEqual(report["paired_differences"]["elapsed_seconds"]["median"], -30.0)
        self.assertEqual(report["paired_differences"]["acceptance_rate"]["median"], 0.25)

    def test_refuses_missing_cost_evidence_instead_of_counting_it_as_zero(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        del agent_layer["usage"]["cost_usd"]

        with self.assertRaisesRegex(ValueError, "usage.cost_usd"):
            build_report([conventional, agent_layer])

    def test_refuses_windows_style_evidence_path_traversal(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        conventional["usage"]["usage_evidence_ref"] = r"..\\private\\usage.json"
        agent_layer = trial(1, "agent_layer", elapsed=90.0)

        with self.assertRaisesRegex(ValueError, "usage_evidence_ref"):
            build_report([conventional, agent_layer])

    def test_refuses_a_pair_missing_one_comparison_arm(self) -> None:
        with self.assertRaisesRegex(ValueError, "exactly one trial per arm"):
            build_report([trial(1, "conventional", elapsed=120.0)])

    def test_refuses_mismatched_base_or_model_controls_inside_a_pair(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        agent_layer["controls"]["base_commit"] = "2" * 40
        agent_layer["independent_verification"]["pre_merge"]["base_commit"] = "2" * 40

        with self.assertRaisesRegex(ValueError, "controls must match within a pair"):
            build_report([conventional, agent_layer])

    def test_refuses_pairs_that_did_not_alternate_arm_order(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        agent_layer["order"] = "first"

        with self.assertRaisesRegex(ValueError, "opposite order positions"):
            build_report([conventional, agent_layer])

    def test_refuses_model_or_budget_changes_between_pairs(self) -> None:
        records = [
            trial(1, "conventional", elapsed=120.0),
            trial(1, "agent_layer", elapsed=90.0),
            trial(2, "conventional", elapsed=120.0),
            trial(2, "agent_layer", elapsed=90.0),
        ]
        for item in records[2:]:
            item["controls"]["model_config_sha256"] = "9" * 64

        with self.assertRaisesRegex(ValueError, "model and budget controls must be consistent"):
            build_report(records)

    def test_keeps_failed_delivery_trials_in_the_paired_summary(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0, met=1)
        agent_layer = trial(1, "agent_layer", elapsed=90.0, met=4)

        report = build_report([conventional, agent_layer])

        self.assertEqual(report["arm_summaries"]["conventional"]["trial_count"], 1)
        self.assertEqual(report["arm_summaries"]["conventional"]["verified_delivery_count"], 0)
        self.assertEqual(report["arm_summaries"]["agent_layer"]["verified_delivery_count"], 1)

    def test_refuses_pre_merge_verification_bound_to_another_candidate(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        agent_layer["independent_verification"]["pre_merge"]["checked_commit"] = "9" * 40

        with self.assertRaisesRegex(ValueError, "checked_commit must match the recorded commit"):
            build_report([conventional, agent_layer])

    def test_refuses_pre_merge_verification_bound_to_another_base(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        agent_layer["independent_verification"]["pre_merge"]["base_commit"] = "9" * 40

        with self.assertRaisesRegex(ValueError, "base_commit must match controls.base_commit"):
            build_report([conventional, agent_layer])

    def test_merged_trial_requires_post_merge_verification_record(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0, met=4)
        agent_layer = trial(1, "agent_layer", elapsed=90.0, met=4)
        agent_layer["independent_verification"]["post_merge"] = None

        with self.assertRaisesRegex(ValueError, "merged trial requires post_merge verification"):
            build_report([conventional, agent_layer])

    def test_post_merge_failure_is_retained_as_unverified_delivery(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0, met=4)
        post_merge = agent_layer["independent_verification"]["post_merge"]
        post_merge["result"] = "fail"
        post_merge["acceptance_criteria_met"] = 3
        post_merge["escaped_regressions"] = 1

        report = build_report([conventional, agent_layer])

        self.assertEqual(report["arm_summaries"]["agent_layer"]["verified_delivery_count"], 0)
        self.assertEqual(report["arm_summaries"]["agent_layer"]["escaped_regressions"], 1)
        self.assertEqual(
            report["arm_summaries"]["agent_layer"]["acceptance_criteria_met"], 3
        )

    def test_post_merge_base_advance_is_reported_separately(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0, met=4)
        agent_layer["independent_verification"]["post_merge"]["base_commit"] = "9" * 40

        report = build_report([conventional, agent_layer])

        self.assertEqual(report["arm_summaries"]["agent_layer"]["post_merge_base_drift_count"], 1)
        self.assertEqual(report["paired_differences"]["post_merge_base_drift"]["median"], 1)

    def test_merge_after_pre_merge_failure_is_reported_as_policy_violation(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0, met=4)
        agent_layer["independent_verification"]["pre_merge"]["result"] = "fail"
        agent_layer["independent_verification"]["pre_merge"]["acceptance_criteria_met"] = 3

        report = build_report([conventional, agent_layer])

        self.assertEqual(
            report["arm_summaries"]["agent_layer"]["merged_without_pre_merge_pass_count"], 1
        )
        self.assertEqual(
            report["paired_differences"]["merged_without_pre_merge_pass"]["median"], 1
        )

    def test_post_merge_verification_must_match_observed_merge_commit(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0, met=4)
        agent_layer = trial(1, "agent_layer", elapsed=90.0, met=4)
        agent_layer["outcome"]["merged_commit"] = "9" * 40

        with self.assertRaisesRegex(ValueError, "checked_commit must match the recorded commit"):
            build_report([conventional, agent_layer])

    def test_refuses_verifier_labeled_as_the_executing_agent(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        agent_layer["independent_verification"]["pre_merge"]["verifier_id"] = (
            agent_layer["controls"]["agent_runtime_id"]
        )

        with self.assertRaisesRegex(
            ValueError, "verifier_id must differ from controls.agent_runtime_id"
        ):
            build_report([conventional, agent_layer])

    def test_refuses_agent_runtime_change_within_an_arm(self) -> None:
        records = [
            trial(1, "conventional", elapsed=120.0),
            trial(1, "agent_layer", elapsed=90.0),
            trial(2, "conventional", elapsed=110.0),
            trial(2, "agent_layer", elapsed=95.0),
        ]
        records[2]["controls"]["agent_runtime_id"] = "conventional-runtime-v2"

        with self.assertRaisesRegex(ValueError, "agent runtime must be consistent within each arm"):
            build_report(records)

    def test_rejects_duplicate_independent_verifier_run_id(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        agent_layer["independent_verification"]["pre_merge"]["run_id"] = (
            conventional["independent_verification"]["pre_merge"]["run_id"]
        )

        with self.assertRaisesRegex(ValueError, "duplicate independent verifier run_id"):
            build_report([conventional, agent_layer])

    def test_exact_calibration_design_is_still_not_confirmatory(self) -> None:
        records = []
        for task_number in range(12):
            for replicate in range(3):
                pair_id = f"task-{task_number:02d}-rep-{replicate + 1}"
                if (task_number * 3 + replicate) % 2:
                    order_by_arm = {"conventional": "second", "agent_layer": "first"}
                else:
                    order_by_arm = {"conventional": "first", "agent_layer": "second"}
                for arm in ("conventional", "agent_layer"):
                    item = trial(task_number, arm, elapsed=100.0)
                    item["pair_id"] = pair_id
                    item["trial_id"] = f"{pair_id}-{arm}"
                    item["task_id"] = f"task-{task_number:02d}"
                    item["replicate"] = replicate + 1
                    item["order"] = order_by_arm[arm]
                    item["independent_verification"]["pre_merge"]["run_id"] = f"pre-{pair_id}-{arm}"
                    records.append(item)

        report = build_report(records)

        self.assertEqual(report["observed_pairs"], 36)
        self.assertEqual(report["pilot_status"], "complete_not_confirmatory")
        self.assertEqual(report["planned_pairs"], 36)
        self.assertEqual(
            report["order_counts"], {"conventional_first": 18, "agent_layer_first": 18}
        )
        self.assertFalse(report["gain_claim_supported"])

    def test_full_pilot_with_one_arm_always_first_is_not_order_balanced(self) -> None:
        records = []
        for task_number in range(12):
            for replicate in range(3):
                pair_id = f"task-{task_number:02d}-rep-{replicate + 1}"
                for arm, order in (("conventional", "first"), ("agent_layer", "second")):
                    item = trial(task_number, arm, elapsed=100.0)
                    item["pair_id"] = pair_id
                    item["trial_id"] = f"{pair_id}-{arm}"
                    item["task_id"] = f"task-{task_number:02d}"
                    item["replicate"] = replicate + 1
                    item["order"] = order
                    item["independent_verification"]["pre_merge"]["run_id"] = f"pre-{pair_id}-{arm}"
                    records.append(item)

        report = build_report(records)

        self.assertEqual(report["pilot_status"], "complete_but_order_unbalanced_not_confirmatory")
        self.assertEqual(report["order_counts"]["conventional_first"], 36)
        self.assertEqual(report["order_counts"]["agent_layer_first"], 0)

    def test_cli_emits_the_same_descriptive_report_as_the_library(self) -> None:
        records = [trial(1, "conventional", elapsed=120.0), trial(1, "agent_layer", elapsed=90.0)]
        repository = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as temporary:
            input_path = Path(temporary) / "trials.jsonl"
            input_path.write_text("".join(json.dumps(item) + "\n" for item in records))
            completed = subprocess.run(
                [
                    sys.executable,
                    str(repository / "scripts/benchmark_trial_report.py"),
                    "--input",
                    str(input_path),
                ],
                cwd=repository,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(completed.stdout)
        self.assertEqual(report["observed_pairs"], 1)
        self.assertFalse(report["evidence_hashes_verified"])
        self.assertFalse(report["gain_claim_supported"])

    def test_rejects_duplicate_trial_ids(self) -> None:
        conventional = trial(1, "conventional", elapsed=120.0)
        agent_layer = trial(1, "agent_layer", elapsed=90.0)
        duplicate = copy.deepcopy(agent_layer)

        with self.assertRaisesRegex(ValueError, "duplicate trial_id"):
            build_report([conventional, agent_layer, duplicate])

    def test_rejects_duplicate_task_replicate_under_different_pair_ids(self) -> None:
        records = [
            trial(1, "conventional", elapsed=120.0),
            trial(1, "agent_layer", elapsed=90.0),
            trial(2, "conventional", elapsed=110.0),
            trial(2, "agent_layer", elapsed=95.0),
        ]
        for item in records[2:]:
            item["pair_id"] = "renamed-pair"
            item["trial_id"] = f"renamed-pair-{item['arm']}"
            item["task_id"] = records[0]["task_id"]

        with self.assertRaisesRegex(ValueError, "duplicate task_id/replicate"):
            build_report(records)

    def test_refuses_task_material_change_between_replicates(self) -> None:
        records = []
        for replicate in (1, 2):
            pair_id = f"stable-task-rep-{replicate}"
            for arm in ("conventional", "agent_layer"):
                item = trial(replicate, arm, elapsed=100.0)
                item["pair_id"] = pair_id
                item["trial_id"] = f"{pair_id}-{arm}"
                item["task_id"] = "stable-task"
                item["replicate"] = replicate
                item["independent_verification"]["pre_merge"]["run_id"] = f"pre-{pair_id}-{arm}"
                records.append(item)
        for item in records[2:]:
            item["controls"]["request_sha256"] = "9" * 64

        with self.assertRaisesRegex(ValueError, "content controls must be consistent"):
            build_report(records)

    def test_36_pairs_with_wrong_task_replicate_shape_are_not_complete(self) -> None:
        records = []
        for task_number in range(9):
            for replicate in range(1, 5):
                pair_id = f"task-{task_number:02d}-rep-{replicate}"
                order_by_arm = (
                    {"conventional": "first", "agent_layer": "second"}
                    if len(records) % 4 < 2
                    else {"conventional": "second", "agent_layer": "first"}
                )
                for arm in ("conventional", "agent_layer"):
                    item = trial(task_number, arm, elapsed=100.0)
                    item["pair_id"] = pair_id
                    item["trial_id"] = f"{pair_id}-{arm}"
                    item["task_id"] = f"task-{task_number:02d}"
                    item["replicate"] = replicate
                    item["order"] = order_by_arm[arm]
                    item["independent_verification"]["pre_merge"]["run_id"] = f"pre-{pair_id}-{arm}"
                    records.append(item)

        report = build_report(records)

        self.assertEqual(report["observed_pairs"], 36)
        self.assertEqual(report["pilot_status"], "complete_but_design_mismatch_not_confirmatory")
        self.assertFalse(report["gain_claim_supported"])


if __name__ == "__main__":
    unittest.main()
