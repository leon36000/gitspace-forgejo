import argparse
import hashlib
import json
import sys
import tempfile
import unittest
import uuid
import zipfile
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from composition_source_snapshot import (  # noqa: E402
    SourceSnapshotError,
    capture_sources,
    copy_snapshot,
    discover_source_paths,
    verify_sources_unchanged,
)
import composition_source_snapshot as snapshot_module  # noqa: E402
import run_mission_012_live_composed_e2e as composed_runner  # noqa: E402
import run_forgejo_live_webhook_mission_admission as live_admission  # noqa: E402
from run_mission_012_live_composed_e2e import (  # noqa: E402
    EXPECTED_STAGES,
    execute_stages,
)


class CompositionSourceSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="composition-source-snapshot-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "repo"
        for directory in ("src", "scripts", "tests/fixtures", "tests/__pycache__"):
            (self.root / directory).mkdir(parents=True, exist_ok=True)
        (self.root / "Cargo.toml").write_text("[package]\nname='fixture'\n")
        (self.root / "Cargo.lock").write_text("version = 4\n")
        (self.root / "src/lib.rs").write_text("pub fn answer() -> u8 { 42 }\n")
        (self.root / "scripts/runner.py").write_text("print('fixture')\n")
        (self.root / "tests/fixtures/requirements.json").write_text('{"schema_version":1}\n')
        (self.root / "tests/__pycache__/ignored.py").write_text("must not be captured\n")

    def test_capture_is_readable_and_verifies_the_exact_source_list(self):
        paths = discover_source_paths(self.root)
        archive_path = self.root / "snapshot.zip"
        snapshot = capture_sources(self.root, archive_path, paths)

        self.assertEqual(
            set(snapshot["source_sha256"]),
            {"Cargo.lock", "Cargo.toml", "scripts/runner.py", "src/lib.rs", "tests/fixtures/requirements.json"},
        )
        verify_sources_unchanged(self.root, snapshot["source_sha256"])
        with zipfile.ZipFile(archive_path) as archive:
            manifest = json.loads(archive.read("source-manifest.json"))
            self.assertEqual(manifest["schema_version"], 1)
            self.assertEqual(
                archive.read("sources/src/lib.rs"),
                b"pub fn answer() -> u8 { 42 }\n",
            )
            self.assertNotIn("sources/tests/__pycache__/ignored.py", archive.namelist())

    def test_workspace_drift_is_rejected_after_capture(self):
        paths = discover_source_paths(self.root)
        snapshot = capture_sources(self.root, self.root / "snapshot.zip", paths)
        (self.root / "src/lib.rs").write_text("pub fn answer() -> u8 { 0 }\n")

        with self.assertRaisesRegex(SourceSnapshotError, "source changed after snapshot"):
            verify_sources_unchanged(self.root, snapshot["source_sha256"])

    def test_added_source_file_is_rejected_after_capture(self):
        snapshot = capture_sources(self.root, self.root / "snapshot.zip", discover_source_paths(self.root))
        (self.root / "src/new_module.rs").write_text("pub fn added() {}\n")

        with self.assertRaisesRegex(SourceSnapshotError, "source file set changed after snapshot"):
            verify_sources_unchanged(self.root, snapshot["source_sha256"])

    def test_source_symlink_is_refused(self):
        external = self.root.parent / "outside.py"
        external.write_text("do not read through a source symlink\n")
        (self.root / "scripts/external.py").symlink_to(external)

        with self.assertRaisesRegex(SourceSnapshotError, "symbolic link"):
            discover_source_paths(self.root)

    def test_symlinked_cargo_config_parent_is_refused(self):
        external = self.root.parent / "external-cargo-config"
        external.mkdir()
        (external / "config.toml").write_text("[build]\njobs=1\n")
        (self.root / ".cargo").symlink_to(external, target_is_directory=True)

        with self.assertRaisesRegex(SourceSnapshotError, "symbolic link"):
            discover_source_paths(self.root)

    def test_existing_snapshot_destination_is_not_overwritten(self):
        destination = self.root / "snapshot.zip"
        destination.write_bytes(b"keep existing file")

        with self.assertRaisesRegex(SourceSnapshotError, "already exists"):
            capture_sources(self.root, destination, discover_source_paths(self.root))
        self.assertEqual(destination.read_bytes(), b"keep existing file")

    def test_snapshot_copy_is_exclusive_and_hash_checked(self):
        snapshot_path = self.root / "snapshot.zip"
        snapshot = capture_sources(self.root, snapshot_path, discover_source_paths(self.root))
        evidence_copy = self.root / "evidence-copy.zip"

        copied_hash = copy_snapshot(snapshot_path, evidence_copy, snapshot["archive_sha256"])

        self.assertEqual(copied_hash, snapshot["archive_sha256"])
        self.assertEqual(evidence_copy.read_bytes(), snapshot_path.read_bytes())
        with self.assertRaisesRegex(SourceSnapshotError, "already exists"):
            copy_snapshot(snapshot_path, evidence_copy, snapshot["archive_sha256"])

    def test_noncanonical_source_path_is_rejected(self):
        outside = self.root.parent / "outside.py"
        outside.write_text("must not be captured\n")

        with self.assertRaisesRegex(SourceSnapshotError, "invalid source path"):
            capture_sources(self.root, self.root / "snapshot.zip", ["../outside.py"])

    def test_source_and_archive_byte_limits_are_checked_before_unbounded_reads(self):
        with patch.object(snapshot_module, "MAX_SOURCE_SNAPSHOT_BYTES", 1):
            with self.assertRaisesRegex(SourceSnapshotError, "source bytes exceed"):
                capture_sources(self.root, self.root / "oversized.zip", ["src/lib.rs"])
        self.assertFalse((self.root / "oversized.zip").exists())

        archive = self.root / "archive.zip"
        archive.write_bytes(b"0123456789")
        with patch.object(snapshot_module, "MAX_SOURCE_SNAPSHOT_ARCHIVE_BYTES", 4):
            with self.assertRaisesRegex(SourceSnapshotError, "archive exceeds"):
                copy_snapshot(archive, self.root / "copy.zip", hashlib.sha256(archive.read_bytes()).hexdigest())
        self.assertFalse((self.root / "copy.zip").exists())

    def test_composed_runner_stops_when_a_stage_changes_source_inputs(self):
        source_file = self.root / "src/lib.rs"
        snapshot = capture_sources(self.root, self.root / "snapshot.zip", discover_source_paths(self.root))

        def mutate_source(_command, _timeout):
            source_file.write_text("pub fn answer() -> u8 { 0 }\n")
            return 1, b"stage failed"

        with self.assertRaisesRegex(RuntimeError, "source changed during admission"):
            execute_stages(
                [(name, ["fixture"], 1) for name in EXPECTED_STAGES],
                self.root / "pipeline.log",
                "0.10",
                expected_allocation_id="mission012-fixture",
                stage_runner=mutate_source,
                admission_evidence_dir=self.root / "admission",
                agent_adapter="local-test-adapter",
                source_root=self.root,
                source_sha256=snapshot["source_sha256"],
            )

    def test_composed_runner_preserves_only_a_redacted_stage_failure_diagnostic(self):
        output = b"private traceback frame\nRuntimeError: admission token=synthetic-private-token\n"

        with self.assertRaises(RuntimeError) as raised:
            execute_stages(
                [(name, ["fixture"], 1) for name in EXPECTED_STAGES],
                self.root / "failure-pipeline.log",
                "0.10",
                expected_allocation_id="mission012-fixture",
                stage_runner=lambda _command, _timeout: (1, output),
                admission_evidence_dir=self.root / "admission",
                agent_adapter="local-test-adapter",
            )

        message = str(raised.exception)
        self.assertIn("admission stage failed with exit code 1", message)
        self.assertIn("stage_output_sha256=" + hashlib.sha256(output).hexdigest(), message)
        self.assertIn("RuntimeError: admission token=[redacted]", message)
        self.assertNotIn("synthetic-private-token", message)

    def test_source_head_git_overrides_are_scoped_to_one_git_process(self):
        with patch.dict(
            "os.environ",
            {"GIT_DIR": "/host/authority.git", "GIT_WORK_TREE": "/host/private-tree"},
        ), patch.object(
            composed_runner.subprocess,
            "run",
            side_effect=[SimpleNamespace(stdout=b"a" * 40 + b"\n")] * 2,
        ) as git_run:
            head = composed_runner.read_source_head(
                Path("/run/worktree"),
                git_dir="/run/source-git/worktrees/test",
                git_work_tree="/run/worktree",
            )
            composed_runner.read_source_head(Path("/ordinary/repo"))

        self.assertEqual(head, "a" * 40)
        scoped_env = git_run.call_args_list[0].kwargs["env"]
        self.assertEqual(scoped_env["GIT_DIR"], "/run/source-git/worktrees/test")
        self.assertEqual(scoped_env["GIT_WORK_TREE"], "/run/worktree")
        self.assertEqual(scoped_env["GIT_OPTIONAL_LOCKS"], "0")
        ordinary_env = git_run.call_args_list[1].kwargs["env"]
        self.assertNotIn("GIT_DIR", ordinary_env)
        self.assertNotIn("GIT_WORK_TREE", ordinary_env)

    def test_admission_source_provenance_scopes_git_overrides_to_read_commands(self):
        with tempfile.TemporaryDirectory() as temp:
            git_dir = Path(temp) / "git-dir"
            git_dir.mkdir()
            with patch.dict(
                "os.environ",
                {"GIT_DIR": "/host/authority.git", "GIT_WORK_TREE": "/host/private-tree"},
            ), patch.object(
                live_admission.subprocess,
                "run",
                side_effect=[SimpleNamespace(stdout=""), SimpleNamespace(stdout="b" * 40)],
            ) as git_run, patch.object(
                live_admission.webhook_helpers,
                "sha256_file",
                return_value="c" * 64,
            ):
                provenance = live_admission._source_provenance(
                    ROOT,
                    git_dir=str(git_dir),
                    git_work_tree=str(ROOT),
                )

        self.assertEqual(provenance["head_commit"], "b" * 40)
        self.assertEqual(len(git_run.call_args_list), 2)
        for call in git_run.call_args_list:
            environment = call.kwargs["env"]
            self.assertEqual(environment["GIT_DIR"], str(git_dir))
            self.assertEqual(environment["GIT_WORK_TREE"], str(ROOT))
            self.assertEqual(environment["GIT_OPTIONAL_LOCKS"], "0")

    def test_composed_stage_path_includes_the_read_only_configured_rust_toolchain(self):
        environment = composed_runner.stage_environment()
        self.assertIn(
            str(composed_runner.pr_driver.RUST_TOOLCHAIN / "bin"),
            environment["PATH"].split(composed_runner.os.pathsep),
        )

    def test_composed_runner_persists_source_archive_in_final_receipt_and_manifest(self):
        (self.root / "evidence").mkdir()
        (self.root / "src/main.rs").write_text("pub fn answer() -> u8 { 42 }\n")
        binary = self.root / "forgejo"
        binary.write_bytes(b"isolated fixture binary")
        binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()

        state_dir = Path("/tmp") / f"composition-receipt-state-{uuid.uuid4().hex}"
        source_archive = state_dir.parent / f".{state_dir.name}.composition-source-snapshot.zip"
        pipeline_log = state_dir.parent / f".{state_dir.name}.pipeline.log"
        self.addCleanup(lambda: source_archive.unlink(missing_ok=True))
        self.addCleanup(lambda: pipeline_log.unlink(missing_ok=True))
        evidence_dir = self.root / "evidence" / "composed-run"
        args = argparse.Namespace(
            state_dir=str(state_dir),
            forgejo_binary=str(binary),
            evidence_dir=str(evidence_dir),
            execution_id="fixture-execution",
            allocation_id="mission012-fixture",
            budget_cap_usd="0.10",
            agent_adapter="local-test-adapter",
            inject_concurrent_base_advance=False,
            expected_host_network_namespace="net:[999999999]",
        )

        def fake_execute_stages(*_args, **_kwargs):
            evidence_dir.mkdir()
            (evidence_dir / "forgejo-result.json").write_text('{"readback":"current"}\n')
            return {
                "admission": {},
                "agent": {},
                "pull-request": {},
                "webhook_admission_handoff": {
                    "delivery_id": "fixture-delivery",
                    "admission_source_sha256": "a" * 64,
                    "webhook_inbox_sha256": "b" * 64,
                },
            }

        completed_git = composed_runner.subprocess.CompletedProcess(
            args=["git", "rev-parse", "HEAD"],
            returncode=0,
            stdout=b"a" * 40,
            stderr=b"",
        )
        with (
            patch.object(composed_runner, "ROOT", self.root),
            patch.object(composed_runner, "FORGEJO_SHA256", binary_sha256),
            patch.object(composed_runner.subprocess, "run", return_value=completed_git),
            patch.object(composed_runner, "build_stage_commands", return_value=[]),
            patch.object(composed_runner, "execute_stages", side_effect=fake_execute_stages),
            patch.object(
                composed_runner,
                "validate_admission_build_provenance",
                return_value={"manifest_sha256": "a" * 64, "mcp_host_binary_sha256": "b" * 64},
            ),
            patch.object(composed_runner, "validate_joined_results", return_value={"verdict": "merged"}),
            patch.object(composed_runner, "validate_postmerge_readback"),
            patch.object(composed_runner, "_admission_summary", return_value={"status": "admitted"}),
            patch.object(composed_runner, "_agent_summary", return_value={"status": "completed"}),
            patch.object(composed_runner, "_pull_request_summary", return_value={"status": "merged"}),
        ):
            result = composed_runner.run(args)

        receipt = json.loads((evidence_dir / "composed-run.json").read_text())
        snapshot_info = receipt["composition_source_snapshot"]
        archive_path = evidence_dir / snapshot_info["archive"]
        archive_bytes = archive_path.read_bytes()
        self.assertEqual(result, receipt)
        self.assertEqual(snapshot_info["schema_version"], 1)
        self.assertEqual(snapshot_info["archive_sha256"], hashlib.sha256(archive_bytes).hexdigest())
        self.assertEqual(receipt["source_head"], "a" * 40)
        with zipfile.ZipFile(archive_path) as archive:
            source_manifest = json.loads(archive.read("source-manifest.json"))
        self.assertEqual(
            receipt["composition_source_sha256"],
            {item["path"]: item["sha256"] for item in source_manifest["files"]},
        )
        self.assertEqual(snapshot_info["file_count"], source_manifest["file_count"])

        checksums = {}
        for line in (evidence_dir / "SHA256SUMS").read_text().splitlines():
            digest, name = line.split("  ", 1)
            checksums[name] = digest
        self.assertEqual(checksums[archive_path.name], snapshot_info["archive_sha256"])
        self.assertEqual(checksums["composed-run.json"], hashlib.sha256((evidence_dir / "composed-run.json").read_bytes()).hexdigest())
        self.assertEqual(checksums["forgejo-result.json"], receipt["readback_sha256"])


if __name__ == "__main__":
    unittest.main()
