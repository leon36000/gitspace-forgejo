#!/usr/bin/env python3
"""Build and run read-only preflight CLI checks in an allowlisted sandbox."""

from __future__ import annotations

import argparse
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile

import run_forgejo_mission_delivery_process_e2e as sandbox


REPO = sandbox.REPO
TEST_TARGET = "forgejo_preflight_process_e2e"
BIN_TARGETS = ("forgejo-reconcile-local",)
TESTS = (
    (
        "actual_preflight_cli_accepts_target_base_with_fork_head_without_writes",
        True,
    ),
    (
        "actual_preflight_cli_refuses_foreign_base_repository_without_writes",
        True,
    ),
    ("actual_preflight_cli_refuses_draft_pull_request_without_writes", True),
    (
        "actual_preflight_cli_refuses_stale_head_base_requirements_and_policy_without_writes",
        True,
    ),
    (
        "actual_preflight_cli_reports_anonymous_401_without_claiming_a_credential_was_sent",
        True,
    ),
    ("request_header_reader_preserves_a_pipelined_follow_up_request", False),
)


def isolated_test_command(
    bwrap: Path,
    source_tree: Path,
    test_binary: Path,
    binaries: dict[str, Path],
    test_name: str,
    ignored: bool,
    rust_sysroot: Path,
    masked_home: Path,
) -> list[str]:
    command = sandbox.sandbox_prefix(
        bwrap,
        source_tree,
        rust_sysroot,
        masked_home,
        None,
    )
    command.extend(["--dir", "/build/target", "--dir", "/build/target/debug"])
    for binary_name in BIN_TARGETS:
        command.extend(
            [
                "--ro-bind",
                str(binaries[binary_name]),
                f"/build/target/debug/{binary_name}",
            ]
        )
    command.extend(
        [
            "--ro-bind",
            str(test_binary.resolve()),
            "/test-binary",
            "/usr/bin/prlimit",
            "--cpu=240",
            "--as=4294967296",
            "--nproc=384",
            "--nofile=128",
            "--fsize=67108864",
            "--",
            "/test-binary",
            test_name,
            "--exact",
        ]
    )
    if ignored:
        command.append("--ignored")
    command.append("--nocapture")
    return command


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", choices=[name for name, _ in TESTS])
    args = parser.parse_args()

    rustc = shutil.which("rustc")
    bwrap = Path("/usr/bin/bwrap")
    if rustc is None:
        raise SystemExit("rustc not found in PATH")
    if not bwrap.is_file():
        raise SystemExit("/usr/bin/bwrap is required to isolate the loopback Forgejo fixture")

    rust_sysroot = Path(
        subprocess.run([rustc, "--print", "sysroot"], check=True, capture_output=True, text=True)
        .stdout.strip()
    ).resolve()
    if not rust_sysroot.is_dir():
        raise SystemExit(f"rustc returned a non-directory sysroot: {rust_sysroot}")

    masked_home = Path.home().resolve()
    registry_source = masked_home / ".cargo" / "registry"
    if not registry_source.is_dir():
        raise SystemExit(f"offline Cargo registry cache is unavailable: {registry_source}")

    selected_tests = tuple(
        (name, ignored)
        for name, ignored in TESTS
        if args.scenario is None or args.scenario == name
    )
    with tempfile.TemporaryDirectory(prefix="forgejo-preflight-cli-e2e-", dir="/tmp") as temp:
        temp_root = Path(temp).resolve()
        source_tree = temp_root / "source"
        sandbox.copy_source_tree(source_tree)
        registry_cache = temp_root / "cargo-registry"
        sandbox.copy_locked_registry(registry_source, registry_cache)

        print(f"evidence.git_head={subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=REPO, check=True, capture_output=True, text=True).stdout.strip()}", flush=True)
        print(f"evidence.source_tree_sha256={sandbox.sha256_tree(source_tree)}", flush=True)
        print(f"evidence.cargo_lock_sha256={sandbox.sha256_file(source_tree / 'Cargo.lock')}", flush=True)
        print(f"evidence.runner_sha256={sandbox.sha256_file(Path(__file__).resolve())}", flush=True)
        print(f"evidence.registry_stage_sha256={sandbox.sha256_tree(registry_cache)}", flush=True)
        print(f"evidence.rustc={subprocess.run([rustc, '--version'], check=True, capture_output=True, text=True).stdout.strip()}", flush=True)
        print(f"evidence.cargo={subprocess.run(['cargo', '--version'], check=True, capture_output=True, text=True).stdout.strip()}", flush=True)
        print(f"evidence.bwrap={subprocess.run([str(bwrap), '--version'], check=True, capture_output=True, text=True).stdout.strip()}", flush=True)
        print(f"evidence.platform={platform.platform()}", flush=True)

        artifact_names = (TEST_TARGET, *BIN_TARGETS)
        artifacts: dict[str, Path] = {}
        for name in artifact_names:
            artifact = temp_root / name
            artifact.touch(mode=0o700)
            artifact.chmod(0o700)
            artifacts[name] = artifact

        binaries = sandbox.compile_targets(
            bwrap,
            (TEST_TARGET,),
            BIN_TARGETS,
            artifacts,
            source_tree,
            rust_sysroot,
            masked_home,
            registry_cache,
        )

        for test_name, ignored in selected_tests:
            command = isolated_test_command(
                bwrap,
                source_tree,
                binaries[TEST_TARGET],
                binaries,
                test_name,
                ignored,
                rust_sysroot,
                masked_home,
            )
            sandbox.run(command, cwd=REPO, test_name=test_name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
