from __future__ import annotations

import hashlib
import hmac
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import mission_012_test_agent_adapter as test_adapter  # noqa: E402
import mission_012_sandboxed_test_adapter as sandbox_wrapper  # noqa: E402
import run_forgejo_live_pr_merge_e2e as runner  # noqa: E402
import run_forgejo_native_actions_pending_webhook_e2e as native_actions  # noqa: E402
import run_mission_012_stdio_mcp_egress_gate_e2e as stdio_runner  # noqa: E402


class FakeClock:
    def __init__(self) -> None:
        self.now = 0.0

    def monotonic(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.now += seconds


class UpdatedPrMergeabilityTests(unittest.TestCase):
    def test_waits_for_async_recheck_instead_of_accepting_stale_true(self) -> None:
        states = iter((True, False, True, True, True))
        calls = 0

        def api(_method: str, _path: str) -> tuple[int, dict[str, object], dict[str, str]]:
            nonlocal calls
            calls += 1
            return (
                200,
                {
                    "state": "open",
                    "head": {"sha": "candidate"},
                    "base": {"sha": "base"},
                    "mergeable": next(states),
                },
                {},
            )

        clock = FakeClock()
        with (
            mock.patch.object(runner.time, "monotonic", side_effect=clock.monotonic),
            mock.patch.object(runner.time, "sleep", side_effect=clock.sleep),
        ):
            current, observations = runner.wait_for_updated_pr_mergeability(
                api,
                "/pulls/2",
                expected_head_sha="candidate",
                expected_base_sha="base",
            )

        self.assertEqual(calls, 5)
        self.assertEqual([item["mergeable"] for item in observations], [True, False, True, True, True])
        self.assertIs(current["mergeable"], True)
        self.assertGreaterEqual(clock.now, 1.0)


class ForgejoActionsConfigurationTests(unittest.TestCase):
    def test_actions_remain_disabled_by_default_and_can_be_enabled_explicitly(self) -> None:
        with tempfile.TemporaryDirectory(prefix="forgejo-actions-config-") as directory:
            root = Path(directory)
            disabled_state = root / "disabled"
            enabled_state = root / "enabled"
            runner.create_app_ini(disabled_state)
            runner.create_app_ini(enabled_state, actions_enabled=True)

            disabled_config = (disabled_state / "custom/conf/app.ini").read_text()
            enabled_config = (enabled_state / "custom/conf/app.ini").read_text()

        self.assertIn("[actions]\nENABLED = false\n", disabled_config)
        self.assertIn("[actions]\nENABLED = true\n", enabled_config)


class ComposedStdioFixtureConsistencyTests(unittest.TestCase):
    def test_egress_composition_reuses_the_pinned_agent_task_fixture(self) -> None:
        initial, fixed, acceptance, failure_markers = runner.build_stdio_mcp_candidate_fixture()

        self.assertEqual(stdio_runner.INITIAL_SOURCE, initial.decode("utf-8"))
        self.assertEqual(stdio_runner.FIXED_SOURCE, fixed.decode("utf-8"))
        self.assertEqual(stdio_runner.ACCEPTANCE_SOURCE, acceptance.decode("utf-8"))
        self.assertEqual(stdio_runner.ACTIONABLE_FAILURE_MARKERS, failure_markers)

    def test_route_probe_gate_requires_connect_phase_for_every_direct_route(self) -> None:
        diagnostics = [
            {"name": "api", "phase": "connect", "error_category": "os_error", "errno": 101},
            {"name": "web", "phase": "connect", "error_category": "os_error", "errno": 101},
            {"name": "git_receive_pack", "phase": "connect", "error_category": "os_error", "errno": 101},
        ]
        validator = getattr(stdio_runner, "validate_direct_route_probe_diagnostics", None)
        self.assertTrue(callable(validator), "the gate must validate route probe stages")
        self.assertIsNone(validator(diagnostics))

        send_failure = [*diagnostics]
        send_failure[1] = {**diagnostics[1], "phase": "send"}
        with self.assertRaisesRegex(RuntimeError, "connect stage"):
            validator(send_failure)

        incomplete = diagnostics[:2]
        with self.assertRaisesRegex(RuntimeError, "all direct routes"):
            validator(incomplete)

    def test_live_pr_gate_rejects_route_error_after_connection(self) -> None:
        diagnostics = [
            {"name": "api", "phase": "connect", "error_category": "os_error", "errno": 101},
            {"name": "web", "phase": "connect", "error_category": "os_error", "errno": 101},
            {"name": "git_receive_pack", "phase": "connect", "error_category": "os_error", "errno": 101},
        ]
        validator = getattr(runner, "validate_direct_route_probe_errors", None)
        self.assertTrue(callable(validator), "the live PR gate must validate route probe stages")
        self.assertIsNone(validator(diagnostics))

        send_failure = [*diagnostics]
        send_failure[2] = {**diagnostics[2], "phase": "send"}
        with self.assertRaisesRegex(RuntimeError, "connect stage"):
            validator(send_failure)


class NativeActionsDeliveryBindingTests(unittest.TestCase):
    def test_pending_workflow_run_event_is_bound_to_real_run_and_delivery(self) -> None:
        commit = "a" * 40
        record = {
            "sequence": 1,
            "idempotency_key": "forgejo-delivery-v1:native-run-1",
            "kind": "forgejo.webhook.v1.workflow_run_waiting",
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "delivery_id": "native-run-1",
                "event": "workflow_run_waiting",
                "payload": {
                    "action": "new_run_attempt",
                    "run": {
                        "id": 7001,
                        "index_in_repo": 41,
                        "status": "waiting",
                        "commit_sha": commit,
                        "repository": {"full_name": f"forge-admin/{native_actions.REPOSITORY}"},
                    },
                },
            },
        }

        parsed = native_actions.validate_native_run_delivery(
            record, run_id=7001, run_index=41, commit_sha=commit
        )

        self.assertEqual(parsed["event"], "workflow_run_waiting")
        self.assertEqual(parsed["action"], "new_run_attempt")
        self.assertEqual(parsed["status_at_event"], "waiting")

    def test_native_actions_delivery_refuses_identity_or_status_mismatch(self) -> None:
        commit = "a" * 40
        record = {
            "sequence": 1,
            "idempotency_key": "forgejo-delivery-v1:native-run-1",
            "kind": "forgejo.webhook.v1.workflow_run_waiting",
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "delivery_id": "native-run-1",
                "event": "workflow_run_waiting",
                "payload": {
                    "action": "new_run_attempt",
                    "run": {
                        "id": 7001,
                        "index_in_repo": 41,
                        "status": "waiting",
                        "commit_sha": commit,
                        "repository": {"full_name": f"forge-admin/{native_actions.REPOSITORY}"},
                    },
                },
            },
        }

        for change in (
            lambda value: value["payload"]["payload"]["run"].update(id=7002),
            lambda value: value["payload"]["payload"]["run"].update(status="success"),
            lambda value: value["payload"]["payload"]["run"]["repository"].update(
                full_name="attacker/repo"
            ),
        ):
            candidate = json.loads(json.dumps(record))
            change(candidate)
            with self.subTest(candidate=candidate["payload"]["payload"]):
                with self.assertRaises(ValueError):
                    native_actions.validate_native_run_delivery(
                        candidate, run_id=7001, run_index=41, commit_sha=commit
                    )


class SemanticCanaryOptionTests(unittest.TestCase):
    def test_semantic_canary_is_standalone_from_webhook_and_egress_compositions(self) -> None:
        runner.validate_semantic_canary_options(
            semantic=True,
            observe=False,
            replay=False,
            restart_before_replay=False,
            sandbox_egress=False,
            stdio_mcp_egress=False,
        )
        with self.assertRaisesRegex(RuntimeError, "cannot be combined"):
            runner.validate_semantic_canary_options(
                semantic=True,
                observe=True,
                replay=False,
                restart_before_replay=False,
                sandbox_egress=False,
                stdio_mcp_egress=False,
            )


class StdioMcpLiveFixtureTests(unittest.TestCase):
    def test_supervisor_observes_sandboxed_agent_namespaces_from_proc(self) -> None:
        if not sandbox_wrapper.BWRAP.is_file() or not sandbox_wrapper.PYTHON.is_file():
            self.skipTest("local Bubblewrap/Python runtime is unavailable")

        with tempfile.TemporaryDirectory(prefix="mission012-proc-agent-observation-") as directory:
            process, child_report = runner.start_sandbox_probe_during_merge(
                Path(directory),
                owner=runner.OWNER,
                repository=runner.REPOSITORY,
                pull_number=1,
            )
            try:
                observation = stdio_runner.observe_sandbox_agent_process_tree(process.pid)

                self.assertTrue(observation["agent_alive"])
                self.assertTrue(observation["observed_by_parent_procfs"])
                self.assertNotEqual(
                    observation["agent_network_namespace"],
                    observation["host_network_namespace"],
                )
                self.assertNotEqual(
                    observation["agent_pid_namespace"],
                    observation["host_pid_namespace"],
                )
                self.assertEqual(
                    observation["agent_network_namespace"],
                    child_report["network_namespace"],
                )
                self.assertEqual(
                    observation["agent_entrypoint"], "mission_012_test_agent_adapter.py"
                )
            finally:
                runner.close_sandbox_probe(process)

    def test_supervisor_rejects_a_host_process_as_sandbox_agent(self) -> None:
        process = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(2)"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
            start_new_session=True,
        )
        try:
            with self.assertRaisesRegex(RuntimeError, "sandboxed agent descendant"):
                stdio_runner.observe_sandbox_agent_process_tree(process.pid)
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=3)

    def test_expected_mcp_protocol_includes_shutdown_after_task_calls(self) -> None:
        task_methods = [
            "initialize",
            "tools/list",
            "tools/call:mission_context",
            "tools/call:cargo_test",
            "tools/call:workspace_read_text",
            "tools/call:workspace_read_text",
            "tools/call:workspace_write_text",
            "tools/call:cargo_test",
        ]
        self.assertEqual(
            stdio_runner.expected_mcp_methods(wait_at_gate_barrier=False),
            [*task_methods, "shutdown"],
        )
        self.assertEqual(
            stdio_runner.expected_mcp_methods(wait_at_gate_barrier=True),
            [*task_methods, "tools/call:mission_context", "shutdown"],
        )

    def test_live_fixture_is_accepted_and_corrected_by_the_test_adapter(self) -> None:
        initial, expected, _acceptance, failure_markers = (
            runner.build_stdio_mcp_candidate_fixture()
        )
        initial_source = initial.decode("utf-8")
        expected_fixed_source = expected.decode("utf-8")

        self.assertEqual(initial_source.count(test_adapter.SOURCE_BUG), 1)
        self.assertEqual(
            expected_fixed_source,
            initial_source.replace(test_adapter.SOURCE_BUG, test_adapter.SOURCE_FIX, 1),
        )
        self.assertEqual(failure_markers, ("left: -2", "right: 10"))

    def test_live_mcp_config_passes_through_the_trusted_host_separator(self) -> None:
        with tempfile.TemporaryDirectory(prefix="stdio-mcp-live-config-") as directory:
            root = Path(directory)
            runtime = root / ".runtime"
            runtime.mkdir(mode=0o700)
            ready = runtime / "mcp-gate-ready.json"
            release = runtime / "mcp-gate-release.json"
            config = root / "mcp-config.json"
            host_arguments = ["--workspace", "/tmp/mission-work", "--calls-file", "/tmp/calls.jsonl"]
            config.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "mission012": {
                                "command": "/usr/bin/python3",
                                "args": stdio_runner.build_mcp_server_arguments(host_arguments),
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            config.chmod(0o600)

            command = sandbox_wrapper.supervisor_command(
                config,
                gate_barrier_ready_file=ready,
                gate_barrier_release_file=release,
            )

        self.assertEqual(
            command,
            [
                "/usr/bin/python3",
                str(stdio_runner.SCRIPT_DIR / "mission_012_mcp_supervisor.py"),
                "--gate-barrier-ready",
                str(ready),
                "--gate-barrier-release",
                str(release),
                "--",
                "/usr/bin/python3",
                str(stdio_runner.MCP_HOST),
                *host_arguments,
            ],
        )
        with self.assertRaisesRegex(ValueError, "entrypoint must not be repeated"):
            stdio_runner.build_mcp_server_arguments(
                [str(stdio_runner.MCP_HOST), *host_arguments]
            )


class InitialCandidateVerifierTests(unittest.TestCase):
    def test_stdio_candidate_must_fail_upper_bound_cases_and_pass_orthogonal_cases(self) -> None:
        output = b"""below_min_returns_min ... ok
above_max_returns_max ... FAILED
in_range_is_preserved ... ok
equal_lower_bound_is_preserved ... ok
equal_upper_bound_is_preserved ... ok
singleton_interval_clamps ... ok
negative_interval_clamps ... ok
i32_min_clamps ... ok
i32_max_clamps ... FAILED
7 passed; 2 failed
"""
        runner.validate_initial_candidate_verifier_result(101, output, stdio_mcp=True)

        with self.assertRaisesRegex(RuntimeError, "expected actionable failure"):
            runner.validate_initial_candidate_verifier_result(
                0, output, stdio_mcp=True
            )
        with self.assertRaisesRegex(RuntimeError, "expected actionable failure"):
            runner.validate_initial_candidate_verifier_result(
                101,
                output.replace(b"above_max_returns_max ... FAILED", b"above_max_returns_max ... ok"),
                stdio_mcp=True,
            )


class SupervisorCapabilityIssuanceTests(unittest.TestCase):
    def test_packet_binds_the_live_gate_forgejo_and_exact_pull_request_target(self) -> None:
        packet, record = runner.build_supervisor_capability_packet(
            gate_pid=os.getpid(),
            forgejo_pid=os.getpid(),
            target_profile="merge-014-agent-forge-core",
            pull_request_number=7,
        )
        envelope = json.loads(packet)

        self.assertEqual(envelope["schema_version"], 1)
        self.assertEqual(
            envelope["target"],
            {
                "profile_selector": "merge-014-agent-forge-core",
                "origin": runner.ORIGIN,
                "owner": runner.OWNER,
                "repository": runner.REPOSITORY,
                "pull_request_number": 7,
                "base_branch": "main",
            },
        )
        self.assertEqual(envelope["supervisor"], envelope["gate"])
        self.assertEqual(envelope["gate"], envelope["forgejo"])
        self.assertEqual(record["target"], envelope["target"])
        self.assertEqual(record["capability_packet_sha256"], hashlib.sha256(packet).hexdigest())


class BoundedProcessSpawnHookTests(unittest.TestCase):
    def test_spawn_hook_observes_the_live_child_before_it_finishes(self) -> None:
        observed: list[int] = []

        def observe(process: subprocess.Popen[bytes]) -> None:
            self.assertGreater(process.pid, 0)
            self.assertIsNone(process.poll())
            observed.append(process.pid)

        status, output = runner.bounded_process(
            [sys.executable, "-c", "import time; time.sleep(0.05); print('ready')"],
            after_spawn=observe,
        )

        self.assertEqual(status, 0)
        self.assertEqual(output, b"ready\n")
        self.assertEqual(len(observed), 1)


class PullRequestWebhookBindingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.journal = Path(self.temporary.name) / "inbox.jsonl"
        self.head_sha = "1" * 40
        self.base_sha = "2" * 40
        self.merge_sha = "3" * 40

    def write_closed_record(self, *, head_sha: str | None = None, merge_sha: str | None = None) -> None:
        delivery_id = "delivery-closed-001"
        record = {
            "sequence": 1,
            "kind": "forgejo.webhook.v1.pull_request",
            "idempotency_key": f"forgejo-delivery-v1:{delivery_id}",
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "event": "pull_request",
                "delivery_id": delivery_id,
                "payload": {
                    "action": "closed",
                    "repository": {"full_name": f"{runner.OWNER}/{runner.REPOSITORY}"},
                    "pull_request": {
                        "number": 7,
                        "state": "closed",
                        "merged": True,
                        "merge_commit_sha": merge_sha or self.merge_sha,
                        "head": {"sha": head_sha or self.head_sha},
                        "base": {"sha": self.merge_sha},
                    },
                },
            },
        }
        self.journal.write_text(json.dumps(record) + "\n", encoding="utf-8")

    def test_closed_webhook_is_bound_to_expected_pr_head_and_merge(self) -> None:
        self.write_closed_record()
        event = runner.wait_for_pull_request_webhook(
            self.journal,
            pull_request_number=7,
            action="closed",
            expected_merged=True,
            expected_head_sha=self.head_sha,
            expected_merge_commit_sha=self.merge_sha,
        )
        self.assertEqual(event["head_sha"], self.head_sha)
        self.assertEqual(event["merge_commit_sha"], self.merge_sha)

    def test_closed_webhook_with_another_merge_commit_is_refused(self) -> None:
        self.write_closed_record(merge_sha="4" * 40)
        with self.assertRaisesRegex(RuntimeError, "merge commit differs"):
            runner.wait_for_pull_request_webhook(
                self.journal,
                pull_request_number=7,
                action="closed",
                expected_merged=True,
                expected_head_sha=self.head_sha,
                expected_merge_commit_sha=self.merge_sha,
            )


class SignedWebhookDuplicateReplayTests(unittest.TestCase):
    def setUp(self) -> None:
        self.delivery_id = "delivery-closed-001"
        self.secret = "local-fixture-secret"
        payload = {
            "action": "closed",
            "repository": {"full_name": f"{runner.OWNER}/{runner.REPOSITORY}"},
            "pull_request": {
                "number": 7,
                "state": "closed",
                "merged": True,
                "merge_commit_sha": "3" * 40,
                "head": {"sha": "1" * 40},
                "base": {"sha": "3" * 40},
                "title": "fixture",
            },
        }
        self.record = {
            "sequence": 4,
            "kind": "forgejo.webhook.v1.pull_request",
            "idempotency_key": f"forgejo-delivery-v1:{self.delivery_id}",
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "event": "pull_request",
                "delivery_id": self.delivery_id,
                "payload": payload,
            },
        }

    def test_replay_request_preserves_delivery_identity_and_signs_journaled_payload(self) -> None:
        replay = runner.build_signed_webhook_duplicate_replay(self.record, self.secret)
        raw_body = json.dumps(
            self.record["payload"]["payload"],
            ensure_ascii=False,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")

        self.assertEqual(replay["raw_body"], raw_body)
        self.assertEqual(replay["event_name"], "pull_request")
        self.assertEqual(replay["delivery_id"], self.delivery_id)
        self.assertEqual(replay["sequence"], 4)
        self.assertEqual(
            replay["signature"],
            hmac.new(self.secret.encode("ascii"), raw_body, hashlib.sha256).hexdigest(),
        )
        self.assertNotIn(self.secret, json.dumps({k: v for k, v in replay.items() if k != "raw_body"}))

    def test_replay_request_refuses_inconsistent_journal_binding_and_empty_secret(self) -> None:
        mismatched = json.loads(json.dumps(self.record))
        mismatched["idempotency_key"] = "forgejo-delivery-v1:another-delivery"
        with self.assertRaisesRegex(ValueError, "delivery ID binding"):
            runner.build_signed_webhook_duplicate_replay(mismatched, self.secret)

        with self.assertRaisesRegex(ValueError, "secret must be non-empty"):
            runner.build_signed_webhook_duplicate_replay(self.record, "")


class WebhookReceiverRestartContractTests(unittest.TestCase):
    def test_restart_requires_observed_event_and_duplicate_replay(self) -> None:
        runner.validate_webhook_replay_options(
            observe=True, replay=True, restart_before_replay=True
        )
        with self.assertRaisesRegex(RuntimeError, "requires --observe-pr-webhook"):
            runner.validate_webhook_replay_options(
                observe=False, replay=True, restart_before_replay=False
            )
        with self.assertRaisesRegex(RuntimeError, "requires --replay-pr-webhook"):
            runner.validate_webhook_replay_options(
                observe=True, replay=False, restart_before_replay=True
            )

    def test_graceful_restart_requires_a_live_receiver_and_zero_exit(self) -> None:
        receiver = mock.Mock()
        receiver.pid = 42
        receiver.poll.return_value = None
        receiver.wait.return_value = 0

        stopped = runner.stop_webhook_receiver_for_restart(receiver)

        self.assertEqual(stopped["process_id"], 42)
        self.assertEqual(stopped["stop_signal"], "SIGTERM")
        self.assertEqual(stopped["exit_code"], 0)
        receiver.terminate.assert_called_once_with()

    def test_restart_refuses_an_already_exited_receiver(self) -> None:
        receiver = mock.Mock()
        receiver.poll.return_value = 0
        with self.assertRaisesRegex(RuntimeError, "exited before its controlled restart"):
            runner.stop_webhook_receiver_for_restart(receiver)
        receiver.terminate.assert_not_called()


if __name__ == "__main__":
    unittest.main()
