from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import uuid
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import run_mission_012_live_agent_pr_e2e as driver  # noqa: E402
import run_mission_012_live_composed_e2e as composed  # noqa: E402
import run_mission_012_supervised_private_netns_e2e as supervised  # noqa: E402
import run_forgejo_live_webhook_reconcile_e2e as webhook_helpers  # noqa: E402


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class Mission012LiveAgentDriverTests(unittest.TestCase):
    def test_supervisor_namespace_equality_requires_two_observed_inodes(self) -> None:
        self.assertFalse(supervised._same_namespace_inode(None, None))
        self.assertFalse(supervised._same_namespace_inode(0, 0))
        self.assertFalse(supervised._same_namespace_inode(17, 18))
        self.assertFalse(supervised._same_namespace_inode(True, True))
        self.assertTrue(supervised._same_namespace_inode(17, 17))

    def test_postmerge_acceptance_command_is_networkless_and_credential_free(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = root / "merged"
            toolchain = root / "toolchain"
            snapshot.mkdir()
            toolchain.mkdir()
            command = driver.postmerge_cargo_test_command(snapshot, toolchain)
        self.assertEqual(command[0], "/usr/bin/systemd-run")
        self.assertEqual(command[command.index("--") + 1], "/usr/bin/bwrap")
        self.assertIn("--unshare-all", command)
        self.assertIn("--unshare-pid", command)
        self.assertIn("--clearenv", command)
        self.assertIn("--ro-bind", command)
        self.assertIn("/candidate", command)
        self.assertIn("/toolchain", command)
        self.assertIn("/tmp", command)
        self.assertNotIn("--share-net", command)
        self.assertEqual(command[-5:], ["test", "--offline", "--locked", "--test", "acceptance"])

    def test_supervised_report_requires_postmerge_test_bound_to_exact_merge(self) -> None:
        merge = "a" * 40
        result = {
            "merged": True,
            "pull_request_state_after_merge": "closed",
            "base_commit": "b" * 40,
            "candidate_commit": "c" * 40,
            "merge_commit": merge,
            "main_head_after_merge": merge,
            "requirement_sha256": "d" * 64,
            "policy_sha256": "e" * 64,
            "candidate_source_sha256": "f" * 64,
            "acceptance_sha256": "1" * 64,
            "merge_tree_oid": "2" * 40,
            "postmerge_verification": {
                "verdict": "pass",
                "test_summary": "2 passed; 0 failed",
                "exit_code": 0,
                "timed_out": False,
                "output_bytes": 100,
                "output_sha256": "3" * 64,
                "commit": merge,
                "main_head_after_merge": merge,
                "base_commit": "b" * 40,
                "candidate_commit": "c" * 40,
                "requirement_sha256": "d" * 64,
                "policy_sha256": "e" * 64,
                "source_sha256": "f" * 64,
                "acceptance_sha256": "1" * 64,
                "merge_tree_oid": "2" * 40,
                "candidate_tree_oid": "2" * 40,
                "changed_paths_from_base": ["src/lib.rs"],
            },
        }
        test_output = (
            b"running 2 tests\n"
            b"test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n"
        )
        result["postmerge_verification"]["output_bytes"] = len(test_output)
        result["postmerge_verification"]["output_sha256"] = sha256(test_output)
        self.assertTrue(supervised._postmerge_behavior_is_verified(
            result, result["postmerge_verification"], test_output,
        ))
        result["postmerge_verification"]["commit"] = "9" * 40
        self.assertFalse(supervised._postmerge_behavior_is_verified(
            result, result["postmerge_verification"], test_output,
        ))

    def test_supervised_runner_mounts_the_workspace_read_only_and_masks_claude_home(self) -> None:
        command = supervised._bwrap_command(
            SimpleNamespace(
                forgejo_binary="/tmp/forgejo-17.0.0",
                state_dir="/tmp/mission-012-supervised-unit",
                evidence_dir=str(ROOT / "evidence/mission-012-live-agent-webhook-unit"),
                execution_id="live-mcp-099",
                allocation_id="mission012-supervised-unit",
            ),
            "net:[999999999]",
            os.geteuid(),
        )
        self.assertIn("--unshare-net", command)
        self.assertIn("--ro-bind", command)
        self.assertIn("--tmpfs", command)
        self.assertIn("/run/worktree/.claude", command)
        self.assertIn("/run/worktree/evidence", command)
        git_dir_index = command.index("--source-git-dir")
        self.assertEqual(
            command[git_dir_index + 1],
            str(Path("/run/source-git") / supervised.netns._git_metadata_paths(ROOT)[1]),
        )
        self.assertEqual(command[command.index("--source-git-work-tree") + 1], "/run/worktree")
        setenv_keys = [command[index + 1] for index, item in enumerate(command[:-1]) if item == "--setenv"]
        self.assertNotIn("GIT_DIR", setenv_keys)
        self.assertNotIn("GIT_WORK_TREE", setenv_keys)
        self.assertNotIn("GIT_OPTIONAL_LOCKS", setenv_keys)
        workspace_source = command.index(str(ROOT))
        self.assertEqual(command[workspace_source - 1], "--ro-bind")
        self.assertEqual(command[workspace_source + 1], "/run/worktree")

    def test_supervisor_capability_packet_binds_target_and_live_linux_processes(self) -> None:
        pid = os.getpid()
        packet = driver.supervisor_capability_packet(
            pull_request=23,
            forgejo_pid=pid,
            gate_pid=pid,
        )
        self.assertLessEqual(len(packet), 8192)
        envelope = json.loads(packet)
        self.assertEqual(envelope["target"], {
            "profile_selector": "mission-012-clamp",
            "origin": "http://127.0.0.1:3132",
            "owner": "forge-admin",
            "repository": "mission-012-clamp",
            "pull_request_number": 23,
            "base_branch": "main",
        })
        self.assertEqual(envelope["supervisor"]["pid"], pid)
        self.assertEqual(envelope["gate"], envelope["forgejo"])
        self.assertEqual(len(envelope["nonce"]), 64)

    def test_composed_sandbox_adapter_is_explicit_and_local_only(self) -> None:
        state = Path("/tmp/mission-012-sandbox-adapter-001")
        binary = Path("/tmp/forgejo-17.0.0")
        evidence = ROOT / "evidence" / f"mission-012-live-agent-webhook-sandbox-{uuid.uuid4().hex[:8]}"
        stages = composed.build_stage_commands(
            ROOT,
            state,
            binary,
            evidence,
            "live-mcp-101",
            "mission012-sandbox-101",
            "0.20",
            agent_adapter="local-test-adapter",
            sandbox_test_adapter=True,
            expected_host_network_namespace="net:[999999999]",
            source_git_dir="/run/source-git/worktrees/webhook-005-workspace",
            source_git_work_tree="/run/worktree",
        )
        stages_by_name = {name: argv for name, argv, _ in stages}
        self.assertIn("--sandbox-test-adapter", stages_by_name["agent"])
        self.assertNotIn("--sandbox-test-adapter", stages_by_name["pull-request"])
        self.assertIn("--source-git-dir", stages_by_name["admission"])
        self.assertNotIn("--source-git-dir", stages_by_name["agent"])
        self.assertNotIn("--source-git-dir", stages_by_name["pull-request"])
        self.assertIn("--source-git-work-tree", stages_by_name["admission"])
        self.assertEqual(
            stages_by_name["pull-request"][
                stages_by_name["pull-request"].index("--expected-host-network-namespace") + 1
            ],
            "net:[999999999]",
        )

        with self.assertRaisesRegex(RuntimeError, "only available for local-test-adapter"):
            composed.build_stage_commands(
                ROOT,
                state,
                binary,
                evidence,
                "live-mcp-101",
                "mission012-sandbox-101",
                "0.20",
                agent_adapter="claude",
                sandbox_test_adapter=True,
            )

    def test_restart_recovery_composition_is_fixture_only_and_builds_crash_host(self) -> None:
        stages = composed.build_stage_commands(
            ROOT,
            Path("/tmp/mission-012-forgejo-restart-051"),
            Path("/tmp/forgejo-17.0.0"),
            ROOT / "evidence/mission-012-live-agent-webhook-local-adapter-051",
            "live-mcp-051",
            "mission012-restart-051",
            "0.20",
            agent_adapter="local-test-adapter",
            test_host_restart=True,
        )
        stages_by_name = {name: argv for name, argv, _ in stages}
        self.assertIn("--build-test-crash-injection-host", stages_by_name["admission"])
        self.assertIn("--test-host-restart", stages_by_name["agent"])
        self.assertIn("--admission-evidence-dir", stages_by_name["agent"])

        with self.assertRaisesRegex(RuntimeError, "requires local-test-adapter"):
            composed.build_stage_commands(
                ROOT,
                Path("/tmp/mission-012-forgejo-restart-051"),
                Path("/tmp/forgejo-17.0.0"),
                ROOT / "evidence/mission-012-live-agent-webhook-local-adapter-051",
                "live-mcp-051",
                "mission012-restart-051",
                "0.20",
                agent_adapter="claude",
                test_host_restart=True,
            )

    def test_restart_evidence_requires_a_new_fence_and_bounded_remaining_lease(self) -> None:
        host_hash = "a" * 64
        proof = {
            "crash_point": "after_effect",
            "first_attempt_exit_code": -9,
            "first_attempt_timed_out": False,
            "first_attempt_output_exceeded": False,
            "unresolved_intents_before_restart": 1,
            "results_before_restart": 0,
            "candidate_source_sha256_after_effect": "b" * 64,
            "operation_id": "workspace-write-operation-1",
            "old_fencing_token": 1,
            "new_fencing_token": 2,
            "initial_lease_id": "mission012-live-mcp-051",
            "recovery_lease_id": "mission012-live-mcp-051-recovery",
            "initial_runtime_timeout_ms": 300_000,
            "remaining_runtime_ms_at_recovery": 250_000,
            "recovery_lease_ttl_ms": 120_000,
            "lease_expiry_wait_ms": 10_250,
            "results_after_restart": 1,
            "resume_checkpoint_status": "unverified",
            "initial_receipt_sha256": "c" * 64,
            "recovery_receipt_sha256": "d" * 64,
            "combined_receipt_sha256": "e" * 64,
            "initial_receipt_count": 5,
            "recovery_receipt_count": 8,
        }
        agent = {
            "host_binary_sha256": host_hash,
            "restart_recovery": proof,
        }
        self.assertEqual(
            composed.validate_host_restart_evidence(agent, host_hash), proof
        )

        unbounded = dict(agent)
        unbounded["restart_recovery"] = {
            **proof,
            "recovery_lease_ttl_ms": 250_000,
        }
        with self.assertRaisesRegex(RuntimeError, "incomplete"):
            composed.validate_host_restart_evidence(unbounded, host_hash)

    def test_reconcile_result_must_echo_the_requested_delivery_id(self) -> None:
        delivery = {"delivery_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}
        driver.validate_pr_reconciliation_delivery_binding(
            delivery, {"delivery_id": delivery["delivery_id"]}
        )

        with self.assertRaisesRegex(RuntimeError, "delivery"):
            driver.validate_pr_reconciliation_delivery_binding(
                delivery, {"delivery_id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"}
            )
        with self.assertRaisesRegex(RuntimeError, "delivery"):
            driver.validate_pr_reconciliation_delivery_binding(delivery, {})

    def test_verifier_uses_the_current_recovery_lease_after_host_restart(self) -> None:
        self.assertEqual(
            driver.verifier_lease_id_for_summary({}, "live-mcp-059"),
            "mission012-live-mcp-059",
        )
        self.assertEqual(
            driver.verifier_lease_id_for_summary(
                {
                    "restart_recovery": {
                        "initial_lease_id": "mission012-live-mcp-059",
                        "recovery_lease_id": "mission012-live-mcp-059-recovery",
                    }
                },
                "live-mcp-059",
            ),
            "mission012-live-mcp-059-recovery",
        )
        with self.assertRaisesRegex(RuntimeError, "lease IDs"):
            driver.verifier_lease_id_for_summary(
                {
                    "restart_recovery": {
                        "initial_lease_id": "mission012-live-mcp-059",
                        "recovery_lease_id": "mission012-other-recovery",
                    }
                },
                "live-mcp-059",
            )

    def test_webhook_cleanup_deactivation_preserves_event_selection_and_reads_back_state(self) -> None:
        class Owner:
            def __init__(self) -> None:
                self.active = True
                self.calls: list[tuple[str, str, object]] = []

            def expect(self, method: str, path: str, value: object | None = None):
                self.calls.append((method, path, value))
                if method == "PATCH":
                    self.active = value["active"]
                    return 200, {}, {}
                return 200, {
                    "id": 7,
                    "type": "gitea",
                    "config": {
                        "url": "http://127.0.0.1:45678/forgejo/webhook",
                        "content_type": "json",
                    },
                    "events": sorted(driver.webhook_events.effective_hook_events(("pull_request",))),
                    "active": self.active,
                }, {}

        owner = Owner()
        observer = {
            "hook_id": 7,
            "expected_target": "http://127.0.0.1:45678/forgejo/webhook",
            "hook_deactivated": False,
        }
        hook_path = "/repos/forge-admin/mission-012-clamp/hooks/7"

        readback = driver.deactivate_pr_webhook_observer(owner, observer)

        self.assertEqual(owner.calls, [
            ("PATCH", hook_path, {"active": False, "events": ["pull_request"]}),
            ("GET", hook_path, None),
        ])
        self.assertIs(readback["active_after_run"], False)
        self.assertIs(readback["deactivation_readback_confirmed"], True)
        self.assertIs(observer["hook_deactivated"], True)

    def test_composed_merged_webhook_requires_merge_sha_in_event_and_current_state(self) -> None:
        base = "1" * 40
        candidate = "2" * 40
        merge = "3" * 40
        delivery_ids = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            "33333333-3333-4333-8333-333333333333",
        ]

        def receipt(delivery_id: str) -> dict[str, object]:
            return {
                "delivery_id": delivery_id,
                "exit_code": 0,
                "verdict": "current_state_read",
                "upstream_read_attempts": 1,
                "resource": {
                    "kind": "pull_request",
                    "owner": "forge-admin",
                    "repository": "mission-012-clamp",
                    "number": 12,
                },
                "canonical_state": {
                    "state": "closed",
                    "merged": True,
                    "head_sha": candidate,
                    "base_sha": merge,
                    "merge_commit_sha": merge,
                },
            }

        value: dict[str, object] = {
            "schema_version": 5,
            "status": "verified_current_merged_pr",
            "target_profile": "mission-012-clamp",
            "pull_request_number": 12,
            "candidate_commit": candidate,
            "merge_commit": merge,
            "hook_id": 7,
            "receiver_binary_sha256": "a" * 64,
            "event_count": 3,
            "forgejo_task_count": 3,
            "successful_http_202_count": 3,
            "opened_delivery_id": delivery_ids[0],
            "approved_review_delivery_id": delivery_ids[1],
            "closed_delivery_id": delivery_ids[2],
            "opened_sequence": 1,
            "approved_review_sequence": 2,
            "closed_sequence": 3,
            "opened_payload": {
                "state": "open", "merged": False,
                "head_sha": candidate, "base_sha": base,
            },
            "closed_payload": {
                "state": "closed", "merged": True,
                "head_sha": candidate, "base_sha": merge,
                "merge_commit_sha": merge,
            },
            "approved_review_notification": {
                "event": "pull_request_approved",
                "action": "reviewed",
                "review_type": "pull_request_review_approved",
                "pull_request_number": 12,
                "head_sha": candidate,
                "base_sha": base,
                "payload_state": "open",
                "payload_merged": False,
            },
            "reconciled_delivery_ids": delivery_ids,
            "reconciliation_receipts": [receipt(delivery_id) for delivery_id in delivery_ids],
            "all_notifications_reconciled_to_current_state": True,
            "review_notification_is_approval_proof": False,
            "canonical_state": {
                "state": "closed", "merged": True,
                "head_sha": candidate, "base_sha": merge,
                "merge_commit_sha": merge,
            },
            "hook_readback": {
                "type": "gitea",
                "content_type": "json",
                "events": ["pull_request"],
                "configuration_matches_expected": True,
                "active_after_run": False,
                "deactivation_readback_confirmed": True,
                "secret_value_recorded": False,
                "target": "http://127.0.0.1:45678/forgejo/webhook",
            },
            "cleanup": {
                "hook_deactivated": True,
                "receiver_stopped": True,
                "receiver_listener_absent": True,
            },
        }

        def validate(candidate_value: object) -> object:
            return composed.validate_pr_webhook_reconciliation(
                candidate_value,
                pull_request=12,
                base_commit=base,
                candidate_commit=candidate,
                merge_commit=merge,
            )

        self.assertIs(validate(value), value)
        for path, replacement in (
            (("closed_payload", "merge_commit_sha"), None),
            (("closed_payload", "merge_commit_sha"), "4" * 40),
            (("canonical_state", "merge_commit_sha"), None),
            (("canonical_state", "merge_commit_sha"), "4" * 40),
            (("reconciliation_receipts", 0, "canonical_state", "merge_commit_sha"), None),
            (("reconciliation_receipts", 0, "canonical_state", "merge_commit_sha"), "4" * 40),
        ):
            rejected = json.loads(json.dumps(value))
            target: object = rejected
            for key in path[:-1]:
                target = target[key]  # type: ignore[index]
            target[path[-1]] = replacement  # type: ignore[index]
            with self.subTest(path=path, replacement=replacement):
                with self.assertRaisesRegex(RuntimeError, "not bound to the merged candidate"):
                    validate(rejected)

    def test_reconcile_profile_is_allowlisted_and_credential_stays_in_private_fd(self) -> None:
        captured: dict[str, object] = {}

        def fake_run(command: list[str], **kwargs: object) -> SimpleNamespace:
            captured["command"] = command
            captured["kwargs"] = kwargs
            return SimpleNamespace(returncode=0, stdout=b'{"verdict":"current_state_read"}', stderr=b"")

        with (
            mock.patch.object(webhook_helpers.os, "pipe", return_value=(10, 11)),
            mock.patch.object(webhook_helpers.os, "write", return_value=5),
            mock.patch.object(webhook_helpers.os, "close"),
            mock.patch.object(webhook_helpers.subprocess, "run", side_effect=fake_run),
        ):
            status, result = webhook_helpers.run_reconcile(
                Path("/tmp/reconcile"), Path("/tmp/state"), Path("/tmp/inbox.jsonl"),
                "delivery-1", "private-reader-token", target_profile="mission-012-clamp",
            )

        command = captured["command"]
        kwargs = captured["kwargs"]
        self.assertEqual(status, 0)
        self.assertEqual(result["verdict"], "current_state_read")
        self.assertEqual(command[:3], ["/tmp/reconcile", "--target-profile", "mission-012-clamp"])
        self.assertNotIn("private-reader-token", command)
        self.assertEqual(kwargs["pass_fds"], (10,))
        self.assertNotIn("private-reader-token", kwargs["env"].values())

    def test_reconcile_refuses_a_non_allowlisted_target_profile_before_spawning(self) -> None:
        spawn = mock.patch.object(webhook_helpers.subprocess, "run")
        mocked_spawn = spawn.start()
        self.addCleanup(spawn.stop)

        with self.assertRaisesRegex(RuntimeError, "fixed local allowlist"):
            webhook_helpers.run_reconcile(
                Path("/tmp/reconcile"), Path("/tmp/state"), Path("/tmp/inbox.jsonl"),
                "delivery-1", "private-reader-token", target_profile="http://127.0.0.1:9999",
            )

        mocked_spawn.assert_not_called()

    def test_rust_merge_owner_credential_has_read_only_identity_scope_without_admin(self) -> None:
        scopes = set(driver.MERGE_GATE_OWNER_SCOPES)

        self.assertEqual(scopes, {"read:user", "write:repository", "write:issue"})
        self.assertNotIn("admin:org", scopes)
        self.assertNotIn("admin:repo", scopes)

    def test_rust_merge_gate_build_uses_locked_offline_inputs_and_hashes_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            state = root / "state"
            state.mkdir(mode=0o700)
            toolchain = root / "toolchain"
            (toolchain / "bin").mkdir(parents=True)
            for name in ("cargo", "rustc"):
                tool = toolchain / "bin" / name
                tool.write_bytes(b"tool")
                tool.chmod(0o700)
            cargo_home = root / "cargo-home"
            cargo_home.mkdir(mode=0o700)
            captured: dict[str, object] = {}

            def build(command: list[str], **kwargs: object) -> SimpleNamespace:
                captured["command"] = command
                captured["env"] = kwargs["env"]
                target_dir = Path(command[command.index("--target-dir") + 1])
                captured["target_dir"] = target_dir
                debug = target_dir / "debug"
                debug.mkdir(parents=True, mode=0o700)
                for name in ("gate", "forgejo-reconcile-local"):
                    source = debug / f"{name}.cargo-artifact"
                    source.write_bytes(name.encode())
                    source.chmod(0o700)
                    os.link(source, debug / name)
                return SimpleNamespace(returncode=0, stdout=b"offline build")

            with (
                mock.patch.object(driver, "RUST_TOOLCHAIN", toolchain),
                mock.patch.object(driver, "CARGO_HOME", cargo_home),
                mock.patch.object(driver.subprocess, "run", side_effect=build),
            ):
                result = driver.build_rust_merge_gate_tools(state)

            command = captured["command"]
            env = captured["env"]
            target_dir = Path(captured["target_dir"])
            self.assertIn("--offline", command)
            self.assertIn("--locked", command)
            self.assertEqual(env["CARGO_NET_OFFLINE"], "true")
            self.assertEqual(env["HOME"], "/nonexistent")
            self.assertEqual(result["record"]["binary_sha256"]["gate"], sha256(b"gate"))
            self.assertEqual(result["record"]["binary_sha256"]["forgejo-reconcile-local"], sha256(b"forgejo-reconcile-local"))
            for binary in (result["gate_binary"], result["authority_binary"]):
                metadata = Path(binary).lstat()
                self.assertNotEqual(Path(binary).parent, target_dir / "debug")
                self.assertEqual(metadata.st_nlink, 1)
                self.assertEqual(metadata.st_uid, os.geteuid())
                self.assertEqual(metadata.st_mode & 0o777, 0o700)

    def _run_with_cost_summary(self, summary: dict[str, object], expected_error: str) -> None:
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        state = root / "state"
        candidate_root = state / "project/.runtime/live-mcp-001"
        candidate_root.mkdir(parents=True)
        summary_path = candidate_root / "agent-run-summary.json"
        summary_path.write_text(json.dumps({
            "agent_adapter": "claude",
            "model": "claude-opus-5-5",
            "effort": "high",
            "model_invoked_during_run": True,
            **summary,
        }))
        summary_path.chmod(0o600)
        forgejo = root / "forgejo"
        forgejo.write_bytes(b"not started because cost preflight must fail")
        evidence = root / "evidence"
        args = SimpleNamespace(
            state_dir=str(state), evidence_dir=str(evidence), forgejo_binary=str(forgejo),
            execution_id="live-mcp-001", admission_file=None, budget_cap_usd="0.20",
        )
        process = mock.patch.object(driver.subprocess, "Popen")
        popen = process.start()
        self.addCleanup(process.stop)
        with self.assertRaisesRegex(RuntimeError, expected_error):
            driver.run(args)
        self.assertFalse(evidence.exists())
        popen.assert_not_called()

    def test_local_adapter_requires_explicit_no_model_result_and_zero_cost(self) -> None:
        driver.validate_model_cost({
            "agent_adapter": "local-test-adapter",
            "model": "local-test-adapter",
            "effort": "deterministic",
            "model_invoked_during_run": False,
            "budget_cap_usd": 0.20,
            "reported_cost_usd": 0.0,
        }, "0.20")

        for field, value in (
            ("model_invoked_during_run", True),
            ("reported_cost_usd", 0.01),
            ("model", "claude-opus-5-5"),
        ):
            bad = {
                "agent_adapter": "local-test-adapter",
                "model": "local-test-adapter",
                "effort": "deterministic",
                "model_invoked_during_run": False,
                "budget_cap_usd": 0.20,
                "reported_cost_usd": 0.0,
            }
            bad[field] = value
            with self.subTest(field=field):
                with self.assertRaisesRegex(RuntimeError, "adapter|model invocation|zero cost"):
                    driver.validate_model_cost(bad, "0.20")

    def test_missing_reported_cost_is_rejected_before_forgejo_starts(self) -> None:
        self._run_with_cost_summary(
            {
                "status": "candidate_ready_for_independent_verification",
                "budget_cap_usd": 0.20,
            },
            "omitted its reported cost",
        )

    def test_reported_cost_above_cap_is_rejected_before_forgejo_starts(self) -> None:
        self._run_with_cost_summary({
                "status": "candidate_ready_for_independent_verification",
                "budget_cap_usd": 0.20,
                "reported_cost_usd": 0.21,
            },
            "model cost exceeded",
        )

    def test_invalid_and_negative_reported_costs_are_rejected_before_forgejo_starts(self) -> None:
        cases = (
            (True, "invalid cost value"),
            ("not-a-number", "invalid cost value"),
            (-0.01, "model cost exceeded"),
            (float("inf"), "model cost exceeded"),
        )
        for reported_cost, expected_error in cases:
            with self.subTest(reported_cost=reported_cost):
                self._run_with_cost_summary({
                    "status": "candidate_ready_for_independent_verification",
                    "budget_cap_usd": 0.20,
                    "reported_cost_usd": reported_cost,
                }, expected_error)

    def test_summary_budget_above_configured_cap_is_rejected_before_forgejo_starts(self) -> None:
        self._run_with_cost_summary({
            "status": "candidate_ready_for_independent_verification",
            "budget_cap_usd": 0.21,
            "reported_cost_usd": 0.05,
        }, "budget above the configured model cap")

    def test_latest_merge_gate_api_reads_discard_only_headers(self) -> None:
        class FakeApi:
            def expect(self, method: str, path: str, value: object | None = None):
                self.call = (method, path, value)
                return 200, {"state": "open"}, {"X-Total-Count": "1"}

        api = FakeApi()
        status, body = driver.expect_body(api, "GET", "/repos/owner/repo/pulls/1")

        self.assertEqual(status, 200)
        self.assertEqual(body, {"state": "open"})
        self.assertEqual(api.call, ("GET", "/repos/owner/repo/pulls/1", None))

    def test_forged_test_output_candidate_is_rejected_before_cargo(self) -> None:
        malicious = (
            b'pub fn clamp_to_range(_: i32, _: i32, _: i32) -> i32 { '
            b'println!("2 passed; 0 failed"); std::process::exit(0) }\n'
        )
        with tempfile.TemporaryDirectory() as temp:
            snapshot = Path(temp)
            (snapshot / "src").mkdir()
            (snapshot / "tests").mkdir()
            (snapshot / "src/lib.rs").write_bytes(malicious)
            (snapshot / "tests/acceptance.rs").write_bytes(b"[]\n")
            with mock.patch.object(driver, "run_journaled_verifier_test") as cargo_test:
                with self.assertRaisesRegex(RuntimeError, "pinned MISSION-012 sources"):
                    driver.verify_and_run_candidate(snapshot, Path("unused"))
            cargo_test.assert_not_called()

    def test_only_exact_expected_source_and_acceptance_suite_are_accepted(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "pinned MISSION-012 sources"):
            driver.verify_pinned_sources(b"changed implementation", b"changed acceptance suite")

    def test_competing_code_edit_conflicts_with_the_agent_candidate_on_the_same_line(self) -> None:
        base = (
            b"pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n"
            b"    if value > maximum {\n"
            b"        minimum // BUG: values above the range should clamp to maximum.\n"
            b"    } else {\n"
            b"        value\n"
            b"    }\n"
            b"}\n"
        )
        bug_line = b"minimum // BUG: values above the range should clamp to maximum."
        candidate = base.replace(bug_line, b"maximum")

        competitor = driver.build_conflicting_competing_source(base)

        base_lines = base.splitlines()
        candidate_lines = candidate.splitlines()
        competitor_lines = competitor.splitlines()
        candidate_changes = [i for i, pair in enumerate(zip(base_lines, candidate_lines)) if pair[0] != pair[1]]
        competitor_changes = [i for i, pair in enumerate(zip(base_lines, competitor_lines)) if pair[0] != pair[1]]
        self.assertEqual(candidate_changes, [2])
        self.assertEqual(competitor_changes, candidate_changes)
        self.assertEqual(candidate_lines[2], b"        maximum")
        self.assertIn(b"maximum - 1", competitor_lines[2])
        self.assertNotEqual(candidate_lines[2], competitor_lines[2])
        with self.assertRaisesRegex(RuntimeError, "exactly one pinned bug line"):
            driver.build_conflicting_competing_source(b"no pinned source")
        with self.assertRaisesRegex(RuntimeError, "exactly one pinned bug line"):
            driver.build_conflicting_competing_source(base + base)

    def test_postverification_candidate_mutation_breaks_the_verified_upper_boundary(self) -> None:
        fixed = (
            b"pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n"
            b"    if value < minimum {\n"
            b"        minimum\n"
            b"    } else if value > maximum {\n"
            b"        maximum\n"
            b"    } else {\n"
            b"        value\n"
            b"    }\n"
            b"}\n"
        )
        expected = fixed.replace(b"        maximum\n", b"        maximum - 1\n", 1)
        self.assertEqual(driver.build_postverification_candidate_mutation(fixed), expected)
        with self.assertRaisesRegex(RuntimeError, "unique fixed upper-bound return"):
            driver.build_postverification_candidate_mutation(b"no fixed candidate")
        with self.assertRaisesRegex(RuntimeError, "unique fixed upper-bound return"):
            driver.build_postverification_candidate_mutation(fixed + fixed)

    def test_candidate_mutation_refusal_requires_stale_head_and_no_merge(self) -> None:
        result = {
            "schema_version": 1,
            "status": "refused_after_candidate_mutation_before_merge",
            "pull_request": 2,
            "base_commit": "a" * 40,
            "requirement_sha256": driver.REQUIREMENT_SHA,
            "policy_sha256": driver.POLICY_SHA,
            "candidate_commit": "b" * 40,
            "candidate_sha256": "c" * 64,
            "candidate_mutation": {
                "head_before": "b" * 40,
                "head_after": "d" * 40,
                "source_sha256_before": "c" * 64,
                "source_sha256_after": "e" * 64,
            },
            "changed_paths": ["src/lib.rs"],
            "verifier": "pass",
            "reviewer": "verifier-bot",
            "pre_review_merge_refusal_http": 405,
            "reviewer_merge_refusal_http": 405,
            "issue_state": "open",
            "target_pull_request_state": "open",
            "target_pull_request_merged": False,
            "target_pull_request_base_sha_after_mutation": "a" * 40,
            "target_pull_request_head_sha_after_mutation": "d" * 40,
            "target_candidate_source_sha256_after_mutation": "e" * 64,
            "target_merge_commit": None,
            "main_head_after_refusal": "a" * 40,
            "main_source_sha256_after_refusal": driver.BROKEN_SOURCE_SHA,
            "merge_authority": "forgejo-reconcile-local-rust",
            "merge_gate_policy_sha256": "f" * 64,
            "merge_gate_binary_sha256": "1" * 64,
            "merge_gate_verdict": "rejected",
            "merge_authorized": False,
            "merge_attempted": False,
            "merge_gate_error_code": "invalid_review_base_binding",
        }
        driver.validate_candidate_mutation_refusal_result(
            result,
            expected_base="a" * 40,
            expected_candidate_commit="b" * 40,
            expected_candidate_sha256="c" * 64,
        )
        for field, value in (
            ("merge_attempted", True),
            ("target_pull_request_merged", True),
            ("target_pull_request_head_sha_after_mutation", "b" * 40),
            ("merge_gate_error_code", ""),
        ):
            with self.subTest(field=field):
                with self.assertRaisesRegex(RuntimeError, "candidate-mutation refusal"):
                    driver.validate_candidate_mutation_refusal_result(
                        {**result, field: value},
                        expected_base="a" * 40,
                        expected_candidate_commit="b" * 40,
                        expected_candidate_sha256="c" * 64,
                    )

    def test_composed_candidate_mutation_mode_is_local_and_exclusive(self) -> None:
        state = Path("/tmp/mission-012-candidate-mutation-067")
        binary = Path("/tmp/forgejo-17.0.0")
        evidence = ROOT / "evidence" / f"mission-012-live-agent-webhook-unit-{uuid.uuid4().hex}"
        stages = composed.build_stage_commands(
            ROOT,
            state,
            binary,
            evidence,
            "live-mcp-067",
            "mission012-candidate-mutation-067",
            "0.20",
            agent_adapter="local-test-adapter",
            inject_candidate_mutation_after_verification=True,
        )
        stages_by_name = {name: argv for name, argv, _ in stages}
        self.assertIn(
            "--inject-candidate-mutation-after-verification",
            stages_by_name["pull-request"],
        )
        with self.assertRaisesRegex(RuntimeError, "cannot be combined"):
            composed.build_stage_commands(
                ROOT,
                state,
                binary,
                evidence,
                "live-mcp-067",
                "mission012-candidate-mutation-067",
                "0.20",
                agent_adapter="local-test-adapter",
                test_host_restart=True,
                inject_candidate_mutation_after_verification=True,
            )

    def test_composed_conflict_readback_requires_the_git_conflict_and_exact_path(self) -> None:
        evidence = {
            "schema_version": 1,
            "base_before": "a" * 40,
            "main_after": "b" * 40,
            "merged": True,
            "merge_http_status": 200,
            "same_file_conflict": True,
            "changed_path": "src/lib.rs",
            "main_source_sha256": "c" * 64,
            "git_merge_tree_exit_code": 1,
            "git_merge_conflict_paths": ["src/lib.rs"],
            "git_merge_tree_stdout_sha256": "d" * 64,
            "readme_marker_sha256": None,
        }
        self.assertTrue(composed.concurrent_change_evidence_is_valid(
            evidence, same_file_conflict=True,
            expected_base="a" * 40, expected_main="b" * 40,
        ))
        for field, value in (
            ("changed_path", "README.md"),
            ("git_merge_tree_exit_code", 0),
            ("git_merge_conflict_paths", ["src/other.rs"]),
        ):
            tampered = {**evidence, field: value}
            self.assertFalse(composed.concurrent_change_evidence_is_valid(
                tampered, same_file_conflict=True,
                expected_base="a" * 40, expected_main="b" * 40,
            ))

    def test_journaled_verifier_requires_a_resolved_receipt_from_the_pinned_host(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            host = root / "mission-012-mcp-host"
            host.write_bytes(b"trusted test host")
            host.chmod(0o700)
            journal = root / "mission-execution.jsonl"
            journal.write_text("journal")
            journal.chmod(0o600)
            snapshot = root / "snapshot"
            snapshot.mkdir(mode=0o700)
            target = root / "target"
            sysroot = root / "sysroot"
            sysroot.mkdir()
            reservation_id = "pr-base-red-0123456789abcdef0123456789abcdef"
            ledger_entry = {
                "reservation_id": reservation_id,
                "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
                "charged_cpu_ms": 5000,
                "resolved": True,
            }
            measurement = {
                "schema_version": 1,
                "verdict": "measurement_complete",
                "mission_id": "mission-v1-" + "a" * 64,
                "lease_id": "mission012-live-mcp-001",
                "reservation_id": reservation_id,
                "policy_sha256": driver.VERIFIER_CPU_POLICY_SHA256,
                "mission_cpu_budget_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS,
                "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
                "charged_cpu_ms": 5000,
                "elapsed_ms": 1500,
                "accounting_elapsed_ms": 1500,
                "exit_code": 101,
                "timed_out": False,
                "output_truncated": False,
                "stdout": "1 passed; 1 failed",
                "stderr": "",
                "ledger": {
                    "mission_id": "mission-v1-" + "a" * 64,
                    "policy_sha256": driver.VERIFIER_CPU_POLICY_SHA256,
                    "mission_cpu_budget_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS,
                    "charged_cpu_ms": 5000,
                    "remaining_cpu_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS - 5000,
                    "reservations": [ledger_entry],
                },
            }
            with mock.patch.object(
                driver.subprocess,
                "run",
                return_value=SimpleNamespace(
                    returncode=0,
                    stdout=json.dumps(measurement).encode(),
                    stderr=b"",
                ),
            ) as run_process:
                result = driver.run_journaled_verifier_test(
                    host, journal, "mission012-live-mcp-001", reservation_id,
                    snapshot, target, sysroot, sha256(b"trusted test host"),
                )
            self.assertEqual(result["exit_code"], 101)
            self.assertEqual(result["reservation_id"], reservation_id)
            command = run_process.call_args.args[0]
            self.assertEqual(command[0:5], [
                str(host), "--run-journaled-verifier-test", str(journal),
                "mission012-live-mcp-001", reservation_id,
            ])
            self.assertEqual(command[-3:], [str(snapshot), str(target), str(sysroot)])
            self.assertEqual(run_process.call_args.kwargs["timeout"], 160)

            measurement["ledger"]["reservations"][0]["resolved"] = False
            measurement["ledger"]["reservations"][0]["charged_cpu_ms"] = driver.VERIFIER_CPU_RESERVATION_MS
            measurement["ledger"]["charged_cpu_ms"] = driver.VERIFIER_CPU_RESERVATION_MS
            measurement["ledger"]["remaining_cpu_ms"] = (
                driver.MISSION_VERIFICATION_CPU_BUDGET_MS - driver.VERIFIER_CPU_RESERVATION_MS
            )
            with mock.patch.object(
                driver.subprocess,
                "run",
                return_value=SimpleNamespace(
                    returncode=0,
                    stdout=json.dumps(measurement).encode(),
                    stderr=b"",
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "durably resolved"):
                    driver.run_journaled_verifier_test(
                        host, journal, "mission012-live-mcp-001", reservation_id,
                        snapshot, root / "target-2", sysroot, sha256(b"trusted test host"),
                    )

    def test_verifier_cpu_reservation_ids_bind_stage_and_both_commits(self) -> None:
        base, candidate = "a" * 40, "b" * 40
        base_id = driver.verifier_reservation_id("base-red", base, candidate)
        self.assertEqual(base_id, driver.verifier_reservation_id("base-red", base, candidate))
        self.assertNotEqual(base_id, driver.verifier_reservation_id("candidate-green", base, candidate))
        self.assertNotEqual(base_id, driver.verifier_reservation_id("base-red", "c" * 40, candidate))
        self.assertNotEqual(base_id, driver.verifier_reservation_id("base-red", base, "d" * 40))
        with self.assertRaisesRegex(RuntimeError, "unknown host-owned verifier stage"):
            driver.verifier_reservation_id("agent-controlled", base, candidate)

    def test_base_red_receipt_requires_the_expected_acceptance_failure(self) -> None:
        base, candidate = "a" * 40, "b" * 40
        output = b"error: could not compile the acceptance test crate"
        result = {
            "mission_id": "mission-v1-" + "c" * 64,
            "reservation_id": driver.verifier_reservation_id("base-red", base, candidate),
            "policy_sha256": driver.VERIFIER_CPU_POLICY_SHA256,
            "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
            "charged_cpu_ms": 4400,
            "accounting_elapsed_ms": 1200,
            "elapsed_ms": 800,
            "exit_code": 101,
            "timed_out": False,
            "output_truncated": False,
            "output_sha256": driver.digest(output),
            "output": output,
        }
        with self.assertRaisesRegex(RuntimeError, "expected acceptance assertion failure"):
            driver.verifier_cpu_run_evidence(
                "base-red", base, candidate, base, result,
                expected_mission_id=result["mission_id"],
            )

    def test_verifier_cpu_receipt_rejects_a_measurement_from_another_mission(self) -> None:
        base, candidate = "a" * 40, "b" * 40
        admitted_mission = "mission-v1-" + "a" * 64
        output = (
            b"test values_outside_the_range_clamp_to_the_nearest_boundary\n"
            b"assertion `left == right` failed\n left: -2\n right: 10\n"
            b"1 passed; 1 failed\n"
        )
        result = {
            "mission_id": "mission-v1-" + "c" * 64,
            "reservation_id": driver.verifier_reservation_id("base-red", base, candidate),
            "policy_sha256": driver.VERIFIER_CPU_POLICY_SHA256,
            "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
            "charged_cpu_ms": 4400,
            "accounting_elapsed_ms": 1200,
            "elapsed_ms": 800,
            "exit_code": 101,
            "timed_out": False,
            "output_truncated": False,
            "output_sha256": driver.digest(output),
            "output": output,
        }
        with self.assertRaisesRegex(RuntimeError, "mission identity"):
            driver.verifier_cpu_run_evidence(
                "base-red", base, candidate, base, result,
                expected_mission_id=admitted_mission,
            )

    def test_cpu_verifier_binary_must_match_admission_build_provenance(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            state = root / "state"
            artifact = state / "build/host-artifacts/mission-012-mcp-host"
            artifact.parent.mkdir(parents=True, mode=0o700)
            artifact.write_bytes(b"admission-built verifier")
            artifact.chmod(0o700)
            evidence = root / "admission-evidence"
            evidence.mkdir(mode=0o700)
            provenance = {
                "mcp_host_built": True,
                "private_state_dir": str(state),
                "built_executable_sha256": {
                    "mission-012-mcp-host": sha256(artifact.read_bytes()),
                },
            }
            provenance_bytes = json.dumps(provenance, sort_keys=True).encode()
            provenance_path = evidence / "provenance.json"
            provenance_path.write_bytes(provenance_bytes)
            provenance_path.chmod(0o600)
            manifest = evidence / "SHA256SUMS"
            manifest.write_text(f"{sha256(provenance_bytes)}  provenance.json\n")
            manifest.chmod(0o600)

            self.assertEqual(
                driver.verified_mcp_host_binary(state, evidence),
                (artifact, sha256(artifact.read_bytes())),
            )
            artifact.write_bytes(b"untrusted replacement")
            with self.assertRaisesRegex(RuntimeError, "differs from admission build provenance"):
                driver.verified_mcp_host_binary(state, evidence)

    def test_final_verifier_cpu_ledger_requires_each_pr_step_to_be_resolved(self) -> None:
        base, candidate = "a" * 40, "b" * 40
        reservation_id = driver.verifier_reservation_id("base-red", base, candidate)
        run = {
            "reservation_id": reservation_id,
            "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
            "charged_cpu_upper_bound_ms": 4000,
        }
        ledger = {
            "mission_id": "mission-v1-" + "c" * 64,
            "policy_sha256": driver.VERIFIER_CPU_POLICY_SHA256,
            "mission_cpu_budget_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS,
            "charged_cpu_ms": 4000,
            "remaining_cpu_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS - 4000,
            "reservations": [{
                "reservation_id": reservation_id,
                "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
                "charged_cpu_ms": 4000,
                "resolved": True,
            }],
        }
        run["mission_id"] = ledger["mission_id"]
        self.assertEqual(
            driver.validate_final_verifier_cpu_ledger(
                ledger, [run], expected_mission_id=ledger["mission_id"]
            ),
            ledger,
        )
        unresolved = json.loads(json.dumps(ledger))
        unresolved["reservations"][0]["resolved"] = False
        unresolved["reservations"][0]["charged_cpu_ms"] = driver.VERIFIER_CPU_RESERVATION_MS
        unresolved["charged_cpu_ms"] = driver.VERIFIER_CPU_RESERVATION_MS
        unresolved["remaining_cpu_ms"] = (
            driver.MISSION_VERIFICATION_CPU_BUDGET_MS - driver.VERIFIER_CPU_RESERVATION_MS
        )
        with self.assertRaisesRegex(RuntimeError, "durable CPU resolution"):
            driver.validate_final_verifier_cpu_ledger(
                unresolved, [run], expected_mission_id=ledger["mission_id"]
            )

        unresolved_extra = json.loads(json.dumps(ledger))
        unresolved_extra["reservations"].append({
            "reservation_id": "unrelated-unresolved-attempt",
            "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
            "charged_cpu_ms": 1000,
            "resolved": False,
        })
        unresolved_extra["charged_cpu_ms"] += 1000
        unresolved_extra["remaining_cpu_ms"] -= 1000
        with self.assertRaisesRegex(RuntimeError, "unresolved reservation"):
            driver.validate_final_verifier_cpu_ledger(
                unresolved_extra, [run], expected_mission_id=ledger["mission_id"]
            )

    def test_final_verifier_cpu_ledger_rejects_mission_identity_mismatch(self) -> None:
        reservation_id = driver.verifier_reservation_id("base-red", "a" * 40, "b" * 40)
        run = {
            "mission_id": "mission-v1-" + "a" * 64,
            "reservation_id": reservation_id,
            "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
            "charged_cpu_upper_bound_ms": 4000,
        }
        ledger = {
            "mission_id": "mission-v1-" + "c" * 64,
            "policy_sha256": driver.VERIFIER_CPU_POLICY_SHA256,
            "mission_cpu_budget_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS,
            "charged_cpu_ms": 4000,
            "remaining_cpu_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS - 4000,
            "reservations": [{
                "reservation_id": reservation_id,
                "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
                "charged_cpu_ms": 4000,
                "resolved": True,
            }],
        }
        with self.assertRaisesRegex(RuntimeError, "mission identity"):
            driver.validate_final_verifier_cpu_ledger(
                ledger, [run], expected_mission_id=run["mission_id"]
            )

    def test_premerge_verifier_output_is_persisted_with_its_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            evidence = Path(temp)
            output = b"running 2 tests\ntest result: ok. 2 passed; 0 failed\n"

            receipt = driver.write_premerge_verifier_output(evidence, output)

            output_path = evidence / "premerge-reverification-cargo-test.log"
            self.assertEqual(output_path.read_bytes(), output)
            self.assertEqual(receipt["output_file"], output_path.name)
            self.assertEqual(receipt["output_sha256"], sha256(output))

    def test_final_verifier_cpu_ledger_rejects_fractional_totals(self) -> None:
        reservation_id = driver.verifier_reservation_id("base-red", "a" * 40, "b" * 40)
        mission_id = "mission-v1-" + "c" * 64
        run = {
            "mission_id": mission_id,
            "reservation_id": reservation_id,
            "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
            "charged_cpu_upper_bound_ms": 4000,
        }
        ledger = {
            "mission_id": mission_id,
            "policy_sha256": driver.VERIFIER_CPU_POLICY_SHA256,
            "mission_cpu_budget_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS,
            "charged_cpu_ms": 4000,
            "remaining_cpu_ms": driver.MISSION_VERIFICATION_CPU_BUDGET_MS - 4000,
            "reservations": [{
                "reservation_id": reservation_id,
                "reserved_cpu_ms": driver.VERIFIER_CPU_RESERVATION_MS,
                "charged_cpu_ms": 4000,
                "resolved": True,
            }],
        }
        for field, value in (
            ("charged_cpu_ms", 4000.0),
            ("remaining_cpu_ms", float(driver.MISSION_VERIFICATION_CPU_BUDGET_MS - 4000)),
        ):
            corrupted = {**ledger, field: value}
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, "reconcile"):
                driver.validate_final_verifier_cpu_ledger(
                    corrupted, [run], expected_mission_id=mission_id
                )

    def test_listener_requires_loopback_and_the_expected_process(self) -> None:
        good = 'LISTEN 0 4096 127.0.0.1:3132 0.0.0.0:* users:(("forgejo",pid=1234,fd=6))\n'
        self.assertTrue(driver.listener_output_is_loopback(good, 1234))
        self.assertFalse(driver.listener_output_is_loopback(good.replace("127.0.0.1", "0.0.0.0"), 1234))
        self.assertFalse(driver.listener_output_is_loopback(good.replace("pid=1234", "pid=4321"), 1234))
        shared = good.replace('fd=6))', 'fd=6),("other",pid=4321,fd=7))')
        self.assertFalse(driver.listener_output_is_loopback(shared, 1234))
        self.assertFalse(driver.listener_output_is_loopback(good + good.replace("127.0.0.1", "*"), 1234))

    def test_branch_protection_requires_current_base_gate(self) -> None:
        valid = {
            "branch_name": "main",
            "required_approvals": 1,
            "enable_approvals_whitelist": True,
            "approvals_whitelist_username": ["verifier-bot"],
            "enable_merge_whitelist": True,
            "merge_whitelist_usernames": ["forge-admin"],
            "merge_whitelist_teams": [],
            "apply_to_admins": True,
            "dismiss_stale_approvals": True,
            "block_on_outdated_branch": True,
            "enable_push": False,
        }
        self.assertTrue(driver.branch_protection_matches_policy(valid))
        self.assertFalse(driver.branch_protection_matches_policy({**valid, "enable_push": True}))
        self.assertFalse(driver.branch_protection_matches_policy({k: v for k, v in valid.items() if k != "enable_push"}))
        self.assertFalse(driver.branch_protection_matches_policy({**valid, "merge_whitelist_teams": ["release-managers"]}))
        self.assertFalse(driver.branch_protection_matches_policy({k: v for k, v in valid.items() if k != "merge_whitelist_teams"}))
        self.assertFalse(driver.branch_protection_matches_policy({**valid, "block_on_outdated_branch": False}))
        self.assertFalse(driver.branch_protection_matches_policy({k: v for k, v in valid.items() if k != "block_on_outdated_branch"}))

    def test_merge_parent_validation_rejects_duplicate_or_extra_parents(self) -> None:
        base, candidate = "a" * 40, "b" * 40
        self.assertTrue(driver.merge_parents_match([base, candidate], base, candidate))
        self.assertFalse(driver.merge_parents_match([base, candidate, candidate], base, candidate))
        self.assertFalse(driver.merge_parents_match([base, base], base, candidate))

    def test_rust_merge_policy_projects_versioned_fixture_and_one_reviewer(self) -> None:
        source_policy = json.loads(
            (ROOT / "tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v4.json").read_bytes()
        )

        policy = driver.build_rust_merge_policy(42)

        self.assertEqual(policy["schema_version"], 1)
        self.assertEqual(policy["revision"], source_policy["revision"])
        self.assertEqual(policy["verifier_id"], driver.MISSION_VERIFIER_ID)
        self.assertEqual(
            policy["environment"],
            f"{source_policy['verification']['environment']}; source policy sha256={driver.POLICY_SHA}",
        )
        self.assertEqual(policy["acceptance_command"], source_policy["verification"]["command"])
        self.assertEqual(
            policy["protected_criteria"],
            source_policy["candidate_policy"]["protected_unchanged_paths"],
        )
        self.assertEqual(
            policy["merge_approval_policy"],
            {"schema_version": 1, "required_approvals": 1, "authorized_reviewer_ids": [42]},
        )
        self.assertNotIn("forge-admin", policy["merge_approval_policy"]["authorized_reviewer_ids"])

    def test_rust_merge_gate_result_must_confirm_exact_candidate_and_readback(self) -> None:
        base, candidate, merge = "a" * 40, "b" * 40, "c" * 40
        admission = {
            "issue_number": 1,
            "requirement_sha256": "d" * 64,
            "request": {},
            "allocation": {
                "mission_id": "mission-012",
                "allocation_id": "allocation-1",
                "execution_generation": 1,
            },
        }
        admission_sha256 = "e" * 64
        result = {
            "schema_version": 1,
            "operation": "local_test_merge_authority",
            "verdict": "merged",
            "pull_request_number": 2,
            "merge_authorized": True,
            "merge_attempted": True,
            "http_status": 200,
            "submitted_head_sha": candidate,
            "validated_base_sha": base,
            "merge_commit_sha": merge,
            "main_head_sha": merge,
            "postmerge_base_sha": merge,
            "merge_commit_base_parent_sha": base,
            "merge_commit_head_parent_sha": candidate,
            "issue_state": "closed",
            "mission_admission": {
                "schema_version": 1,
                "admission_sha256": admission_sha256,
                "issue_number": 1,
                "mission_id": "mission-012",
                "allocation_id": "allocation-1",
                "execution_generation": 1,
                "request_sha256": "f" * 64,
                "base_commit": base,
                "requirement_sha256": "d" * 64,
            },
            "target": {
                "selector": "mission-012-clamp",
                "origin": driver.ORIGIN,
                "owner": driver.OWNER,
                "repository": driver.REPOSITORY,
                "base_branch": "main",
            },
        }

        self.assertIsNone(driver.validate_rust_merge_gate_result(
            result, pull_request=2, base=base, candidate=candidate,
            admission=admission, admission_sha256=admission_sha256,
        ))
        for field, value in (
            ("merge_authorized", False),
            ("submitted_head_sha", "d" * 40),
            ("postmerge_base_sha", base),
            ("issue_state", "open"),
        ):
            with self.subTest(field=field):
                bad = {**result, field: value}
                with self.assertRaisesRegex(RuntimeError, "Rust merge authority"):
                    driver.validate_rust_merge_gate_result(
                        bad, pull_request=2, base=base, candidate=candidate,
                        admission=admission, admission_sha256=admission_sha256,
                    )

    def test_subprocess_gate_receives_process_bound_capability_and_owner_token_by_fd(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            executable = root / "fake-rust-gate"
            executable.write_text(
                "#!/usr/bin/python3\n"
                "import json, os, socket, struct, sys\n"
                "cap_fd = int(sys.argv[sys.argv.index('--supervisor-capability-fd') + 1])\n"
                "packet = json.loads(os.read(cap_fd, 8193))\n"
                "token_fd = int(sys.argv[-1])\n"
                "token = os.read(token_fd, 4096)\n"
                "assert token == b'fixture-secret-token'\n"
                "assert b'fixture-secret-token' not in ' '.join(sys.argv).encode()\n"
                "channel = socket.socket(fileno=os.dup(cap_fd))\n"
                "peer = struct.unpack('3i', channel.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))\n"
                "assert peer[0] == packet['supervisor']['pid']\n"
                "assert packet['gate']['pid'] == os.getpid()\n"
                "assert packet['forgejo']['pid'] == packet['supervisor']['pid']\n"
                "audit = {k: packet[k] for k in ('schema_version','target','supervisor','supervisor_uid','gate','forgejo','expires_at_monotonic_ns')}\n"
                "audit.update({'nonce_sha256': 'a' * 64, 'pre_post_revalidated': True, 'supervisor_policy_independently_verified': False, 'profile': 'local_single_writer_sandbox'})\n"
                "print(json.dumps({'verdict': 'rejected', 'writer_admission': audit}))\n"
            )
            executable.chmod(0o700)
            inputs = {
                "pull_request": 2,
                "proof": root / "proof.json",
                "expected": root / "expected.json",
                "key": root / "key",
                "policy": root / "policy.json",
                "binding": root / "binding.json",
                "admission": root / "admission.json",
                "journal": root / "mission-execution.jsonl",
            }
            for key in ("proof", "expected", "key", "policy", "binding", "admission", "journal"):
                inputs[key].write_bytes(b"fixture")
                inputs[key].chmod(0o600)

            status, result, launch = driver.run_rust_merge_gate(
                executable,
                pull_request=inputs["pull_request"],
                forgejo_pid=os.getpid(),
                proof=inputs["proof"],
                expected=inputs["expected"],
                key=inputs["key"],
                policy=inputs["policy"],
                review_base_binding=inputs["binding"],
                admission_file=inputs["admission"],
                mission_journal=inputs["journal"],
                owner_token="fixture-secret-token",
            )

        self.assertEqual(status, 0)
        self.assertEqual(result["verdict"], "rejected")
        self.assertTrue(result["writer_admission"]["pre_post_revalidated"])
        self.assertTrue(launch["capability_sent_before_gate_exec"])
        self.assertEqual(launch["gate"]["pid"], result["writer_admission"]["gate"]["pid"])

    def test_receipt_must_be_private_hash_bound_and_finalized_candidate_ready(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_root = Path(temp)
            root = temp_root / "candidate"
            root.mkdir(mode=0o700)
            receipt = root / "host-receipts.jsonl"
            binding = {
                "allocation_id": "allocation-1",
                "base_commit": "a" * 40,
                "execution_generation": 2,
                "mission_id": "mission-1",
                "requirement_sha256": "b" * 64,
                "workspace_id": "workspace-1",
            }
            rows = [
                {"event": "execution_started", "execution_binding": binding},
                {
                    "event": "host_finalization", "outcome": "candidate_ready",
                    "execution_binding": binding, "result": {"changed_paths": ["src/lib.rs"]},
                },
            ]
            data = b"".join(json.dumps(row, sort_keys=True).encode() + b"\n" for row in rows)
            receipt.write_bytes(data)
            os.chmod(receipt, 0o600)
            summary = {
                "host_receipt_path": str(receipt),
                "receipt_sha256": sha256(data),
                "receipt_count": len(rows),
                "changed_paths": ["src/lib.rs"],
                "tool_names_from_receipts": [],
            }
            self.assertEqual(driver.verify_host_receipt(summary, root, binding), sha256(data))

            mismatched_rows = [rows[0], {**rows[1], "execution_binding": {**binding, "base_commit": "c" * 40}}]
            mismatched_data = b"".join(json.dumps(row, sort_keys=True).encode() + b"\n" for row in mismatched_rows)
            receipt.write_bytes(mismatched_data)
            os.chmod(receipt, 0o600)
            mismatched_summary = dict(summary, receipt_sha256=sha256(mismatched_data))
            with self.assertRaisesRegex(RuntimeError, "execution binding"):
                driver.verify_host_receipt(mismatched_summary, root, binding)
            receipt.write_bytes(data)
            os.chmod(receipt, 0o600)

            alias = root / "receipt-link.jsonl"
            alias.symlink_to(receipt.name)
            bad_path = dict(summary, host_receipt_path=str(alias))
            with self.assertRaisesRegex(RuntimeError, "receipt"):
                driver.verify_host_receipt(bad_path, root)

            root_alias = temp_root / "candidate-root-link"
            root_alias.symlink_to(root, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "candidate root"):
                driver.verify_host_receipt(summary, root_alias)

            os.chmod(root, 0o755)
            with self.assertRaisesRegex(RuntimeError, "receipt"):
                driver.verify_host_receipt(summary, root)
            os.chmod(root, 0o700)

            bad_hash = dict(summary, receipt_sha256="0" * 64)
            with self.assertRaisesRegex(RuntimeError, "receipt"):
                driver.verify_host_receipt(bad_hash, root)

            nonfinal = data.splitlines()[0] + b"\n"
            receipt.write_bytes(nonfinal)
            os.chmod(receipt, 0o600)
            bad_finalization = dict(summary, receipt_sha256=sha256(nonfinal), receipt_count=1)
            with self.assertRaisesRegex(RuntimeError, "finalization"):
                driver.verify_host_receipt(bad_finalization, root)

    def test_restart_receipt_requires_two_consistent_fenced_host_segments(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "candidate"
            root.mkdir(mode=0o700)
            receipt = root / "host-receipts.jsonl"
            admitted = {
                "allocation_id": "allocation-1",
                "base_commit": "a" * 40,
                "execution_generation": 2,
                "mission_id": "mission-v1-" + "f" * 64,
                "requirement_sha256": "b" * 64,
                "workspace_id": "workspace-1",
            }
            first_binding = {
                **admitted,
                "fencing_token": 1,
                "holder_id": "host-1",
                "lease_id": "lease-1",
                "issued_at_unix_ms": 100,
                "expires_at_unix_ms": 10_100,
            }
            recovery_binding = {
                **admitted,
                "fencing_token": 2,
                "holder_id": "host-1",
                "lease_id": "lease-2",
                "issued_at_unix_ms": 10_200,
                "expires_at_unix_ms": 130_200,
            }
            rows = [
                {"event": "execution_started", "execution_binding": first_binding},
                {
                    "event": "tool_call", "tool_id": "workspace.write_text",
                    "execution_binding": first_binding,
                },
                {"event": "execution_started", "execution_binding": recovery_binding},
                {
                    "event": "host_finalization", "outcome": "candidate_ready",
                    "execution_binding": recovery_binding,
                    "result": {"changed_paths": ["src/lib.rs"]},
                },
            ]
            segments = [
                b"".join(json.dumps(row, sort_keys=True).encode() + b"\n" for row in rows[:2]),
                b"".join(json.dumps(row, sort_keys=True).encode() + b"\n" for row in rows[2:]),
            ]
            data = b"".join(segments)
            receipt.write_bytes(data)
            os.chmod(receipt, 0o600)
            restart = {
                "old_fencing_token": 1,
                "new_fencing_token": 2,
                "initial_lease_id": "lease-1",
                "recovery_lease_id": "lease-2",
                "initial_receipt_count": 2,
                "recovery_receipt_count": 2,
                "initial_receipt_sha256": sha256(segments[0]),
                "recovery_receipt_sha256": sha256(segments[1]),
                "combined_receipt_sha256": sha256(data),
            }
            summary = {
                "host_receipt_path": str(receipt),
                "receipt_sha256": sha256(data),
                "receipt_count": len(rows),
                "changed_paths": ["src/lib.rs"],
                "tool_names_from_receipts": ["workspace.write_text"],
                "restart_recovery": restart,
            }

            self.assertEqual(driver.verify_host_receipt(summary, root, admitted), sha256(data))

            malformed = dict(summary, restart_recovery={**restart, "recovery_receipt_count": 1})
            with self.assertRaisesRegex(RuntimeError, "restart segment"):
                driver.verify_host_receipt(malformed, root, admitted)


if __name__ == "__main__":
    unittest.main()
