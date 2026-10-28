import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from run_forgejo_base_freshness_race_e2e import classify_race_result


def valid_refusal():
    return {
        "main_sha_before_race": "base-before",
        "main_sha_after_concurrent_merge_before_target_merge": "base-after",
        "main_sha_after_target_attempt": "base-after",
        "target_head_sha_at_last_read": "target-head",
        "target_merge_commit": None,
        "target_pull_request_after_attempt": {
            "state": "open",
            "merged": False,
            "base_sha": "base-after",
            "head_sha": "target-head",
        },
        "concurrent_pull_request_approved_head": "other-head",
        "concurrent_pull_request_review": {
            "id": 11,
            "state": "APPROVED",
            "commit_id": "other-head",
            "reviewer_login": "forge-reviewer",
        },
        "concurrent_pull_request_merge_http_status": 200,
        "concurrent_pull_request_after": {
            "state": "closed",
            "merged": True,
            "head_sha": "other-head",
            "merge_commit_sha": "base-after",
        },
        "target_merge_attempts": [
            {
                "label": "immediate_after_base_advance",
                "http_status": 405,
                "reason": "Please try again later",
            },
            {
                "label": "after_forgejo_mergeability_readback",
                "http_status": 405,
                "reason": "The head branch is behind the base branch",
            },
        ],
        "mergeability_readbacks_after_initial_refusal": [
            {
                "mergeable": False,
                "base_sha": "base-after",
                "head_sha": "target-head",
                "state": "open",
            },
            {
                "mergeable": True,
                "base_sha": "base-after",
                "head_sha": "target-head",
                "state": "open",
            },
        ],
    }


class ClassificationTests(unittest.TestCase):
    def test_requires_approved_competitor_and_post_readback_stale_base_reason(self):
        self.assertEqual(classify_race_result(valid_refusal()), "stale_base_merge_refused")

    def test_generic_error_after_race_is_not_a_verified_stale_base_refusal(self):
        result = valid_refusal()
        result["target_merge_attempts"][-1]["reason"] = "some unrelated HTTP error"
        self.assertEqual(
            classify_race_result(result), "stale_base_merge_refused_unclassified"
        )

    def test_accepts_forgejo_wrapped_reason_without_widening_the_match(self):
        result = valid_refusal()
        result["target_merge_attempts"][-1]["reason"] = (
            "not allowed to merge [reason: The head branch is behind the base branch]"
        )
        self.assertEqual(classify_race_result(result), "stale_base_merge_refused")

    def test_reports_a_merge_as_accepted_only_with_concurrent_advance_evidence(self):
        result = valid_refusal()
        result["target_pull_request_after_attempt"].update(
            {"state": "closed", "merged": True}
        )
        self.assertEqual(classify_race_result(result), "stale_base_merge_accepted")

        result["concurrent_pull_request_after"]["merged"] = False
        self.assertEqual(classify_race_result(result), "inconsistent_readback")

    def test_requires_mergeable_readback_before_retry(self):
        result = valid_refusal()
        result["mergeability_readbacks_after_initial_refusal"] = [
            {
                "mergeable": False,
                "base_sha": "base-after",
                "head_sha": "target-head",
                "state": "open",
            }
        ]
        self.assertEqual(
            classify_race_result(result), "stale_base_merge_refused_unclassified"
        )

    def test_requires_expected_http_405_and_competitor_merge_commit(self):
        result = valid_refusal()
        result["target_merge_attempts"][-1]["http_status"] = 500
        self.assertEqual(
            classify_race_result(result), "stale_base_merge_refused_unclassified"
        )

        result = valid_refusal()
        result["concurrent_pull_request_after"]["merge_commit_sha"] = "some-other-sha"
        self.assertEqual(
            classify_race_result(result), "stale_base_merge_refused_unclassified"
        )


if __name__ == "__main__":
    unittest.main()
