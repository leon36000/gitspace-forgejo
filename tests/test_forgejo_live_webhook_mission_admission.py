from __future__ import annotations

import base64
import hashlib
import importlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))


class LiveWebhookMissionAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.driver = importlib.import_module("run_forgejo_live_webhook_mission_admission")

    def test_delivery_cli_arguments_pin_the_receiver_attestation_key(self) -> None:
        public_key = "ab" * 32

        arguments = self.driver.delivery_attestation_arguments(
            "delivery-001", "cd" * 32, public_key
        )

        self.assertEqual(
            arguments,
            [
                "--delivery-id", "delivery-001",
                "--requirement-sha256", "cd" * 32,
                "--trusted-webhook-attestation-public-key-hex", public_key,
            ],
        )

    def test_delivery_cli_arguments_reject_noncanonical_attestation_keys(self) -> None:
        for public_key in ("", "AB" * 32, "ab" * 31, "zz" * 32):
            with self.subTest(public_key=public_key):
                with self.assertRaisesRegex(ValueError, "attestation public key"):
                    self.driver.delivery_attestation_arguments(
                        "delivery-001", "cd" * 32, public_key
                    )

    def test_binds_one_successful_forgejo_task_to_its_receiver_delivery(self) -> None:
        validate = self.driver.validate_single_hook_task_binding
        task = {
            "uuid": "forgejo-delivery-001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
        }
        event = {
            "sequence": 1,
            "delivery_id": "forgejo-delivery-001",
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
        }

        self.assertEqual(
            validate([task], event),
            {
                "forgejo_delivery_uuid": "forgejo-delivery-001",
                "receiver_delivery_id": "forgejo-delivery-001",
                "receiver_sequence": 1,
                "response_status": 202,
            },
        )

    def test_binds_issue_comment_task_to_the_real_journal_delivery(self) -> None:
        task = {
            "uuid": "forgejo-comment-delivery-001",
            "event_type": "issue_comment",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
        }
        event = {
            "sequence": 2,
            "delivery_id": "forgejo-comment-delivery-001",
            "event": "issue_comment",
            "action": "created",
            "issue_number": 1,
            "comment_id": 7,
            "is_pull": False,
        }

        self.assertEqual(
            self.driver.validate_single_issue_comment_task_binding([task], event),
            {
                "forgejo_delivery_uuid": "forgejo-comment-delivery-001",
                "receiver_delivery_id": "forgejo-comment-delivery-001",
                "receiver_sequence": 2,
                "response_status": 202,
            },
        )

    def test_issue_comment_task_binding_rejects_unacknowledged_or_wrong_event(self) -> None:
        task = {
            "uuid": "forgejo-comment-delivery-001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
        }
        event = {
            "sequence": 2,
            "delivery_id": "forgejo-comment-delivery-001",
            "event": "issue_comment",
            "action": "created",
            "issue_number": 1,
            "comment_id": 7,
            "is_pull": False,
        }

        with self.assertRaises(ValueError):
            self.driver.validate_single_issue_comment_task_binding([task], event)

    def test_native_issue_comment_refusal_requires_unchanged_claim_and_requirement(self) -> None:
        event = {
            "sequence": 2,
            "delivery_id": "forgejo-comment-delivery-001",
            "event": "issue_comment",
            "action": "created",
            "issue_number": 1,
            "comment_id": 7,
            "is_pull": False,
        }
        binding = {
            "forgejo_delivery_uuid": "forgejo-comment-delivery-001",
            "receiver_delivery_id": "forgejo-comment-delivery-001",
            "receiver_sequence": 2,
            "response_status": 202,
        }
        state = {
            ".runtime/host-admission.json": b"existing-admission",
            ".runtime/mission-execution.jsonl": b"existing-journal",
        }

        result = self.driver.validate_native_issue_comment_refusal(
            1,
            {"error": {"code": "webhook_delivery_resource_mismatch"}},
            event=event,
            task_binding=binding,
            project_state_before=state,
            project_state_after=dict(state),
            requirement_before=b"versioned requirement",
            requirement_after=b"versioned requirement",
        )

        self.assertEqual(result["verdict"], "refused")
        self.assertIs(result["comment_allocation_or_claim_created"], False)
        self.assertIs(result["existing_admission_unchanged"], True)
        self.assertIs(result["requirement_unchanged"], True)

    def test_native_issue_comment_refusal_rejects_state_change_and_wrong_error(self) -> None:
        event = {
            "sequence": 2,
            "delivery_id": "forgejo-comment-delivery-001",
            "event": "issue_comment",
            "action": "created",
            "issue_number": 1,
            "comment_id": 7,
            "is_pull": False,
        }
        binding = {
            "forgejo_delivery_uuid": "forgejo-comment-delivery-001",
            "receiver_delivery_id": "forgejo-comment-delivery-001",
            "receiver_sequence": 2,
            "response_status": 202,
        }
        common = {
            "event": event,
            "task_binding": binding,
            "project_state_before": {".runtime/host-admission.json": b"before"},
            "requirement_before": b"requirement",
            "requirement_after": b"requirement",
        }

        with self.assertRaises(ValueError):
            self.driver.validate_native_issue_comment_refusal(
                1,
                {"error": {"code": "webhook_delivery_resource_mismatch"}},
                **common,
                project_state_after={".runtime/host-admission.json": b"changed"},
            )
        with self.assertRaises(ValueError):
            self.driver.validate_native_issue_comment_refusal(
                0,
                {"verdict": "admitted"},
                **common,
                project_state_after=common["project_state_before"],
            )

    def test_binds_native_issue_edit_task_to_the_edited_receiver_delivery(self) -> None:
        task = {
            "uuid": "forgejo-edited-delivery-002",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
        }
        event = {
            "sequence": 2,
            "delivery_id": "forgejo-edited-delivery-002",
            "event": "issues",
            "action": "edited",
            "issue_number": 1,
            "payload_state": "open",
        }

        self.assertEqual(
            self.driver.validate_single_edited_issue_task_binding([task], event),
            {
                "forgejo_delivery_uuid": "forgejo-edited-delivery-002",
                "receiver_delivery_id": "forgejo-edited-delivery-002",
                "receiver_sequence": 2,
                "response_status": 202,
                "issue_number": 1,
            },
        )

    def test_native_issue_edit_conflict_refusal_requires_unchanged_claim_and_inbox(self) -> None:
        event = {
            "sequence": 2,
            "delivery_id": "forgejo-edited-delivery-002",
            "event": "issues",
            "action": "edited",
            "issue_number": 1,
            "payload_state": "open",
        }
        binding = {
            "forgejo_delivery_uuid": "forgejo-edited-delivery-002",
            "receiver_delivery_id": "forgejo-edited-delivery-002",
            "receiver_sequence": 2,
            "response_status": 202,
            "issue_number": 1,
        }
        state = {
            ".runtime/host-admission.json": b"existing-admission",
            ".runtime/mission-execution.jsonl": b"existing-journal",
        }
        inbox = b"opened delivery\nedited delivery\n"

        result = self.driver.validate_native_issue_edit_conflict(
            1,
            {"verdict": "inconclusive", "error": {"code": "execution_idempotency_conflict"}},
            event=event,
            task_binding=binding,
            project_state_before=state,
            project_state_after=dict(state),
            requirement_before=b"versioned requirement",
            requirement_after=b"versioned requirement",
            inbox_before=inbox,
            inbox_after=inbox,
        )

        self.assertEqual(result["verdict"], "refused")
        self.assertIs(result["existing_admission_unchanged"], True)
        self.assertIs(result["mission_journal_unchanged"], True)
        self.assertIs(result["inbox_unchanged_during_cli"], True)
        self.assertIs(result["requirement_unchanged"], True)

    def test_native_issue_edit_conflict_rejects_wrong_event_error_or_mutation(self) -> None:
        event = {
            "sequence": 2,
            "delivery_id": "forgejo-edited-delivery-002",
            "event": "issues",
            "action": "edited",
            "issue_number": 1,
            "payload_state": "open",
        }
        binding = {
            "forgejo_delivery_uuid": "forgejo-edited-delivery-002",
            "receiver_delivery_id": "forgejo-edited-delivery-002",
            "receiver_sequence": 2,
            "response_status": 202,
            "issue_number": 1,
        }
        common = {
            "event": event,
            "task_binding": binding,
            "project_state_before": {".runtime/host-admission.json": b"before"},
            "requirement_before": b"requirement",
            "requirement_after": b"requirement",
            "inbox_before": b"opened\nedited\n",
            "inbox_after": b"opened\nedited\n",
        }
        with self.assertRaises(ValueError):
            self.driver.validate_native_issue_edit_conflict(
                1,
                {"verdict": "inconclusive", "error": {"code": "wrong_error"}},
                **common,
                project_state_after=common["project_state_before"],
            )
        with self.assertRaises(ValueError):
            self.driver.validate_native_issue_edit_conflict(
                1,
                {"verdict": "inconclusive", "error": {"code": "execution_idempotency_conflict"}},
                **common,
                project_state_after={".runtime/host-admission.json": b"changed"},
            )

    def test_native_issue_edit_conflict_flag_is_opt_in_and_exclusive(self) -> None:
        base = [
            "--forgejo-binary", "/tmp/forgejo",
            "--state-dir", "/tmp/state",
            "--evidence-dir", "/tmp/evidence",
        ]
        self.assertFalse(self.driver.parse_args(base).native_issue_edit_conflict)
        self.assertTrue(
            self.driver.parse_args([*base, "--native-issue-edit-conflict"]).native_issue_edit_conflict
        )
        with self.assertRaises(SystemExit) as error:
            self.driver.parse_args(
                [*base, "--native-issue-edit-conflict", "--native-issue-comment-refusal"]
            )
        self.assertEqual(error.exception.code, 2)

    def test_native_issue_comment_refusal_flag_is_opt_in(self) -> None:
        base = [
            "--forgejo-binary", "/tmp/forgejo",
            "--state-dir", "/tmp/state",
            "--evidence-dir", "/tmp/evidence",
        ]

        self.assertFalse(self.driver.parse_args(base).native_issue_comment_refusal)
        self.assertTrue(
            self.driver.parse_args(
                [*base, "--native-issue-comment-refusal"]
            ).native_issue_comment_refusal
        )

    def test_schema_v3_source_review_mode_is_opt_in_and_exclusive(self) -> None:
        base = [
            "--forgejo-binary", "/tmp/forgejo",
            "--state-dir", "/tmp/state",
            "--evidence-dir", "/tmp/evidence",
        ]

        self.assertFalse(self.driver.parse_args(base).schema_v3_source_review)
        self.assertTrue(
            self.driver.parse_args([*base, "--schema-v3-source-review"]).schema_v3_source_review
        )
        with self.assertRaises(SystemExit) as error:
            self.driver.parse_args(
                [*base, "--schema-v3-source-review", "--native-replay-admission"]
            )
        self.assertEqual(error.exception.code, 2)

    def test_source_review_protection_can_retain_merged_approval_receipt(self) -> None:
        policy = self.driver.mission_branch_protection_policy(
            dismiss_stale_approvals=False
        )

        self.assertIs(policy["dismiss_stale_approvals"], False)
        self.assertEqual(policy["required_approvals"], 1)
        self.assertIs(policy["apply_to_admins"], True)
        self.assertIs(policy["enable_approvals_whitelist"], True)
        self.assertTrue(
            self.driver.branch_protection_matches_policy(
                policy, dismiss_stale_approvals=False
            )
        )

    def test_derives_hook_target_and_listener_port_from_actual_receiver_address(self) -> None:
        self.assertEqual(
            self.driver.receiver_target(("127.0.0.1", 49152)),
            (49152, "http://127.0.0.1:49152/forgejo/webhook"),
        )
        with self.assertRaises(ValueError):
            self.driver.receiver_target(("0.0.0.0", 49152))

    def test_decodes_only_base64_forgejo_contents_readbacks(self) -> None:
        contents = b'{"requirements": []}\n'
        encoded = base64.b64encode(contents).decode("ascii")

        self.assertEqual(
            self.driver.decode_forgejo_contents_readback(
                {"encoding": "base64", "content": encoded}
            ),
            contents,
        )
        with self.assertRaises(ValueError):
            self.driver.decode_forgejo_contents_readback(
                {"encoding": "utf-8", "content": contents.decode("ascii")}
            )
        with self.assertRaises(ValueError):
            self.driver.decode_forgejo_contents_readback(
                {"encoding": "base64", "content": "not base64!"}
            )

    def test_mission_branch_protection_matches_the_reviewer_and_merge_authority(self) -> None:
        policy = self.driver.mission_branch_protection_policy()

        self.assertTrue(self.driver.branch_protection_matches_policy(policy))
        self.assertEqual(policy["approvals_whitelist_username"], ["verifier-bot"])
        self.assertEqual(policy["merge_whitelist_usernames"], [self.driver.OWNER])
        self.assertEqual(policy["merge_whitelist_teams"], [])
        self.assertFalse(
            self.driver.branch_protection_matches_policy(
                {**policy, "merge_whitelist_teams": ["release-managers"]}
            )
        )

    def test_generates_one_ascii_hex_webhook_secret_for_both_receiver_and_forgejo(self) -> None:
        secret = self.driver.generate_webhook_secret()

        self.assertEqual(len(secret), 64)
        self.assertTrue(all(byte in b"0123456789abcdef" for byte in secret))
        self.assertEqual(self.driver.forgejo_secret_value(secret), secret.decode("ascii"))
        with self.assertRaises(ValueError):
            self.driver.forgejo_secret_value(bytes(range(32)))

    def test_issue_hook_deactivation_preserves_the_selected_event(self) -> None:
        self.assertEqual(
            self.driver.emitted.hook_deactivation_patch(("issues",)),
            {"active": False, "events": ["issues"]},
        )

    def test_issue_hook_readback_uses_forgejo_values_and_omits_secret(self) -> None:
        target = "http://127.0.0.1:49152/forgejo/webhook"
        record = self.driver.mission_hook_readback_record(
            7,
            {
                "id": 7,
                "type": "gitea",
                "active": False,
                "events": [
                    "issues",
                    "issue_assign",
                    "issue_label",
                    "issue_milestone",
                    "issue_comment",
                ],
                "config": {
                    "url": target,
                    "content_type": "json",
                    "secret": "DO NOT PERSIST",
                },
            },
            target,
        )

        self.assertEqual(record["target"], target)
        self.assertEqual(record["events"][0], "issues")
        self.assertIs(record["configuration_matches_expected"], True)
        self.assertIs(record["deactivation_readback_confirmed"], True)
        self.assertNotIn("DO NOT PERSIST", str(record))

    def test_issue_hook_readback_rejects_wrong_configuration(self) -> None:
        record = self.driver.mission_hook_readback_record(
            7,
            {
                "id": 7,
                "type": "gitea",
                "active": False,
                "events": ["push"],
                "config": {
                    "url": "http://127.0.0.1:49999/wrong",
                    "content_type": "json",
                },
            },
            "http://127.0.0.1:49152/forgejo/webhook",
        )

        self.assertIs(record["configuration_matches_expected"], False)
        self.assertIs(record["deactivation_readback_confirmed"], True)

    def test_accepts_optional_mcp_host_build_flag(self) -> None:
        args = self.driver.parse_args(
            [
                "--forgejo-binary", "/tmp/forgejo",
                "--state-dir", "/tmp/state",
                "--evidence-dir", "/tmp/evidence",
                "--build-mcp-host",
            ]
        )

        self.assertTrue(args.build_mcp_host)
        self.assertFalse(
            self.driver.parse_args(
                [
                    "--forgejo-binary", "/tmp/forgejo",
                    "--state-dir", "/tmp/state",
                    "--evidence-dir", "/tmp/evidence",
                ]
            ).build_mcp_host
        )

    def test_test_crash_host_build_flag_is_explicit_and_requires_mcp_host(self) -> None:
        base = [
            "--forgejo-binary", "/tmp/forgejo",
            "--state-dir", "/tmp/state",
            "--evidence-dir", "/tmp/evidence",
        ]
        args = self.driver.parse_args(
            [*base, "--build-mcp-host", "--build-test-crash-injection-host"]
        )

        self.assertTrue(args.build_mcp_host)
        self.assertTrue(args.build_test_crash_injection_host)
        self.assertFalse(self.driver.parse_args(base).build_test_crash_injection_host)

        args = self.driver.parse_args(
            [*base, "--build-test-crash-injection-host"]
        )
        with self.assertRaisesRegex(RuntimeError, "requires the MCP host build"):
            self.driver.run(args)

    def test_native_replay_admission_is_opt_in(self) -> None:
        base = [
            "--forgejo-binary", "/tmp/forgejo",
            "--state-dir", "/tmp/state",
            "--evidence-dir", "/tmp/evidence",
        ]

        self.assertFalse(self.driver.parse_args(base).native_replay_admission)
        self.assertTrue(
            self.driver.parse_args([*base, "--native-replay-admission"]).native_replay_admission
        )

    def test_receiver_overload_recovery_mode_requires_native_replay(self) -> None:
        base = [
            "--forgejo-binary", "/tmp/forgejo",
            "--state-dir", "/tmp/state",
            "--evidence-dir", "/tmp/evidence",
            "--receiver-overload-first-delivery",
        ]

        with self.assertRaises(SystemExit) as error:
            self.driver.parse_args(base)
        self.assertEqual(error.exception.code, 2)

        args = self.driver.parse_args([*base, "--native-replay-admission"])
        self.assertTrue(args.receiver_overload_first_delivery)
        self.assertTrue(args.native_replay_admission)

    def test_mcp_host_binary_hash_is_required_and_recorded_when_requested(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp)
            mission = state / "mission"
            receiver = state / "receiver"
            mcp_host = state / "build/host-artifacts/mission-012-mcp-host"
            mcp_host.parent.mkdir(parents=True)
            mission.write_bytes(b"mission")
            receiver.write_bytes(b"receiver")
            mcp_host.write_bytes(b"mcp-host")

            hashes = self.driver.built_executable_hashes(
                state, mission, receiver, include_mcp_host=True
            )

        self.assertEqual(set(hashes), {"forgejo-mission-local", "forgejo-webhook-local", "mission-012-mcp-host"})
        self.assertEqual(hashes["mission-012-mcp-host"], hashlib.sha256(b"mcp-host").hexdigest())

        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp)
            mission = state / "mission"
            receiver = state / "receiver"
            mission.write_bytes(b"mission")
            receiver.write_bytes(b"receiver")
            with self.assertRaisesRegex(RuntimeError, "omitted the expected executable"):
                self.driver.built_executable_hashes(
                    state, mission, receiver, include_mcp_host=True
                )

    def test_refuses_task_without_receiver_acknowledgement(self) -> None:
        task = {
            "uuid": "forgejo-delivery-001",
            "event_type": "issues",
            "is_delivered": False,
            "is_succeed": False,
            "response_status": None,
        }
        event = {
            "sequence": 1,
            "delivery_id": "forgejo-delivery-001",
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
        }

        with self.assertRaises(ValueError):
            self.driver.validate_single_hook_task_binding([task], event)

    def test_waits_until_forgejo_records_the_receiver_response(self) -> None:
        in_flight = {
            "uuid": "forgejo-delivery-001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": False,
            "response_status": None,
        }
        acknowledged = {
            **in_flight,
            "is_succeed": True,
            "response_status": 202,
        }

        with (
            patch.object(
                self.driver.emitted,
                "hook_task_readback",
                side_effect=[[in_flight], [acknowledged]],
            ) as readback,
            patch.object(self.driver.time, "sleep"),
        ):
            tasks = self.driver._wait_for_issue_task(Path("unused.sqlite"), 7, timeout=1.0)

        self.assertEqual(tasks, [acknowledged])
        self.assertEqual(readback.call_count, 2)

    def test_returns_terminal_http_failure_for_actionable_validation(self) -> None:
        rejected = {
            "uuid": "forgejo-delivery-001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": False,
            "response_status": 401,
        }

        with (
            patch.object(
                self.driver.emitted,
                "hook_task_readback",
                return_value=[rejected],
            ) as readback,
            patch.object(self.driver.time, "sleep") as sleep,
        ):
            tasks = self.driver._wait_for_issue_task(Path("unused.sqlite"), 7, timeout=1.0)

        self.assertEqual(tasks, [rejected])
        readback.assert_called_once_with(Path("unused.sqlite"), 7)
        sleep.assert_not_called()

    def test_failure_diagnostic_redacts_host_tokens_and_webhook_secret_encodings(self) -> None:
        secret = b"private-webhook-secret"
        token = "private-host-token"
        detail = self.driver.safe_failure_detail(
            RuntimeError(
                f"failed token={token} secret={secret.hex()} "
                f"base64={base64.b64encode(secret).decode()} "
                f"raw={secret.decode()}"
            ),
            [token],
            secret,
        )

        self.assertIn("failed token=", detail)
        for private_value in (
            token,
            secret.hex(),
            base64.b64encode(secret).decode(),
            secret.decode(),
        ):
            self.assertNotIn(private_value, detail)

    def test_refuses_mismatched_delivery_uuid_or_non_open_event(self) -> None:
        task = {
            "uuid": "forgejo-delivery-other",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
        }
        event = {
            "sequence": 1,
            "delivery_id": "forgejo-delivery-001",
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
        }
        with self.assertRaises(ValueError):
            self.driver.validate_single_hook_task_binding([task], event)

        task["uuid"] = "forgejo-delivery-001"
        event["action"] = "closed"
        event["payload_state"] = "closed"
        with self.assertRaises(ValueError):
            self.driver.validate_single_hook_task_binding([task], event)

    def test_admission_validator_binds_reconciled_proof_to_the_emitted_event(self) -> None:
        validate = self.driver.validate_admission_result
        digest = "ab" * 32
        base = "cd" * 20
        event = {
            "sequence": 1,
            "delivery_id": "forgejo-delivery-001",
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
        }
        result = {
            "schema_version": 2,
            "verdict": "claimed",
            "admission_mode": "webhook_delivery",
            "profile_id": "mission-012-clamp",
            "webhook": {
                "delivery_id": "forgejo-delivery-001",
                "reconciled_state": "open",
                "requirement_sha256_matches_cli_argument": True,
            },
            "host_admission": {
                "scope": {
                    "origin": "http://127.0.0.1:3132",
                    "owner": "forge-admin",
                    "repository": "mission-012-clamp",
                    "branch": "main",
                },
                "issue_number": 1,
                "base_commit": base,
                "requirement_path": "requirements/MISSION-012-CLAMP-001.json",
                "requirement_sha256": digest,
                "allocation": {"allocation_id": "mission012-live-001", "execution_generation": 1},
            },
            "agent_runtime_invoked": False,
            "sandbox_only": True,
        }

        validate(
            result,
            event=event,
            requirement_sha256=digest,
            base_commit=base,
            allocation_id="mission012-live-001",
        )
        result["webhook"]["delivery_id"] = "other-delivery"
        with self.assertRaises(ValueError):
            validate(
                result,
                event=event,
                requirement_sha256=digest,
                base_commit=base,
                allocation_id="mission012-live-001",
            )

    def test_v3_admission_validator_binds_source_pr_and_authorized_reviewer(self) -> None:
        validate = self.driver.validate_admission_result
        digest = "ab" * 32
        base = "cd" * 20
        source_head = "ef" * 20
        source_merge = "12" * 20
        policy_digest = "34" * 32
        review_snapshot = "56" * 32
        event = {
            "sequence": 1,
            "delivery_id": "forgejo-v3-delivery-001",
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
        }
        result = {
            "schema_version": 2,
            "admission_schema_version": 3,
            "verdict": "claimed",
            "admission_mode": "webhook_delivery",
            "profile_id": "mission-012-clamp",
            "webhook": {
                "delivery_id": "forgejo-v3-delivery-001",
                "reconciled_state": "open",
                "requirement_sha256_matches_cli_argument": True,
            },
            "requirement_arbitration": {
                "schema_version": 1,
                "source_pull_request_number": 7,
                "source_head_sha": source_head,
                "source_merge_commit_sha": source_merge,
                "reviewer_ids": [9001],
                "requirements_sha256": digest,
                "policy_revision": 1,
                "policy_sha256": policy_digest,
                "review_snapshot_sha256": review_snapshot,
                "approved_review_ids": [808],
            },
            "host_admission": {
                "scope": {
                    "origin": "http://127.0.0.1:3132",
                    "owner": "forge-admin",
                    "repository": "mission-012-clamp",
                    "branch": "main",
                },
                "issue_number": 1,
                "base_commit": base,
                "requirement_path": "requirements/MISSION-012-CLAMP-001.json",
                "requirement_sha256": digest,
                "allocation": {"allocation_id": "mission012-live-v3-001", "execution_generation": 1},
            },
            "agent_runtime_invoked": False,
            "sandbox_only": True,
        }

        validate(
            result,
            event=event,
            requirement_sha256=digest,
            base_commit=base,
            allocation_id="mission012-live-v3-001",
            expected_admission_schema_version=3,
            expected_source_pull_request_number=7,
            expected_reviewer_ids=[9001],
            expected_source_head_sha=source_head,
            expected_source_merge_commit_sha=source_merge,
            expected_policy_revision=1,
            expected_policy_sha256=policy_digest,
            expected_approved_review_ids=[808],
        )

        result["requirement_arbitration"]["reviewer_ids"] = [9002]
        with self.assertRaises(ValueError):
            validate(
                result,
                event=event,
                requirement_sha256=digest,
                base_commit=base,
                allocation_id="mission012-live-v3-001",
                expected_admission_schema_version=3,
                expected_source_pull_request_number=7,
                expected_reviewer_ids=[9001],
                expected_source_head_sha=source_head,
                expected_source_merge_commit_sha=source_merge,
                expected_policy_revision=1,
                expected_policy_sha256=policy_digest,
                expected_approved_review_ids=[808],
            )

    def test_v3_requirement_document_preserves_issue_and_source_pr_decision(self) -> None:
        document = json.loads(self.driver.schema_v3_requirement_document(1, 7))

        self.assertEqual(
            document,
            {
                "schema_version": 3,
                "id": "MISSION-012-CLAMP-001",
                "revision": 1,
                "source_issue_number": 1,
                "source_pull_request_number": 7,
                "request": "Correct clamp_to_range so values above maximum return maximum.",
                "acceptance": [
                    "Values below minimum return minimum.",
                    "Values above maximum return maximum.",
                    "Values within the inclusive range are unchanged.",
                ],
                "ambiguities": [],
                "decisions": [
                    {
                        "ambiguity": "Which branch may the code change touch?",
                        "resolution": "Only src/lib.rs may change for the mission implementation.",
                        "rationale": "Keep the small behavioral correction scoped to the faulty function.",
                    }
                ],
            },
        )

        for issue_number, pull_request_number in ((0, 7), (1, 0), (True, 7)):
            with self.subTest(issue_number=issue_number, pull_request_number=pull_request_number):
                with self.assertRaisesRegex(ValueError, "positive Forgejo issue and pull request"):
                    self.driver.schema_v3_requirement_document(issue_number, pull_request_number)

    def test_schema_v3_source_pr_readback_requires_current_approved_merge(self) -> None:
        expected = b'{"schema_version":3,"source_pull_request_number":7}\n'
        head = "ab" * 20
        merge = "cd" * 20
        pull_request = {
            "number": 7,
            "state": "closed",
            "merged": True,
            "base": {"ref": "main"},
            "head": {"ref": "requirements/mission-012-v3", "sha": head},
            "merge_commit_sha": merge,
        }
        review = {
            "id": 81,
            "state": "APPROVED",
            "commit_id": head,
            "user": {"id": 9001},
        }
        main_branch = {"commit": {"id": merge}}

        receipt = self.driver.validate_schema_v3_source_pr_readback(
            pull_request,
            review,
            main_branch,
            expected,
            expected_requirement_bytes=expected,
            expected_pull_request_number=7,
            expected_branch="requirements/mission-012-v3",
            expected_reviewer_id=9001,
        )

        self.assertEqual(receipt["source_pull_request_number"], 7)
        self.assertEqual(receipt["source_head_sha"], head)
        self.assertEqual(receipt["source_merge_commit_sha"], merge)
        self.assertEqual(receipt["approved_review_id"], 81)
        self.assertEqual(receipt["reviewer_id"], 9001)
        self.assertEqual(receipt["requirements_sha256"], hashlib.sha256(expected).hexdigest())

        stale_review = {**review, "commit_id": "ef" * 20}
        with self.assertRaisesRegex(ValueError, "current approved source pull request"):
            self.driver.validate_schema_v3_source_pr_readback(
                pull_request,
                stale_review,
                main_branch,
                expected,
                expected_requirement_bytes=expected,
                expected_pull_request_number=7,
                expected_branch="requirements/mission-012-v3",
                expected_reviewer_id=9001,
            )
        with self.assertRaisesRegex(ValueError, "current approved source pull request"):
            self.driver.validate_schema_v3_source_pr_readback(
                pull_request,
                review,
                main_branch,
                expected + b"tampered",
                expected_requirement_bytes=expected,
                expected_pull_request_number=7,
                expected_branch="requirements/mission-012-v3",
                expected_reviewer_id=9001,
            )

    def test_binds_native_forgejo_replay_to_a_new_uuid_for_the_same_issue_payload(self) -> None:
        first_task = {
            "uuid": "00000000-0000-4000-8000-000000000001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
            "payload_sha256": "a" * 64,
            "payload_semantic_sha256": "b" * 64,
        }
        replay_task = {
            **first_task,
            "uuid": "00000000-0000-4000-8000-000000000002",
        }
        first_event = {
            "sequence": 1,
            "delivery_id": first_task["uuid"],
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
            "payload_semantic_sha256": "b" * 64,
        }
        replay_event = {
            **first_event,
            "sequence": 2,
            "delivery_id": replay_task["uuid"],
        }

        result = self.driver.validate_replayed_issue_delivery_binding(
            first_task, replay_task, first_event, replay_event
        )

        self.assertEqual(result["original_delivery_id"], first_task["uuid"])
        self.assertEqual(result["replay_delivery_id"], replay_task["uuid"])
        self.assertEqual(result["receiver_sequences"], [1, 2])
        self.assertIs(result["payload_unchanged"], True)

    def test_failed_503_then_native_replay_binds_one_admissible_event(self) -> None:
        failed_task = {
            "uuid": "00000000-0000-4000-8000-000000000001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": False,
            "response_status": 503,
            "payload_sha256": "a" * 64,
            "payload_semantic_sha256": "b" * 64,
        }
        replay_task = {
            **failed_task,
            "uuid": "00000000-0000-4000-8000-000000000002",
            "is_succeed": True,
            "response_status": 202,
        }
        failed_proxy = {
            "method": "POST",
            "path": "/forgejo/webhook",
            "event_type": "issues",
            "delivery_id": failed_task["uuid"],
            "forwarded_status": 503,
            "forwarded_retry_after": "5",
            "request_body_sha256": "a" * 64,
        }
        replay_proxy = {
            "method": "POST",
            "path": "/forgejo/webhook",
            "event_type": "issues",
            "delivery_id": replay_task["uuid"],
            "forwarded_status": 202,
            "request_body_sha256": "a" * 64,
        }
        queue_drain = {
            "partial_worker_requests": 4,
            "partial_worker_statuses": [408] * 4,
            "queued_probe_requests": 16,
            "queued_probe_statuses": [405] * 16,
            "drained": True,
        }
        replay_event = {
            "sequence": 1,
            "delivery_id": replay_task["uuid"],
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
            "payload_semantic_sha256": "b" * 64,
        }

        result = self.driver.validate_overload_replay_admission_binding(
            failed_task,
            replay_task,
            failed_proxy,
            replay_proxy,
            queue_drain,
            [],
            replay_event,
        )

        self.assertEqual(result["failed_delivery_id"], failed_task["uuid"])
        self.assertEqual(result["admitted_delivery_id"], replay_task["uuid"])
        self.assertEqual(result["receiver_sequence"], 1)
        self.assertIs(result["rejected_attempt_journaled"], False)
        self.assertIs(result["payload_unchanged"], True)

    def test_503_native_replay_binding_rejects_journaled_failure_and_bad_retry(self) -> None:
        failed_task = {
            "uuid": "00000000-0000-4000-8000-000000000001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": False,
            "response_status": 503,
            "payload_sha256": "a" * 64,
            "payload_semantic_sha256": "b" * 64,
        }
        replay_task = {
            **failed_task,
            "uuid": "00000000-0000-4000-8000-000000000002",
            "is_succeed": True,
            "response_status": 202,
        }
        failed_proxy = {
            "method": "POST",
            "path": "/forgejo/webhook",
            "event_type": "issues",
            "delivery_id": failed_task["uuid"],
            "forwarded_status": 503,
            "forwarded_retry_after": "5",
            "request_body_sha256": "a" * 64,
        }
        replay_proxy = {
            "method": "POST",
            "path": "/forgejo/webhook",
            "event_type": "issues",
            "delivery_id": replay_task["uuid"],
            "forwarded_status": 202,
            "request_body_sha256": "a" * 64,
        }
        queue_drain = {
            "partial_worker_requests": 4,
            "partial_worker_statuses": [408] * 4,
            "queued_probe_requests": 16,
            "queued_probe_statuses": [405] * 16,
            "drained": True,
        }
        replay_event = {
            "sequence": 1,
            "delivery_id": replay_task["uuid"],
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
            "payload_semantic_sha256": "b" * 64,
        }
        validate = self.driver.validate_overload_replay_admission_binding

        with self.assertRaisesRegex(ValueError, "503.*Retry-After"):
            validate(
                failed_task,
                replay_task,
                {**failed_proxy, "forwarded_retry_after": None},
                replay_proxy,
                queue_drain,
                [],
                replay_event,
            )
        with self.assertRaisesRegex(ValueError, "journaled"):
            validate(
                failed_task,
                replay_task,
                failed_proxy,
                replay_proxy,
                queue_drain,
                [{"delivery_id": failed_task["uuid"]}],
                replay_event,
            )
        with self.assertRaisesRegex(ValueError, "same original payload"):
            validate(
                failed_task,
                {**replay_task, "payload_sha256": "c" * 64},
                failed_proxy,
                replay_proxy,
                queue_drain,
                [],
                replay_event,
            )
        with self.assertRaisesRegex(ValueError, "503.*Retry-After"):
            validate(
                failed_task,
                replay_task,
                {**failed_proxy, "path": "/api/v1/forgejo/webhook"},
                replay_proxy,
                queue_drain,
                [],
                replay_event,
            )

    def test_rejects_native_replay_with_changed_issue_or_payload(self) -> None:
        task = {
            "uuid": "00000000-0000-4000-8000-000000000001",
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
            "payload_sha256": "a" * 64,
            "payload_semantic_sha256": "b" * 64,
        }
        replay_task = {**task, "uuid": "00000000-0000-4000-8000-000000000002"}
        first_event = {
            "sequence": 1,
            "delivery_id": task["uuid"],
            "event": "issues",
            "action": "opened",
            "issue_number": 1,
            "payload_state": "open",
            "payload_semantic_sha256": "b" * 64,
        }
        replay_event = {
            **first_event,
            "sequence": 2,
            "delivery_id": replay_task["uuid"],
        }

        replay_task["payload_sha256"] = "c" * 64
        with self.assertRaises(ValueError):
            self.driver.validate_replayed_issue_delivery_binding(
                task, replay_task, first_event, replay_event
            )

        replay_task["payload_sha256"] = task["payload_sha256"]
        replay_event["payload_semantic_sha256"] = "c" * 64
        with self.assertRaises(ValueError):
            self.driver.validate_replayed_issue_delivery_binding(
                task, replay_task, first_event, replay_event
            )

        replay_event["payload_semantic_sha256"] = "b" * 64
        replay_event["issue_number"] = 2
        with self.assertRaises(ValueError):
            self.driver.validate_replayed_issue_delivery_binding(
                task, replay_task, first_event, replay_event
            )

    def test_hashes_the_complete_received_issue_payload(self) -> None:
        payload = {
            "action": "opened",
            "repository": {"full_name": "forge-admin/mission-012-clamp"},
            "issue": {"number": 1, "state": "open", "body": "context"},
            "extra_field": {"labels": ["mission"]},
        }
        record = {
            "sequence": 1,
            "kind": "forgejo.webhook.v1.issues",
            "idempotency_key": "forgejo-delivery-v1:00000000-0000-4000-8000-000000000001",
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "delivery_id": "00000000-0000-4000-8000-000000000001",
                "event": "issues",
                "payload": payload,
            },
        }

        event = self.driver.parse_received_issue_event(
            record,
            owner="forge-admin",
            repository="mission-012-clamp",
            issue_number=1,
            expected_action="opened",
        )

        self.assertEqual(
            event["payload_semantic_sha256"],
            self.driver.native_replay.canonical_json_sha256(payload),
        )

    def test_rejects_received_replay_with_only_a_secondary_payload_field_changed(self) -> None:
        original_payload = {
            "action": "opened",
            "repository": {"full_name": "forge-admin/mission-012-clamp"},
            "issue": {"number": 1, "state": "open"},
            "extra_field": {"labels": ["mission"]},
        }
        replay_payload = {**original_payload, "extra_field": {"labels": ["different"]}}

        def received_record(delivery_id: str, payload: dict[str, object]) -> dict[str, object]:
            return {
                "sequence": 1 if delivery_id.endswith("1") else 2,
                "kind": "forgejo.webhook.v1.issues",
                "idempotency_key": f"forgejo-delivery-v1:{delivery_id}",
                "payload": {
                    "schema_version": 1,
                    "provider": "forgejo",
                    "delivery_id": delivery_id,
                    "event": "issues",
                    "payload": payload,
                },
            }

        original_id = "00000000-0000-4000-8000-000000000001"
        replay_id = "00000000-0000-4000-8000-000000000002"
        original_event = self.driver.parse_received_issue_event(
            received_record(original_id, original_payload),
            owner="forge-admin",
            repository="mission-012-clamp",
            issue_number=1,
            expected_action="opened",
        )
        replay_event = self.driver.parse_received_issue_event(
            received_record(replay_id, replay_payload),
            owner="forge-admin",
            repository="mission-012-clamp",
            issue_number=1,
            expected_action="opened",
        )
        first_task = {
            "uuid": original_id,
            "event_type": "issues",
            "is_delivered": True,
            "is_succeed": True,
            "response_status": 202,
            "payload_sha256": "a" * 64,
            "payload_semantic_sha256": self.driver.native_replay.canonical_json_sha256(original_payload),
        }
        replay_task = {**first_task, "uuid": replay_id}

        self.assertNotEqual(
            original_event["payload_semantic_sha256"],
            replay_event["payload_semantic_sha256"],
        )
        with self.assertRaises(ValueError):
            self.driver.validate_replayed_issue_delivery_binding(
                first_task, replay_task, original_event, replay_event
            )

    def test_native_replay_reuses_the_same_mission_claim_without_appending(self) -> None:
        host_admission = {
            "scope": {"origin": "http://127.0.0.1:3132", "owner": "forge-admin"},
            "issue_number": 1,
            "base_commit": "c" * 40,
            "requirement_sha256": "d" * 64,
            "allocation": {"allocation_id": "mission012-replay-001", "execution_generation": 1},
            "request": {
                "mission_id": "mission-v1-example",
                "idempotency_key": "execution-v1-example-1",
            },
        }
        first = {
            "verdict": "claimed",
            "allocation_replayed": False,
            "admission_mode": "webhook_delivery",
            "host_admission": host_admission,
            "webhook": {
                "delivery_id": "00000000-0000-4000-8000-000000000001",
                "reconciled_state": "open",
            },
        }
        replay = {
            "verdict": "already_claimed",
            "allocation_replayed": True,
            "admission_mode": "webhook_delivery",
            "host_admission": host_admission,
            "webhook": {
                "delivery_id": "00000000-0000-4000-8000-000000000002",
                "reconciled_state": "open",
            },
        }

        result = self.driver.validate_native_replay_admission_result(
            first,
            replay,
            expected_delivery_id="00000000-0000-4000-8000-000000000002",
            mission_journal_before=b"stable-journal",
            mission_journal_after=b"stable-journal",
            admission_file_before=b"stable-admission",
            admission_file_after=b"stable-admission",
        )

        self.assertEqual(result["mission_id"], "mission-v1-example")
        self.assertEqual(result["delivery_id"], replay["webhook"]["delivery_id"])
        self.assertIs(result["mission_journal_unchanged"], True)
        self.assertEqual(
            result["mission_journal_before_sha256"], result["mission_journal_after_sha256"]
        )
        self.assertIs(result["admission_file_unchanged"], True)
        self.assertEqual(
            result["admission_file_before_sha256"], result["admission_file_after_sha256"]
        )

    def test_refuses_native_replay_that_changes_claim_or_persistent_state(self) -> None:
        host_admission = {
            "issue_number": 1,
            "allocation": {"allocation_id": "mission012-replay-001", "execution_generation": 1},
            "request": {"mission_id": "mission-v1-example", "idempotency_key": "execution-v1-example-1"},
        }
        first = {
            "verdict": "claimed",
            "allocation_replayed": False,
            "admission_mode": "webhook_delivery",
            "host_admission": host_admission,
            "webhook": {"delivery_id": "delivery-one", "reconciled_state": "open"},
        }
        replay = {
            "verdict": "already_claimed",
            "allocation_replayed": True,
            "admission_mode": "webhook_delivery",
            "host_admission": host_admission,
            "webhook": {"delivery_id": "delivery-two", "reconciled_state": "open"},
        }

        for changes in (
            {"verdict": "claimed"},
            {"host_admission": {**host_admission, "allocation": {"allocation_id": "other"}}},
        ):
            candidate = {**replay, **changes}
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.driver.validate_native_replay_admission_result(
                    first,
                    candidate,
                    expected_delivery_id="delivery-two",
                    mission_journal_before=b"stable-journal",
                    mission_journal_after=b"changed-journal",
                    admission_file_before=b"stable-admission",
                    admission_file_after=b"stable-admission",
                )


if __name__ == "__main__":
    unittest.main()
