from __future__ import annotations

import importlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
driver = importlib.import_module("run_forgejo_private_netns_webhook_e2e")


class PrivateNetworkObservationTests(unittest.TestCase):
    def test_runner_identity_ignores_bubblewrap_argv_and_matches_python_script(self) -> None:
        script = "/run/workspace/scripts/run_forgejo_live_webhook_reconcile_emitted.py"
        self.assertFalse(driver.is_emitted_runner("/usr/bin/bwrap", ["bwrap", script]))
        self.assertTrue(driver.is_emitted_runner("/usr/bin/python3.12", ["/usr/bin/python3", script]))
        self.assertFalse(driver.is_emitted_runner("/usr/bin/python3.12", ["python3", "other.py", script]))

    def test_reads_pretty_printed_runner_verdict_from_its_safe_result_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            (evidence / "result.json").write_text(
                '{\n  "schema_version": 1,\n  "verdict": "passed"\n}\n',
                encoding="utf-8",
            )
            self.assertEqual(driver.read_inner_result(evidence), {"schema_version": 1, "verdict": "passed"})

    def test_reassesses_a_false_negative_only_when_live_namespace_and_inner_result_agree(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            observation = {
                "host_network_namespace_inode": 100,
                "inner_runner_exit_code": 0,
                "inner_runner_timed_out": False,
                "captured_output_exceeded": False,
                "inner_failure_stage": None,
                "run_status": "failed",
                "private_service_observation": {
                    "host_network_namespace_inode": 100,
                    "runner_network_namespace_inode": 200,
                    "forgejo_network_namespace_inode": 200,
                    "host_loopback_probe_errno": 111,
                    "host_setns_probe": {"status": "denied_operation_not_permitted"},
                    "observed_while_forgejo_process_alive": True,
                },
            }
            (evidence / "private-netns-observation.json").write_text(
                json.dumps(observation), encoding="utf-8"
            )
            (evidence / "result.json").write_text(
                '{"verdict":"passed","forgejo":{"version":"fixture"}}',
                encoding="utf-8",
            )
            assessment = driver.reassess_saved_run(evidence)
            self.assertEqual(assessment["verdict"], "passed")
            self.assertEqual(assessment["prior_outer_run_status"], "failed")
            self.assertEqual(assessment["network_namespace_inodes"]["runner"], 200)
            self.assertEqual(assessment["inner_verdict"], "passed")
            observation["private_service_observation"]["host_loopback_probe_errno"] = 0
            (evidence / "private-netns-observation.json").write_text(
                json.dumps(observation), encoding="utf-8"
            )
            refused_assessment = driver.reassess_saved_run(evidence)
            self.assertEqual(refused_assessment["verdict"], "inconclusive")
            self.assertFalse(refused_assessment["checks"]["host_loopback_connection_refused"])
            self.assertFalse(
                all(driver.private_run_checks(observation, {"verdict": "passed"}).values())
            )

    def test_parses_only_linux_network_namespace_links(self) -> None:
        self.assertEqual(driver.parse_network_namespace_link("net:[4026535807]"), 4026535807)
        for value in ("", "net:4026535807", "pid:[4026535807]", "net:[0]", "net:[x]"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                driver.parse_network_namespace_link(value)

    def test_finds_only_loopback_listeners_for_the_requested_port(self) -> None:
        rows = """  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0C3A 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 12345
   1: 0100007F:0C3A 0100007F:ABCD 01 00000000:00000000 00:00000000 00000000 1000 0 12346
   2: 00000000:0C3A 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 12347
   3: 0100007F:0C3B 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 12348
"""
        self.assertEqual(driver.proc_tcp_listener_inodes(rows, 3130), {12345})

    def test_mounts_writable_opt_before_creating_read_only_toolchain_mountpoints(self) -> None:
        args = driver._workspace_mounts(driver.ROOT, Path("/tmp/forgejo-17.0.0"))
        opt = args.index("/opt")
        self.assertEqual(args[opt - 1], "--tmpfs")
        self.assertLess(opt, args.index("/opt/cargo"))
        self.assertNotIn(["--dir", "/workspace"], [args[i : i + 2] for i in range(len(args) - 1)])
        run_tmpfs = args.index("/run")
        self.assertEqual(args[run_tmpfs - 1], "--tmpfs")
        workspace = args.index("/run/workspace")
        self.assertEqual(args[workspace - 1], "--dir")
        self.assertLess(run_tmpfs, workspace)

    def test_mounts_only_git_metadata_needed_by_read_only_source_tree(self) -> None:
        args = driver._workspace_mounts(driver.ROOT, Path("/tmp/forgejo-17.0.0"))
        expected_common_dir = str(driver.ROOT.parents[1] / ".git")
        self.assertIn(expected_common_dir, args)
        self.assertIn("/run/source-git", args)
        self.assertIn("/run/source-git/worktrees/webhook-005-workspace", args)
        self.assertIn("GIT_WORK_TREE", args)
        self.assertIn("/run/workspace", args)
        self.assertNotIn("/home/pc6", args)

    def test_creates_a_sha256sum_compatible_manifest_when_inner_run_has_no_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            artifact = evidence / "private-netns-observation.json"
            artifact.write_text('{"run_status":"passed"}\n', encoding="utf-8")
            driver.create_checksum_manifest(evidence, artifact.name)
            manifest = evidence / "SHA256SUMS"
            self.assertEqual(
                manifest.read_text(encoding="ascii"),
                f"{driver.hash_file(artifact)}  {artifact.name}\n",
            )
            subprocess_result = subprocess.run(
                ["sha256sum", "-c", str(manifest)],
                cwd=evidence,
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(subprocess_result.returncode, 0, subprocess_result.stderr)


if __name__ == "__main__":
    unittest.main()
