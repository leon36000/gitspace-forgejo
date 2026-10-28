from __future__ import annotations

from decimal import Decimal
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import run_mission_012_live_mcp_agent_e2e as driver  # noqa: E402
from systemd_process_scope import ProcessScopeSnapshot  # noqa: E402


class AgentCommandTests(unittest.TestCase):
    def test_sandbox_evidence_requires_the_agent_to_lack_the_systemd_bus(self) -> None:
        route_specs = [
            {
                "name": "api",
                "method": "GET",
                "path": "/api/v1/repos/forge-admin/mission-012-clamp/pulls/2",
            },
            {
                "name": "web",
                "method": "GET",
                "path": "/forge-admin/mission-012-clamp/pulls/2",
            },
            {
                "name": "git_receive_pack",
                "method": "POST",
                "path": "/forge-admin/mission-012-clamp.git/git-receive-pack",
            },
        ]
        sandbox_checks = {
            "credential_visible": False,
            "network_connected": False,
            "host_systemd_bus_visible": False,
            "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
            "open_fds": [0, 1, 2],
            "route_probe_specs": route_specs,
            "supervisor_observed_route_connections": 0,
            "private_runtime_files_not_visible": [
                "admission_source",
                "forgejo_read_token",
                "host_admission",
                "invalid_forgejo_read_token",
                "mission_journal",
                "webhook_inbox",
            ],
            "direct_forgejo_route_probe_errors": [
                {"name": name, "phase": "connect", "error_category": "os_error", "errno": 111}
                for name in ("api", "web", "git_receive_pack")
            ],
            "probe_target_port": 3132,
            "network_namespace": "net:[201]",
            "pid_namespace": "pid:[101]",
            "environment_keys": [
                "HOME",
                "LANG",
                "LC_ALL",
                "PATH",
                "PWD",
                "PYTHONDONTWRITEBYTECODE",
            ],
        }
        response = {
            "sandbox_checks": sandbox_checks,
            "host_network_namespace": "net:[200]",
            "host_pid_namespace": "pid:[100]",
        }

        self.assertEqual(
            driver.validate_sandbox_agent_observations(response, {"schema_version": 1}),
            sandbox_checks,
        )
        for changed_checks in (
            {key: value for key, value in sandbox_checks.items() if key != "host_systemd_bus_visible"},
            {**sandbox_checks, "host_systemd_bus_visible": True},
        ):
            with self.subTest(changed=changed_checks.get("host_systemd_bus_visible", "omitted")):
                with self.assertRaisesRegex(RuntimeError, "credential and network refusals"):
                    driver.validate_sandbox_agent_observations(
                        {**response, "sandbox_checks": changed_checks},
                        {"schema_version": 1},
                    )

    def test_sandbox_route_gate_requires_all_routes_to_fail_at_connect(self) -> None:
        diagnostics = [
            {"name": "api", "phase": "connect", "error_category": "os_error", "errno": 111},
            {"name": "web", "phase": "connect", "error_category": "os_error", "errno": 111},
            {"name": "git_receive_pack", "phase": "connect", "error_category": "os_error", "errno": 111},
        ]
        checks = {"direct_forgejo_route_probe_errors": diagnostics}
        self.assertIsNone(driver.validate_sandbox_route_probe_observations(checks))

        send_failure = [*diagnostics]
        send_failure[1] = {**send_failure[1], "phase": "send"}
        with self.assertRaisesRegex(RuntimeError, "connect stage"):
            driver.validate_sandbox_route_probe_observations(
                {"direct_forgejo_route_probe_errors": send_failure}
            )

        with self.assertRaisesRegex(RuntimeError, "all direct routes"):
            driver.validate_sandbox_route_probe_observations(
                {"direct_forgejo_route_probe_errors": diagnostics[:2]}
            )

        with self.assertRaises(RuntimeError):
            driver.validate_sandbox_route_probe_observations({})

    def test_gate_requires_parent_observation_of_the_live_agent_process_tree(self) -> None:
        observation = {
            "schema_version": 1,
            "source": "parent_procfs",
            "host_pid_namespace": "pid:[100]",
            "host_network_namespace": "net:[200]",
            "host_mount_namespace": "mnt:[300]",
            "host_user_namespace": "user:[400]",
            "sandbox_pid_namespace": "pid:[101]",
            "sandbox_network_namespace": "net:[201]",
            "sandbox_mount_namespace": "mnt:[301]",
            "sandbox_user_namespace": "user:[401]",
            "all_descendants_share_root_cgroup": True,
            "sandbox_descendant_count": 2,
            "agent_python_process_count": 1,
            "agent_python_open_fds": [0, 1, 2],
        }
        response = {
            "host_pid_namespace": "pid:[100]",
            "host_network_namespace": "net:[200]",
            "host_observed_agent_process_tree": observation,
        }
        sandbox_checks = {"pid_namespace": "pid:[101]", "network_namespace": "net:[201]"}

        self.assertEqual(
            driver.validate_host_observed_agent_process_tree(response, sandbox_checks),
            observation,
        )
        with self.assertRaisesRegex(RuntimeError, "parent process observation"):
            driver.validate_host_observed_agent_process_tree(
                {key: value for key, value in response.items() if key != "host_observed_agent_process_tree"},
                sandbox_checks,
            )
        stale = {**observation, "sandbox_pid_namespace": "pid:[100]"}
        with self.assertRaisesRegex(RuntimeError, "namespace observations"):
            driver.validate_host_observed_agent_process_tree(
                {**response, "host_observed_agent_process_tree": stale},
                sandbox_checks,
            )
        credential_fd = {**observation, "agent_python_open_fds": [0, 1, 2, 5]}
        with self.assertRaisesRegex(RuntimeError, "descriptor set"):
            driver.validate_host_observed_agent_process_tree(
                {**response, "host_observed_agent_process_tree": credential_fd},
                sandbox_checks,
            )

    def test_host_attestation_policy_reads_only_a_private_canonical_public_key(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            key = state / "mission-admission-attestation.public"
            key.write_text("ab" * 32, encoding="ascii")
            key.chmod(0o600)

            self.assertEqual(
                driver.trusted_webhook_attestation_public_keys(state, webhook_delivery=True),
                ["ab" * 32],
            )
            self.assertEqual(
                driver.trusted_webhook_attestation_public_keys(state, webhook_delivery=False),
                [],
            )

            key.write_text("AB" * 32, encoding="ascii")
            key.chmod(0o600)
            with self.assertRaisesRegex(RuntimeError, "canonical Ed25519 public key"):
                driver.trusted_webhook_attestation_public_keys(state, webhook_delivery=True)

            key.write_text("ab" * 32, encoding="ascii")
            key.chmod(0o644)
            with self.assertRaisesRegex(RuntimeError, "private regular file"):
                driver.trusted_webhook_attestation_public_keys(state, webhook_delivery=True)

    def test_local_adapter_failure_detail_is_bounded_and_redacted(self) -> None:
        output = json.dumps(
            {
                "type": "error",
                "error": "MCP request failed: credential=private-token " + "a" * 48,
            }
        ).encode()

        detail = driver.safe_local_adapter_failure_detail("local-test-adapter", output)

        self.assertIsNotNone(detail)
        self.assertIn("MCP request failed", detail)
        self.assertIn("credential=[redacted]", detail)
        self.assertNotIn("private-token", detail)
        self.assertNotIn("a" * 48, detail)
        self.assertLessEqual(len(detail), 512)
        self.assertIsNone(driver.safe_local_adapter_failure_detail("claude", output))

    def test_sandbox_restart_adapter_command_keeps_all_private_file_probes(self) -> None:
        probes = [
            ("admission_source", Path("/tmp/runtime/admission-source.json")),
            ("forgejo_read_token", Path("/tmp/runtime/forgejo-read.token")),
            ("host_admission", Path("/tmp/runtime/host-admission.json")),
            ("invalid_forgejo_read_token", Path("/tmp/runtime/invalid-forgejo-read.token")),
            ("mission_journal", Path("/tmp/runtime/mission-execution.jsonl")),
            ("webhook_inbox", Path("/tmp/runtime/forgejo-webhook-inbox.jsonl")),
        ]

        command = driver.build_agent_command(
            "local-test-adapter",
            Path("/tmp/runtime/mcp.json"),
            Decimal("0.20"),
            "test prompt",
            capture_test_host_stderr=True,
            sandbox_test_adapter=True,
            private_file_probes=probes,
        )

        observed: list[tuple[str, str]] = []
        index = 0
        while index < len(command):
            if command[index] == "--probe-private-file":
                observed.append((command[index + 1], command[index + 2]))
                index += 3
            else:
                index += 1
        self.assertEqual(observed, [(label, str(path)) for label, path in probes])

    def test_parent_scope_cli_refuses_before_start_without_attested_scope(self) -> None:
        with tempfile.TemporaryDirectory(prefix="bounded-cli-parent-refusal-") as temporary:
            marker = Path(temporary) / "command-started"
            with mock.patch.object(
                driver,
                "current_attested_process_scope_snapshot",
                return_value=None,
            ):
                status, stdout, stderr, timed_out, exceeded = driver.run_bounded_cli(
                    [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).touch()", str(marker)],
                    cwd=ROOT,
                    env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                    timeout=5,
                    reuse_attested_parent_scope=True,
                )

        self.assertEqual(status, 125)
        self.assertIn(b"attested parent cgroup scope", stderr)
        self.assertFalse(marker.exists())
        self.assertFalse(stdout)
        self.assertFalse(timed_out)
        self.assertFalse(exceeded)

    def test_parent_scope_cli_rejects_success_if_membership_changed(self) -> None:
        snapshot = ProcessScopeSnapshot(
            "run-mission012-stage-1-0123456789abcdef.scope",
            "/user.slice/user-1000.slice/user@1000.service/app.slice/run-mission012-stage-1-0123456789abcdef.scope",
            (os.getpid(),),
        )
        with (
            mock.patch.object(
                driver,
                "current_attested_process_scope_snapshot",
                return_value=snapshot,
            ),
            mock.patch.object(
                driver,
                "process_scope_snapshot_matches",
                return_value=False,
            ),
        ):
            status, stdout, stderr, timed_out, exceeded = driver.run_bounded_cli(
                [sys.executable, "-c", "print('adapter returned')"],
                cwd=ROOT,
                env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
                timeout=5,
                reuse_attested_parent_scope=True,
            )

        self.assertEqual(status, 125)
        self.assertIn(b"adapter returned", stdout)
        self.assertIn(b"parent cgroup membership changed", stderr)
        self.assertFalse(timed_out)
        self.assertFalse(exceeded)

    def test_bounded_cli_rejects_success_when_same_group_child_outlives_cli(self) -> None:
        with tempfile.TemporaryDirectory(prefix="bounded-cli-group-survivor-") as temporary:
            runtime = Path(temporary)
            child_pid_file = runtime / "background-child.pid"
            child = "import time\nwhile True: time.sleep(1)\n"
            parent = (
                "import pathlib, subprocess, sys\n"
                "child = subprocess.Popen([sys.executable, '-c', sys.argv[2]], "
                "stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, "
                "stderr=subprocess.DEVNULL, close_fds=True)\n"
                "pathlib.Path(sys.argv[1]).write_text(str(child.pid))\n"
                "print('CLI returned')\n"
            )

            status, stdout, stderr, timed_out, exceeded = driver._run_bounded_cli_process_group(
                [sys.executable, "-c", parent, str(child_pid_file), child],
                cwd=ROOT,
                env=os.environ.copy(),
                timeout=5,
            )
            child_pid = int(child_pid_file.read_text())
            try:
                deadline = time.monotonic() + 2
                running = True
                while time.monotonic() < deadline:
                    try:
                        stat_line = Path(f"/proc/{child_pid}/stat").read_text()
                    except FileNotFoundError:
                        running = False
                        break
                    state = stat_line.rsplit(")", 1)[1].split()[0]
                    running = state not in {"Z", "X"}
                    if not running:
                        break
                    time.sleep(0.02)

                self.assertEqual(status, 125, (stdout, stderr))
                self.assertFalse(running, "bounded CLI returned while a same-group child remained active")
                self.assertFalse(timed_out)
                self.assertFalse(exceeded)
            finally:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_test_adapter_work_item_observation_must_be_explicit_and_match_admission(self) -> None:
        self.assertIs(
            driver.validate_test_adapter_work_item_observation(
                {"test_adapter_untrusted_work_item_observed": True},
                expected=True,
            ),
            True,
        )
        self.assertIs(
            driver.validate_test_adapter_work_item_observation(
                {"test_adapter_untrusted_work_item_observed": False},
                expected=False,
            ),
            False,
        )

        for response, expected in (
            ({}, True),
            ({"test_adapter_untrusted_work_item_observed": False}, True),
            ({"test_adapter_untrusted_work_item_observed": True}, False),
            ({"test_adapter_untrusted_work_item_observed": 1}, True),
        ):
            with self.subTest(response=response, expected=expected):
                with self.assertRaisesRegex(RuntimeError, "does not match the admitted issue context"):
                    driver.validate_test_adapter_work_item_observation(
                        response,
                        expected=expected,
                    )

    def test_local_adapter_command_never_resolves_claude(self) -> None:
        config = Path("/tmp/private-mcp-config.json")
        with mock.patch.object(shutil, "which", side_effect=AssertionError("Claude lookup attempted")):
            command = driver.build_agent_command(
                "local-test-adapter", config, Decimal("0.20"), "unused prompt"
            )

        self.assertEqual(command[0], sys.executable)
        self.assertTrue(command[1].endswith("/scripts/mission_012_test_agent_adapter.py"))
        self.assertEqual(command[2:], ["--mcp-config", str(config)])

    def test_restart_fixture_can_capture_only_its_local_host_diagnostics(self) -> None:
        command = driver.build_agent_command(
            "local-test-adapter",
            Path("/tmp/private-mcp-config.json"),
            Decimal("0.20"),
            "unused prompt",
            capture_test_host_stderr=True,
        )

        self.assertEqual(command[-1], "--capture-host-stderr")
        self.assertNotIn("--capture-host-stderr", driver.build_agent_command(
            "local-test-adapter",
            Path("/tmp/private-mcp-config.json"),
            Decimal("0.20"),
            "unused prompt",
        ))

    def test_explicit_sandbox_option_routes_only_the_test_adapter_through_bwrap(self) -> None:
        config = Path("/tmp/private-mcp-config.json")
        command = driver.build_agent_command(
            "local-test-adapter",
            config,
            Decimal("0.20"),
            "unused prompt",
            sandbox_test_adapter=True,
        )

        self.assertEqual(command[0], sys.executable)
        self.assertTrue(command[1].endswith("/scripts/mission_012_sandboxed_test_adapter.py"))
        self.assertEqual(
            command[2:],
            [
                "--mcp-config",
                str(config),
                "--forgejo-probe-port",
                "3132",
                "--probe-owner",
                "forge-admin",
                "--probe-repository",
                "mission-012-clamp",
                "--probe-pull-number",
                "2",
            ],
        )

    def test_webhook_files_are_passed_only_as_bounded_sandbox_visibility_probes(self) -> None:
        config = Path("/tmp/private-mcp-config.json")
        runtime = Path("/tmp/private-project/.runtime")
        probes = [
            ("admission_source", runtime / ("mission-host-admission-source-" + "a" * 64 + ".json")),
            ("forgejo_read_token", runtime / "forgejo-read.token"),
            ("host_admission", runtime / "host-admission.json"),
            ("invalid_forgejo_read_token", runtime / "invalid-forgejo-read.token"),
            ("mission_journal", runtime / "mission-execution.jsonl"),
            ("webhook_inbox", runtime / "forgejo-webhook-inbox.jsonl"),
        ]
        command = driver.build_agent_command(
            "local-test-adapter",
            config,
            Decimal("0.20"),
            "unused prompt",
            sandbox_test_adapter=True,
            private_file_probes=probes,
        )

        probe_args = [
            command[index + 1:index + 3]
            for index, value in enumerate(command)
            if value == "--probe-private-file"
        ]
        self.assertEqual(probe_args, [[label, str(path)] for label, path in probes])

    def test_post_acceptance_barrier_paths_are_routed_only_to_the_sandbox_wrapper(self) -> None:
        config = Path("/tmp/private-mcp-config.json")
        runtime = Path("/tmp/private-project/.runtime")
        command = driver.build_agent_command(
            "local-test-adapter",
            config,
            Decimal("0.20"),
            "unused prompt",
            sandbox_test_adapter=True,
            wait_at_gate_barrier=True,
            gate_barrier_ready_file=runtime / "mcp-gate-ready.json",
            gate_barrier_release_file=runtime / "mcp-gate-release.json",
        )

        self.assertIn("--wait-at-gate-barrier", command)
        ready_index = command.index("--gate-barrier-ready-file")
        release_index = command.index("--gate-barrier-release-file")
        self.assertEqual(command[ready_index + 1], str(runtime / "mcp-gate-ready.json"))
        self.assertEqual(command[release_index + 1], str(runtime / "mcp-gate-release.json"))

    def test_gate_barrier_is_refused_for_non_sandboxed_or_model_agents(self) -> None:
        runtime = Path("/tmp/private-project/.runtime")
        with self.assertRaisesRegex(RuntimeError, "only available for the sandboxed local-test-adapter"):
            driver.build_agent_command(
                "local-test-adapter",
                Path("/tmp/private-mcp-config.json"),
                Decimal("0.20"),
                "unused prompt",
                gate_barrier_ready_file=runtime / "mcp-gate-ready.json",
                gate_barrier_release_file=runtime / "mcp-gate-release.json",
            )

    def test_sandbox_option_is_rejected_for_model_adapter(self) -> None:
        with mock.patch.object(shutil, "which", return_value="/usr/bin/claude"):
            with self.assertRaisesRegex(RuntimeError, "only available for local-test-adapter"):
                driver.build_agent_command(
                    "claude",
                    Path("/tmp/private-mcp-config.json"),
                    Decimal("0.20"),
                    "unused prompt",
                    sandbox_test_adapter=True,
                )

    def test_claude_command_refuses_unisolated_provider_before_cli_lookup(self) -> None:
        with mock.patch.object(
            shutil,
            "which",
            side_effect=AssertionError("the provider executable must not be resolved"),
        ):
            with self.assertRaisesRegex(RuntimeError, "provider egress and filesystem isolation are not attested"):
                driver.build_agent_command(
                    "claude", Path("/tmp/private-mcp-config.json"), Decimal("0.15"), "work"
                )

    def test_model_run_refuses_before_resolving_state_or_credentials(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "provider egress and filesystem isolation are not attested"):
            driver.run(mock.Mock(agent_adapter="claude", state_dir="/definitely/missing"))


if __name__ == "__main__":
    unittest.main()
