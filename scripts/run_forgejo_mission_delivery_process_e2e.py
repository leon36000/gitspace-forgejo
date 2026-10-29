#!/usr/bin/env python3
"""Build the mission process checks, then run each in isolated bwrap namespaces."""

from __future__ import annotations

import json
import hashlib
import argparse
import os
import platform
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import tomllib


REPO = Path(__file__).resolve().parents[1]
SANDBOX_REPO = Path("/workspace")
CARGO_REGISTRY_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
SOURCE_STAGE_LIMIT = 256 * 1024 * 1024
SOURCE_STAGE_FILE_LIMIT = 50000
REGISTRY_STAGE_LIMIT = 128 * 1024 * 1024
ARTIFACT_STAGE_LIMIT = 128 * 1024 * 1024
TEST_OUTPUT_LIMIT = 512 * 1024
BIN_TARGETS = (
    "forgejo-mission-local",
    "forgejo-reconcile-local",
    "forgejo-webhook-local",
    "mission-012-mcp-host",
    "mission-012-verify-local",
)
TESTS = (
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_rejects_stale_context_before_allocation_then_claims_after_base_advance",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_refuses_issue_snapshot_change_during_admission_then_retries_current_state",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_refuses_mismatched_issue_repository_before_allocation_then_retries_corrected",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_refuses_branch_head_change_during_admission_then_retries_current_state",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_concurrent_duplicate_deliveries_claim_one_generation_once",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_uses_private_host_credential_only_for_fixed_forgejo_reads",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_admits_signed_webhook_with_v3_requirement_arbitration",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_refuses_mismatched_source_pull_request_repository_before_allocation_then_retries",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_mission_cli_refuses_same_revision_requirement_edit_after_signed_delivery",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "mission_profile_rejects_caller_digest_before_inbox_credential_or_allocation",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "mission_profile_refuses_unpinned_current_requirements_before_allocation",
    ),
    (
        "forgejo_mission_delivery_process",
        "launched_cli_refuses_an_unknown_delivery_without_creating_runtime_state",
    ),
    (
        "forgejo_mission_delivery_process",
        "launched_cli_requires_attestation_policy_before_reading_host_credential",
    ),
    (
        "forgejo_mission_delivery_process",
        "launched_cli_rejects_untrusted_attestation_before_reading_host_credential",
    ),
    (
        "forgejo_mission_delivery_process",
        "launched_cli_rejects_semantically_valid_rehashed_tampering_before_credential",
    ),
    (
        "forgejo_mission_delivery_process",
        "launched_cli_reports_torn_attested_journal_before_credential_without_repair",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "verifier_process_refuses_caller_supplied_requirement_and_command",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "mission_012_signed_request_context_red_green_independent_verification_and_sandbox_merge",
    ),
    (
        "forgejo_mission_delivery_process_e2e",
        "actual_cli_current_issue_labels_reach_mcp_context_from_signed_webhook",
    ),
    (
        "mission_012_mcp_lifecycle",
        "webhook_source_handoff_process_rejects_missing_and_stale_receipts_before_mcp",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "killed_cli_after_forgejo_merge_recovers_readback_only_and_serializes_duplicate_retries",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "merged_readback_with_contradictory_base_owner_is_inconclusive",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "merged_readback_with_mismatched_pr_base_repository_id_is_inconclusive",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "merged_readback_with_contradictory_issue_repository_is_inconclusive",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "merged_readback_with_mismatched_issue_repository_id_is_inconclusive",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "signed_webhook_reconciliation_preserves_the_merge_cli_readback_sha",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "corrected_issue_action_retries_same_delivery_then_reconciles_through_real_processes",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "inconsistent_review_totals_stop_merge_authority_process_before_post",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "unauthorized_second_review_page_stops_merge_before_authority",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "draft_pull_request_refuses_merge_authority_before_review_or_post",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "killed_cli_after_intent_before_post_keeps_open_pr_unresolved_without_posting",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "protected_host_input_change_after_final_forgejo_read_refuses_merge_post",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "unresolved_merge_on_same_base_refuses_another_pr_before_forgejo_reads",
    ),
    (
        "forgejo_merge_post_crash_recovery_process_e2e",
        "external_base_advance_during_merge_post_is_fail_closed_but_still_mutates_branch",
    ),
    (
        "webhook_receiver_process_e2e",
        "actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details",
    ),
    (
        "webhook_receiver_process_e2e",
        "actual_reconcile_cli_process_refuses_parent_run_identity_mismatch_after_valid_job_readback",
    ),
)


def require_one_passing_test(output: bytes, test_name: str) -> None:
    summaries = [
        line.strip()
        for line in output.splitlines()
        if line.strip().startswith(b"test result: ")
    ]
    expected = b"test result: ok. 1 passed; 0 failed; 0 ignored;"
    test_prefix = f"test {test_name} ...".encode()
    lines = output.splitlines()
    executed_test_passed = any(
        line.startswith(test_prefix)
        and (
            line.rstrip().endswith(b" ok")
            or (index + 1 < len(lines) and lines[index + 1].strip() == b"ok")
        )
        for index, line in enumerate(lines)
    )
    if (
        len(summaries) != 1
        or not summaries[0].startswith(expected)
        or not executed_test_passed
    ):
        raise SystemExit(
            f"isolated test {test_name!r} did not execute exactly one passing test: "
            f"summaries={summaries!r}; test_passed={executed_test_passed}"
        )


def required_binary_targets(target_names: tuple[str, ...]) -> tuple[str, ...]:
    """Return only application binaries referenced by the selected test crates."""
    referenced: set[str] = set()
    pattern = re.compile(r'(?:env|option_env)!\s*\(\s*"CARGO_BIN_EXE_([^"]+)"')
    for target_name in target_names:
        source = REPO / "tests" / f"{target_name}.rs"
        if not source.is_file():
            raise SystemExit(f"selected Cargo test target has no source file: {target_name}")
        referenced.update(pattern.findall(source.read_text(encoding="utf-8")))
    unknown = referenced.difference(BIN_TARGETS)
    if unknown:
        raise SystemExit(f"selected tests reference unknown Cargo binary targets: {sorted(unknown)}")
    return tuple(name for name in BIN_TARGETS if name in referenced)


def run(
    command: list[str],
    *,
    cwd: Path,
    test_name: str,
    env: dict[str, str] | None = None,
) -> None:
    print("$ " + shlex.join(command), flush=True)
    process = subprocess.Popen(
        command,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    output = bytearray()
    overflow = threading.Event()

    def drain() -> None:
        assert process.stdout is not None
        while True:
            try:
                chunk = os.read(process.stdout.fileno(), 65536)
            except InterruptedError:
                continue
            if not chunk:
                return
            if len(output) + len(chunk) > TEST_OUTPUT_LIMIT:
                overflow.set()
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                continue
            output.extend(chunk)

    reader = threading.Thread(target=drain)
    reader.start()
    try:
        status = process.wait(timeout=300)
    except subprocess.TimeoutExpired:
        process.kill()
        status = process.wait()
        reader.join()
        raise SystemExit("sandboxed test exceeded the 300 second wall-clock limit")
    reader.join()
    if output:
        sys.stdout.write(output.decode("utf-8", errors="replace"))
        if not output.endswith(b"\n"):
            sys.stdout.write("\n")
    if overflow.is_set():
        raise SystemExit("sandboxed test exceeded the 512 KiB output limit")
    if status:
        raise subprocess.CalledProcessError(status, command)
    require_one_passing_test(bytes(output), test_name)


def source_files(path: Path) -> list[Path]:
    if path.is_symlink():
        raise SystemExit(f"refusing a symlinked package input: {path}")
    if not path.exists():
        return []
    mode = path.lstat().st_mode
    if stat.S_ISREG(mode):
        return [path]
    if not stat.S_ISDIR(mode):
        raise SystemExit(f"refusing a non-regular package input: {path}")
    files = []
    for entry in path.rglob("*"):
        if entry.is_symlink():
            raise SystemExit(f"refusing a symlinked package input: {entry}")
        entry_mode = entry.lstat().st_mode
        if stat.S_ISREG(entry_mode):
            files.append(entry)
            if len(files) > SOURCE_STAGE_FILE_LIMIT:
                raise SystemExit(
                    f"package inputs exceed {SOURCE_STAGE_FILE_LIMIT} file staging limit"
                )
        elif not stat.S_ISDIR(entry_mode):
            raise SystemExit(f"refusing a special package input: {entry}")
    return files


def copy_source_tree(destination: Path) -> None:
    """Stage bounded Cargo inputs only; omit runtime, evidence and agent state."""
    names = ("Cargo.toml", "Cargo.lock", "src", "tests", "build.rs")
    sources = []
    for name in names:
        source = REPO / name
        if source.is_symlink():
            raise SystemExit(f"refusing a symlinked package input: {source}")
        if source.exists():
            sources.append(source)
    files = [file for source in sources for file in source_files(source)]
    if len(files) > SOURCE_STAGE_FILE_LIMIT:
        raise SystemExit(
            f"package inputs exceed {SOURCE_STAGE_FILE_LIMIT} file staging limit"
        )
    total_bytes = sum(path.lstat().st_size for path in files)
    if total_bytes > SOURCE_STAGE_LIMIT:
        raise SystemExit(f"package inputs exceed {SOURCE_STAGE_LIMIT} byte staging limit")

    destination.mkdir(mode=0o700)
    for source in sources:
        target = destination / source.name
        if source.is_dir():
            shutil.copytree(source, target, symlinks=True)
        else:
            shutil.copy2(source, target, follow_symlinks=False)
    sha256_tree(destination)
    (destination / ".runtime").mkdir(mode=0o700)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_tree(path: Path) -> str:
    """Hash a staged tree by relative name and contents, rejecting special files."""
    digest = hashlib.sha256()
    entries = sorted(path.rglob("*"))
    digest.update(b"forgejo-agent-e2e-tree-v2\0")
    digest.update(len(entries).to_bytes(8, "big"))
    for entry in entries:
        if entry.is_symlink():
            raise SystemExit(f"refusing a symlink while fingerprinting staged inputs: {entry}")
        metadata = entry.lstat()
        mode = metadata.st_mode
        if not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
            raise SystemExit(f"refusing a special file while fingerprinting staged inputs: {entry}")
        relative = entry.relative_to(path).as_posix().encode("utf-8")
        digest.update(b"D" if stat.S_ISDIR(mode) else b"F")
        digest.update(stat.S_IMODE(mode).to_bytes(4, "big"))
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        if stat.S_ISREG(mode):
            digest.update(metadata.st_size.to_bytes(8, "big"))
            with entry.open("rb") as source:
                content_bytes = 0
                while chunk := source.read(1024 * 1024):
                    content_bytes += len(chunk)
                    digest.update(chunk)
            if content_bytes != metadata.st_size:
                raise SystemExit(f"staged input changed while fingerprinting: {entry}")
    return digest.hexdigest()


def registry_index_path(name: str) -> Path:
    normalized = name.lower()
    if len(normalized) == 1:
        return Path("1") / normalized
    if len(normalized) == 2:
        return Path("2") / normalized
    if len(normalized) == 3:
        return Path("3") / normalized[0] / normalized
    return Path(normalized[:2]) / normalized[2:4] / normalized


def checked_cache_path(root: Path, path: Path, *, directory: bool = False) -> Path:
    """Require cache components to be regular directories/files, never symlinks."""
    if root.is_symlink():
        raise SystemExit(f"refusing a symlinked Cargo registry root: {root}")
    root_real = root.resolve(strict=True)
    absolute = path.absolute()
    if not absolute.is_relative_to(root_real):
        raise SystemExit(f"Cargo cache path is outside its root: {path}")
    current = root_real
    parts = absolute.relative_to(root_real).parts
    for index, part in enumerate(parts):
        current = current / part
        if current.is_symlink():
            raise SystemExit(f"refusing a symlink in Cargo cache path: {current}")
        try:
            mode = current.lstat().st_mode
        except FileNotFoundError:
            raise SystemExit(f"Cargo cache path is missing: {current}") from None
        final = index == len(parts) - 1
        if not final and not stat.S_ISDIR(mode):
            raise SystemExit(f"Cargo cache parent is not a directory: {current}")
        if final and not (stat.S_ISDIR(mode) if directory else stat.S_ISREG(mode)):
            raise SystemExit(f"Cargo cache entry has the wrong file type: {current}")
    resolved = current.resolve(strict=True) if parts else root_real
    if not resolved.is_relative_to(root_real):
        raise SystemExit(f"resolved Cargo cache path escaped its root: {path}")
    return resolved


def copy_checked_file(root: Path, source: Path, destination: Path, limit: int) -> tuple[str, int]:
    source = checked_cache_path(root, source)
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    digest = hashlib.sha256()
    copied = 0
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        with os.fdopen(descriptor, "rb") as src, destination.open("xb") as dst:
            while chunk := src.read(1024 * 1024):
                copied += len(chunk)
                if copied > limit:
                    raise SystemExit(f"Cargo cache file exceeds staging limit: {source}")
                digest.update(chunk)
                dst.write(chunk)
    except BaseException:
        destination.unlink(missing_ok=True)
        raise
    return digest.hexdigest(), copied


def copy_locked_registry(cache_root: Path, destination: Path) -> None:
    """Copy only checksummed crate archives and required sparse-index entries."""
    lock = tomllib.loads((REPO / "Cargo.lock").read_text(encoding="utf-8"))
    packages = lock.get("package", [])
    registry_packages = [package for package in packages if package.get("source")]
    if any(package["source"] != CARGO_REGISTRY_SOURCE for package in registry_packages):
        raise SystemExit("runner only permits the verified crates.io Cargo.lock source")

    if cache_root.is_symlink():
        raise SystemExit(f"refusing a symlinked Cargo registry root: {cache_root}")
    cache_root = cache_root.resolve(strict=True)
    for subdirectory in ("cache", "index"):
        checked_cache_path(cache_root, cache_root / subdirectory, directory=True)
    cache_names = sorted(path.name for path in (cache_root / "cache").glob("index.crates.io-*"))
    index_names = sorted(path.name for path in (cache_root / "index").glob("index.crates.io-*"))
    if not cache_names or cache_names != index_names:
        raise SystemExit("the offline crates.io registry cache is incomplete or ambiguous")
    registry_name = cache_names[0]

    source_index = cache_root / "index" / registry_name
    checked_cache_path(cache_root, source_index, directory=True)
    dest_index = destination / "index" / registry_name
    dest_index.mkdir(parents=True)
    copy_checked_file(
        cache_root,
        source_index / "config.json",
        dest_index / "config.json",
        limit=1024 * 1024,
    )
    source_index_cache = source_index / ".cache"
    checked_cache_path(cache_root, source_index_cache, directory=True)
    dest_index_cache = dest_index / ".cache"
    total_bytes = (dest_index / "config.json").stat().st_size
    seen_index_entries: set[str] = set()

    for package in registry_packages:
        package_name = package["name"]
        package_version = package["version"]
        crate_name = f"{package_name}-{package_version}"
        source_archive = cache_root / "cache" / registry_name / f"{crate_name}.crate"
        source_index_entry = source_index_cache / registry_index_path(package_name)
        target_archive = destination / "cache" / registry_name / f"{crate_name}.crate"
        checksum, archive_size = copy_checked_file(
            cache_root,
            source_archive,
            target_archive,
            limit=min(32 * 1024 * 1024, REGISTRY_STAGE_LIMIT - total_bytes),
        )
        if checksum != package.get("checksum"):
            raise SystemExit(f"Cargo.lock checksum mismatch for {crate_name}")
        total_bytes += archive_size

        target_index_entry = dest_index_cache / registry_index_path(package_name)
        index_key = target_index_entry.relative_to(dest_index_cache).as_posix()
        if index_key not in seen_index_entries:
            _, index_size = copy_checked_file(
                cache_root,
                source_index_entry,
                target_index_entry,
                limit=min(32 * 1024 * 1024, REGISTRY_STAGE_LIMIT - total_bytes),
            )
            total_bytes += index_size
            seen_index_entries.add(index_key)
        if total_bytes > REGISTRY_STAGE_LIMIT:
            raise SystemExit(f"locked Cargo registry data exceeds {REGISTRY_STAGE_LIMIT} bytes")


def sandbox_prefix(
    bwrap: Path,
    source_tree: Path,
    rust_sysroot: Path,
    masked_home: Path,
    registry_cache: Path | None,
) -> list[str]:
    # The root filesystem is an allowlist. In particular, never bind the host
    # root or home: Cargo build scripts and proc macros execute during compile.
    command = [
        str(bwrap),
        "--die-with-parent",
        "--unshare-net",
        "--unshare-pid",
        "--new-session",
        "--ro-bind",
        "/usr",
        "/usr",
        "--dir",
        "/etc",
        "--ro-bind",
        "/etc/alternatives",
        "/etc/alternatives",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/bin",
        "/bin",
        "--size",
        "67108864",
        "--tmpfs",
        "/home",
        "--dir",
        str(SANDBOX_REPO),
        "--ro-bind",
        str(source_tree),
        str(SANDBOX_REPO),
        "--dir",
        "/toolchain",
        "--ro-bind",
        str(rust_sysroot),
        "/toolchain",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--size",
        "134217728",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/tmp/cargo-home",
        "--size",
        "1073741824",
        "--tmpfs",
        "/build",
        "--dir",
        "/build/target",
        "--size",
        "67108864",
        "--tmpfs",
        str(SANDBOX_REPO / ".runtime"),
        "--chmod",
        "0700",
        str(SANDBOX_REPO / ".runtime"),
        "--clearenv",
        "--setenv",
        "PATH",
        "/toolchain/bin:/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/tmp",
        "--setenv",
        "TMPDIR",
        "/tmp",
        "--setenv",
        "CARGO_HOME",
        "/tmp/cargo-home",
        "--setenv",
        "CARGO_TARGET_DIR",
        "/build/target",
        "--setenv",
        "CARGO_NET_OFFLINE",
        "true",
        "--setenv",
        "CARGO_BUILD_JOBS",
        "2",
        "--setenv",
        "RUSTC",
        "/toolchain/bin/rustc",
        "--setenv",
        "RUST_TEST_THREADS",
        "1",
        "--setenv",
        "MISSION_E2E_RUST_SYSROOT",
        "/toolchain",
        "--setenv",
        "MISSION_E2E_RUST_BIN_DIR",
        "/toolchain/bin",
        "--setenv",
        "MISSION_E2E_MASKED_HOME",
        str(masked_home),
        "--chdir",
        str(SANDBOX_REPO),
    ]
    if registry_cache is not None:
        command.extend(
            ["--dir", "/bootstrap", "--ro-bind", str(registry_cache), "/bootstrap/registry"]
        )
    return command


def compile_targets(
    bwrap: Path,
    target_names: tuple[str, ...],
    binary_targets: tuple[str, ...],
    artifacts: dict[str, Path],
    source_tree: Path,
    rust_sysroot: Path,
    masked_home: Path,
    registry_cache: Path,
) -> dict[str, Path]:
    command = sandbox_prefix(
        bwrap,
        source_tree,
        rust_sysroot,
        masked_home,
        registry_cache,
    )
    command.extend(["--size", "134217728", "--tmpfs", "/artifacts"])

    # Cargo and every build script run on bounded tmpfs. Only the explicitly
    # named executables are streamed out after compilation has ended.
    helper = r'''import glob, json, os, shutil, subprocess, sys
tests = json.loads(sys.argv[1])
bins = json.loads(sys.argv[2])
artifacts = json.loads(sys.argv[3])
os.makedirs("/tmp/cargo-home/registry", exist_ok=True)
shutil.copytree("/bootstrap/registry/index", "/tmp/cargo-home/registry/index")
shutil.copytree("/bootstrap/registry/cache", "/tmp/cargo-home/registry/cache")
def run_cargo(name):
    process = subprocess.Popen([
        "/toolchain/bin/cargo", "test", "--offline", "--locked",
        "--test", name, "--no-run"
    ], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    output = bytearray()
    while chunk := process.stdout.read(65536):
        output.extend(chunk)
        if len(output) > 65536:
            del output[:-65536]
    status = process.wait()
    if output:
        sys.stderr.buffer.write(output)
        sys.stderr.buffer.flush()
    if status:
        raise SystemExit(status)
for name in tests:
    run_cargo(name)
exports = []
total = 0
for name in tests:
    paths = [
        path for path in glob.glob(f"/build/target/debug/deps/{name}-*")
        if os.path.isfile(path) and os.access(path, os.X_OK)
    ]
    if len(paths) != 1:
        raise SystemExit(f"expected one test executable for {name}, got {paths!r}")
    size = os.path.getsize(paths[0])
    if size > 67108864:
        raise SystemExit(f"test executable exceeds per-file artifact limit: {name}")
    exports.append((name, paths[0], size))
    total += size
for name in bins:
    path = f"/build/target/debug/{name}"
    if not os.path.isfile(path) or not os.access(path, os.X_OK):
        raise SystemExit(f"Cargo binary target missing or not executable: {name}")
    size = os.path.getsize(path)
    if size > 67108864:
        raise SystemExit(f"binary target exceeds per-file artifact limit: {name}")
    exports.append((name, path, size))
    total += size
if total > 134217728:
    raise SystemExit("total exported test and binary artifacts exceed 128 MiB")
for name, path, _size in exports:
    shutil.copyfile(path, f"/artifacts/{name}")
for name in artifacts:
    path = f"/artifacts/{name}"
    size = os.path.getsize(path)
    sys.stdout.buffer.write(f"ARTIFACT {name} {size}\n".encode())
    with open(path, "rb") as source:
        shutil.copyfileobj(source, sys.stdout.buffer)
    sys.stdout.buffer.flush()
'''
    command.extend(
        [
            "/usr/bin/prlimit",
            "--cpu=240",
            "--as=4294967296",
            "--nproc=384",
            "--nofile=128",
            "--fsize=67108864",
            "--",
            "/usr/bin/python3",
            "-c",
            helper,
            json.dumps(list(target_names)),
            json.dumps(list(binary_targets)),
            json.dumps(list(artifacts)),
        ]
    )
    run_artifact_build(command, cwd=REPO, artifacts=artifacts)
    return artifacts


def run_artifact_build(
    command: list[str], *, cwd: Path, artifacts: dict[str, Path]
) -> None:
    print("$ " + shlex.join(command), flush=True)
    process = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    stdout_data = bytearray()
    stderr_tail = bytearray()
    overflow = threading.Event()

    def drain(stream, *, capture_stdout: bool) -> None:
        destination = stdout_data if capture_stdout else stderr_tail
        limit = ARTIFACT_STAGE_LIMIT + 65536 if capture_stdout else 512 * 1024
        while True:
            try:
                chunk = os.read(stream.fileno(), 65536)
            except InterruptedError:
                continue
            if not chunk:
                return
            if capture_stdout and len(destination) + len(chunk) > limit:
                overflow.set()
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                continue
            destination.extend(chunk)
            if not capture_stdout and len(destination) > limit:
                del destination[:-limit]

    readers = [
        threading.Thread(target=drain, args=(process.stdout,), kwargs={"capture_stdout": True}),
        threading.Thread(target=drain, args=(process.stderr,), kwargs={"capture_stdout": False}),
    ]
    for reader in readers:
        reader.start()
    try:
        status = process.wait(timeout=300)
    except subprocess.TimeoutExpired:
        process.kill()
        status = process.wait()
        for reader in readers:
            reader.join()
        raise SystemExit("sandboxed Cargo build exceeded the 300 second wall-clock limit")
    for reader in readers:
        reader.join()
    if stderr_tail:
        sys.stdout.write(stderr_tail.decode("utf-8", errors="replace"))
        if not stderr_tail.endswith(b"\n"):
            sys.stdout.write("\n")
    if overflow.is_set():
        raise SystemExit("sandboxed build exceeded the 128 MiB artifact stream budget")
    if status:
        raise subprocess.CalledProcessError(status, command)

    cursor = 0
    total = 0
    for name, destination in artifacts.items():
        header_end = stdout_data.find(b"\n", cursor)
        if header_end < 0:
            raise SystemExit(f"sandbox build omitted artifact header: {name}")
        header = stdout_data[cursor:header_end].decode("ascii", errors="strict").split()
        if len(header) != 3 or header[0] != "ARTIFACT" or header[1] != name:
            raise SystemExit(f"unexpected sandbox artifact header for {name}: {header!r}")
        try:
            size = int(header[2])
        except ValueError:
            raise SystemExit(f"invalid sandbox artifact size for {name}") from None
        if size < 0 or size > 64 * 1024 * 1024:
            raise SystemExit(f"sandbox artifact exceeds per-file budget: {name}")
        start = header_end + 1
        end = start + size
        if end > len(stdout_data):
            raise SystemExit(f"sandbox artifact stream truncated: {name}")
        destination.write_bytes(stdout_data[start:end])
        destination.chmod(0o700)
        cursor = end
        total += size
    if cursor != len(stdout_data) or total > ARTIFACT_STAGE_LIMIT:
        raise SystemExit("sandbox artifact stream has trailing data or exceeds total budget")


def isolated_command(
    bwrap: Path,
    source_tree: Path,
    test_binary: Path,
    test_name: str,
    binary_targets: tuple[str, ...],
    rust_sysroot: Path,
    masked_home: Path,
) -> list[str]:
    command = sandbox_prefix(
        bwrap,
        source_tree,
        rust_sysroot,
        masked_home,
        None,
    )
    command.extend(["--dir", "/build/target", "--dir", "/build/target/debug"])
    for binary_name in binary_targets:
        command.extend(
            [
                "--ro-bind",
                str(test_binary.parent / binary_name),
                f"/build/target/debug/{binary_name}",
            ]
        )
    command.extend(["--ro-bind", str(test_binary.resolve()), "/test-binary"])
    command.extend(
        [
            "/usr/bin/prlimit",
            "--cpu=240",
            "--as=4294967296",
            "--nproc=384",
            "--nofile=128",
            "--fsize=67108864",
            "--",
        ]
    )
    command.extend(
        [
            "/test-binary",
            test_name,
            "--exact",
            "--ignored",
            "--nocapture",
        ]
    )
    return command


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--only",
        action="append",
        choices=sorted(test_name for _, test_name in TESTS),
        help="run only the named ignored integration test (repeatable)",
    )
    selected_names = set(parser.parse_args().only or ())
    selected_tests = tuple(
        (target_name, test_name)
        for target_name, test_name in TESTS
        if not selected_names or test_name in selected_names
    )
    if selected_names and {name for _, name in selected_tests} != selected_names:
        raise SystemExit("one or more selected test names are not present in the runner")

    rustc = shutil.which("rustc")
    bwrap = Path("/usr/bin/bwrap")
    if rustc is None:
        raise SystemExit("rustc not found in PATH")
    if not bwrap.is_file():
        raise SystemExit("/usr/bin/bwrap is required to isolate the fixed loopback endpoint")

    rust_sysroot = Path(
        subprocess.run(
            [rustc, "--print", "sysroot"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    ).resolve()
    if not rust_sysroot.is_dir():
        raise SystemExit(f"rustc returned a non-directory sysroot: {rust_sysroot}")
    masked_home = Path.home().resolve()
    registry_source = masked_home / ".cargo" / "registry"
    if not registry_source.is_dir():
        raise SystemExit(f"offline Cargo registry cache is unavailable: {registry_source}")
    with tempfile.TemporaryDirectory(prefix="forgejo-mission-delivery-e2e-", dir="/tmp") as temp:
        temp_root = Path(temp).resolve()
        source_tree = temp_root / "source"
        copy_source_tree(source_tree)
        registry_cache = temp_root / "cargo-registry"
        copy_locked_registry(registry_source, registry_cache)

        head = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPO,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        cargo_lock = source_tree / "Cargo.lock"
        print(f"evidence.git_head={head}", flush=True)
        print(f"evidence.source_tree_sha256={sha256_tree(source_tree)}", flush=True)
        print(f"evidence.cargo_lock_sha256={sha256_file(cargo_lock)}", flush=True)
        print(f"evidence.runner_sha256={sha256_file(Path(__file__).resolve())}", flush=True)
        print(f"evidence.registry_stage_sha256={sha256_tree(registry_cache)}", flush=True)
        rustc_version = subprocess.run(
            [rustc, "--version"], check=True, capture_output=True, text=True
        ).stdout.strip()
        cargo_version = subprocess.run(
            ["cargo", "--version"], check=True, capture_output=True, text=True
        ).stdout.strip()
        bwrap_version = subprocess.run(
            [str(bwrap), "--version"], check=True, capture_output=True, text=True
        ).stdout.strip()
        print(f"evidence.rustc={rustc_version}", flush=True)
        print(f"evidence.cargo={cargo_version}", flush=True)
        print(f"evidence.bwrap={bwrap_version}", flush=True)
        print(f"evidence.platform={platform.platform()}", flush=True)

        target_names = tuple(dict.fromkeys(target_name for target_name, _ in selected_tests))
        binary_targets = required_binary_targets(target_names)
        artifact_names = (*target_names, *binary_targets)
        artifacts = {}
        for name in artifact_names:
            artifact = temp_root / name
            artifact.touch(mode=0o700)
            artifact.chmod(0o700)
            artifacts[name] = artifact
        binaries = compile_targets(
            bwrap,
            target_names,
            binary_targets,
            artifacts,
            source_tree,
            rust_sysroot,
            masked_home,
            registry_cache,
        )

        for target_name, test_name in selected_tests:
            run(
                isolated_command(
                    bwrap,
                    source_tree,
                    binaries[target_name],
                    test_name,
                    binary_targets,
                    rust_sysroot,
                    masked_home,
                ),
                cwd=REPO,
                test_name=test_name,
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
