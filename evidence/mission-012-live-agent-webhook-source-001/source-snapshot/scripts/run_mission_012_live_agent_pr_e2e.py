#!/usr/bin/env python3
"""Publish and verify one completed MISSION-012 MCP candidate on local Forgejo.

This driver is deliberately limited to the private loopback fixture. Forgejo
tokens stay in process memory, Cargo runs without network or host credentials,
and the Forgejo server is stopped on exit. It is not a production authority.
"""

from __future__ import annotations

import argparse
import base64
from decimal import Decimal, InvalidOperation
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import socket
import stat
import subprocess
import sys
import tarfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_emitted as webhook_events  # noqa: E402
import run_forgejo_live_webhook_reconcile_e2e as webhook_helpers  # noqa: E402

ORIGIN = "http://127.0.0.1:3132"
OWNER = "forge-admin"
REPOSITORY = "mission-012-clamp"
REVIEWER = "verifier-bot"
BROKEN_SOURCE_SHA = "de0175e0fc5341a0ff3080f647ee16f4f4c1c45a205fc01ee8ec73899b3f9aa2"
EXPECTED_FIXED_SOURCE_SHA = "173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15"
EXPECTED_ACCEPTANCE_SHA = "5d4727542ea32f61aa4500754556fcfd1f39b59b4131ede257cb736fab231dc2"
EXPECTED_BASE_FAILURE = {
    "test_name": "values_outside_the_range_clamp_to_the_nearest_boundary",
    "assertion": "assertion `left == right` failed",
    "actual": "-2",
    "expected": "10",
    "summary": "1 passed; 1 failed",
}
REQUIREMENT_SHA = "3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431"
POLICY_SHA = "e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb"
FORGEJO_SHA = "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7"
MAX_MODEL_BUDGET_USD = Decimal("0.20")
MISSION_VERIFIER_ID = "mission-012-rust-isolated-cargo-acceptance-v2"
MERGE_GATE_VERIFIER_ID = MISSION_VERIFIER_ID
MERGE_GATE_PROFILE = "mission-012-clamp"
MERGE_GATE_EXECUTION_IDENTITY = "host-controlled-local-runtime"
MERGE_GATE_OWNER_SCOPES = ("read:user", "write:repository", "write:issue")
RUST_TOOLCHAIN = Path("/home/pc6/.rustup/toolchains/1.95.0-aarch64-unknown-linux-gnu")
CARGO_HOME = Path("/home/pc6/.cargo")
MISSION_VERIFICATION_CPU_BUDGET_MS = 1_200_000
VERIFIER_CPU_RESERVATION_MS = 240_000
VERIFIER_CPU_QUOTA_PERCENT = 200
VERIFIER_CPU_ACCOUNTING_MARGIN_MS = 1_000
MAX_VERIFIER_OUTPUT_BYTES = 2 * 1024 * 1024
PREMERGE_VERIFIER_OUTPUT_FILE = "premerge-reverification-cargo-test.log"
VERIFIER_CPU_POLICY_SHA256 = "f73785cb8eade278f2c14677c633ee34c391f2a7d7ff55e9e3e02a04b9307a50"
EXPECTED_FILES = {
    "Cargo.toml", "Cargo.lock", "README.md", "src/lib.rs",
    "tests/acceptance.rs", "requirements/MISSION-012-CLAMP-001.json",
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def validate_model_cost(summary: dict[str, object], configured_cap_usd: str) -> None:
    try:
        configured_cap = Decimal(configured_cap_usd)
    except (InvalidOperation, TypeError, ValueError):
        raise RuntimeError("model budget must be a positive decimal no higher than USD 0.20") from None
    if (
        not configured_cap.is_finite()
        or configured_cap <= 0
        or configured_cap > MAX_MODEL_BUDGET_USD
    ):
        raise RuntimeError("model budget must be a positive decimal no higher than USD 0.20")

    reported_cap = summary.get("budget_cap_usd")
    try:
        actual_cap = Decimal(str(reported_cap))
    except (InvalidOperation, TypeError, ValueError):
        raise RuntimeError("agent summary did not report a valid model budget cap") from None
    if not actual_cap.is_finite() or actual_cap <= 0 or actual_cap > configured_cap:
        raise RuntimeError("agent summary reported a budget above the configured model cap")

    agent_adapter = summary.get("agent_adapter")
    if agent_adapter == "local-test-adapter":
        if (
            summary.get("model") != "local-test-adapter"
            or summary.get("effort") != "deterministic"
            or summary.get("model_invoked_during_run") is not False
        ):
            raise RuntimeError("local test adapter has inconsistent identity or claims a model invocation")
        if actual_cap != configured_cap:
            raise RuntimeError("local test adapter summary differs from the configured no-model run")
    elif agent_adapter == "claude":
        if (
            summary.get("model") != "claude-opus-5-5"
            or summary.get("effort") != "high"
            or summary.get("model_invoked_during_run") is not True
        ):
            raise RuntimeError("Claude result does not prove the configured model invocation")
    else:
        raise RuntimeError("agent summary omitted or misstated the execution adapter")

    reported_cost = summary.get("reported_cost_usd")
    if reported_cost is None:
        raise RuntimeError("agent execution omitted its reported cost; refusing an unmeasured candidate")
    if isinstance(reported_cost, bool):
        raise RuntimeError("model reported an invalid cost value")
    try:
        cost = Decimal(str(reported_cost))
    except (InvalidOperation, TypeError, ValueError):
        raise RuntimeError("model reported an invalid cost value") from None
    if not cost.is_finite() or cost < 0 or cost > actual_cap:
        raise RuntimeError("model cost exceeded the configured model cap")
    if agent_adapter == "local-test-adapter" and cost != 0:
        raise RuntimeError("local test adapter must report zero model cost")


def verify_pinned_sources(source: bytes, acceptance: bytes, *, base: bool = False) -> None:
    expected_source = BROKEN_SOURCE_SHA if base else EXPECTED_FIXED_SOURCE_SHA
    if digest(source) != expected_source or digest(acceptance) != EXPECTED_ACCEPTANCE_SHA:
        raise RuntimeError("source or acceptance suite does not match the pinned MISSION-012 sources")


def verify_and_run_candidate(
    snapshot: Path,
    rust_sysroot: Path,
    *,
    host_binary: Path | None = None,
    expected_host_sha256: str | None = None,
    mission_journal: Path | None = None,
    lease_id: str | None = None,
    reservation_id: str | None = None,
    target_dir: Path | None = None,
) -> dict[str, object]:
    source = (snapshot / "src/lib.rs").read_bytes()
    acceptance = (snapshot / "tests/acceptance.rs").read_bytes()
    verify_pinned_sources(source, acceptance)
    if any(value is None for value in (
        host_binary, expected_host_sha256, mission_journal, lease_id, reservation_id, target_dir
    )):
        raise RuntimeError("trusted journaled verifier inputs are required")
    return run_journaled_verifier_test(
        host_binary, mission_journal, lease_id, reservation_id, snapshot, target_dir,
        rust_sysroot, expected_host_sha256,
    )


def verify_and_run_base(
    snapshot: Path,
    rust_sysroot: Path,
    *,
    host_binary: Path,
    expected_host_sha256: str,
    mission_journal: Path,
    lease_id: str,
    reservation_id: str,
    target_dir: Path,
) -> dict[str, object]:
    source = (snapshot / "src/lib.rs").read_bytes()
    acceptance = (snapshot / "tests/acceptance.rs").read_bytes()
    verify_pinned_sources(source, acceptance, base=True)
    return run_journaled_verifier_test(
        host_binary, mission_journal, lease_id, reservation_id, snapshot, target_dir,
        rust_sysroot, expected_host_sha256,
    )


def verifier_reservation_id(stage: str, base_commit: str, candidate_commit: str) -> str:
    if stage not in {"base-red", "candidate-green", "premerge-green"}:
        raise RuntimeError("unknown host-owned verifier stage")
    if not re.fullmatch(r"[0-9a-f]{40}", base_commit) or not re.fullmatch(r"[0-9a-f]{40}", candidate_commit):
        raise RuntimeError("verifier commit binding is malformed")
    operation = f"{stage}\0{base_commit}\0{candidate_commit}".encode()
    return f"pr-{stage}-{digest(operation)[:32]}"


def run_journaled_verifier_test(
    host_binary: Path,
    mission_journal: Path,
    lease_id: str,
    reservation_id: str,
    snapshot: Path,
    target_dir: Path,
    rust_sysroot: Path,
    expected_host_sha256: str,
) -> dict[str, object]:
    paths = (host_binary, mission_journal, snapshot, rust_sysroot)
    if any(path.is_symlink() or not path.is_absolute() for path in paths):
        raise RuntimeError("journaled verifier requires absolute non-symbolic paths")
    try:
        host_metadata = host_binary.lstat()
        journal_metadata = mission_journal.lstat()
        snapshot_metadata = snapshot.lstat()
        sysroot_metadata = rust_sysroot.lstat()
    except OSError:
        raise RuntimeError("journaled verifier input is unavailable") from None
    uid = os.geteuid()
    if (
        not host_binary.is_file() or host_metadata.st_uid != uid
        or host_metadata.st_nlink != 1 or stat.S_IMODE(host_metadata.st_mode) != 0o700
        or hashlib.sha256(host_binary.read_bytes()).hexdigest() != expected_host_sha256
        or not re.fullmatch(r"[0-9a-f]{64}", expected_host_sha256)
        or not mission_journal.is_file() or journal_metadata.st_uid != uid
        or journal_metadata.st_nlink != 1 or stat.S_IMODE(journal_metadata.st_mode) != 0o600
        or not snapshot.is_dir() or snapshot_metadata.st_uid != uid
        or not rust_sysroot.is_dir() or sysroot_metadata.st_uid != uid
        or not re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", lease_id)
        or not re.fullmatch(r"pr-(?:base-red|candidate-green|premerge-green)-[0-9a-f]{32}", reservation_id)
        or target_dir.exists() or target_dir.is_symlink() or not target_dir.is_absolute()
    ):
        raise RuntimeError("journaled verifier inputs failed the private provenance checks")

    command = [
        str(host_binary), "--run-journaled-verifier-test", str(mission_journal),
        lease_id, reservation_id, str(snapshot), str(target_dir), str(rust_sysroot),
    ]
    try:
        completed = subprocess.run(
            command, cwd=ROOT, env={"PATH": "/usr/bin:/bin"},
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            timeout=160, check=False,
        )
    except subprocess.TimeoutExpired:
        raise RuntimeError("trusted verifier host timed out; its unresolved reservation remains charged") from None
    if len(completed.stdout) > 2 * 1024 * 1024 or len(completed.stderr) > 64 * 1024:
        raise RuntimeError("trusted verifier host exceeded the bounded report size")
    try:
        measurement = json.loads(completed.stdout)
    except (json.JSONDecodeError, UnicodeDecodeError):
        raise RuntimeError("trusted verifier host omitted its structured measurement") from None
    if not isinstance(measurement, dict):
        raise RuntimeError("trusted verifier host measurement is not an object")
    if completed.returncode != 0 or measurement.get("verdict") != "measurement_complete":
        reason = measurement.get("reason")
        raise RuntimeError(
            f"trusted verifier host refused the operation: {reason if isinstance(reason, str) else 'inconclusive'}"
        )

    def exact_positive_integer(value: object) -> bool:
        return isinstance(value, int) and not isinstance(value, bool) and value > 0

    ledger = measurement.get("ledger")
    entries = ledger.get("reservations") if isinstance(ledger, dict) else None
    matching = [
        item for item in entries
        if isinstance(item, dict) and item.get("reservation_id") == reservation_id
    ] if isinstance(entries, list) else []
    if (
        measurement.get("schema_version") != 1
        or measurement.get("lease_id") != lease_id
        or measurement.get("reservation_id") != reservation_id
        or measurement.get("policy_sha256") != VERIFIER_CPU_POLICY_SHA256
        or measurement.get("mission_cpu_budget_ms") != MISSION_VERIFICATION_CPU_BUDGET_MS
        or measurement.get("reserved_cpu_ms") != VERIFIER_CPU_RESERVATION_MS
        or not exact_positive_integer(measurement.get("charged_cpu_ms"))
        or measurement.get("charged_cpu_ms") > VERIFIER_CPU_RESERVATION_MS
        or not exact_positive_integer(measurement.get("elapsed_ms"))
        or not exact_positive_integer(measurement.get("accounting_elapsed_ms"))
        or (measurement.get("exit_code") is not None and (
            not isinstance(measurement.get("exit_code"), int)
            or isinstance(measurement.get("exit_code"), bool)
        ))
        or not isinstance(measurement.get("timed_out"), bool)
        or not isinstance(measurement.get("output_truncated"), bool)
        or not isinstance(measurement.get("stdout"), str)
        or not isinstance(measurement.get("stderr"), str)
        or not isinstance(ledger, dict)
        or ledger.get("mission_id") != measurement.get("mission_id")
        or ledger.get("policy_sha256") != VERIFIER_CPU_POLICY_SHA256
        or ledger.get("mission_cpu_budget_ms") != MISSION_VERIFICATION_CPU_BUDGET_MS
        or ledger.get("charged_cpu_ms") != sum(
            item.get("charged_cpu_ms", -1) for item in entries
            if isinstance(item, dict) and isinstance(item.get("charged_cpu_ms"), int)
        )
        or ledger.get("charged_cpu_ms", MISSION_VERIFICATION_CPU_BUDGET_MS + 1) > MISSION_VERIFICATION_CPU_BUDGET_MS
        or len(matching) != 1
        or matching[0].get("resolved") is not True
        or matching[0].get("reserved_cpu_ms") != VERIFIER_CPU_RESERVATION_MS
        or matching[0].get("charged_cpu_ms") != measurement.get("charged_cpu_ms")
    ):
        raise RuntimeError("trusted verifier CPU receipt is incomplete or not durably resolved")
    output = (measurement["stdout"] + "\n" + measurement["stderr"]).encode()
    return {
        "exit_code": measurement["exit_code"],
        "timed_out": measurement["timed_out"],
        "output_truncated": measurement["output_truncated"],
        "elapsed_ms": measurement["elapsed_ms"],
        "accounting_elapsed_ms": measurement["accounting_elapsed_ms"],
        "charged_cpu_ms": measurement["charged_cpu_ms"],
        "reserved_cpu_ms": measurement["reserved_cpu_ms"],
        "policy_sha256": measurement["policy_sha256"],
        "mission_id": measurement["mission_id"],
        "reservation_id": reservation_id,
        "ledger": ledger,
        "output": output,
        "output_sha256": digest(output),
    }


def run_retrospective_cargo_test(snapshot: Path, rust_sysroot: Path) -> tuple[int, bytes]:
    """Run a post-merge diagnostic with the same OS limits, outside pre-merge authority.

    The active PR pipeline must use `run_journaled_verifier_test`; this helper
    exists only for the separate retrospective report, which can be requested
    after the mission lease has expired and therefore cannot amend its ledger.
    """
    runtime = Path(f"/run/user/{os.geteuid()}")
    try:
        runtime_metadata = runtime.lstat()
        bus_metadata = (runtime / "bus").lstat()
    except OSError:
        raise RuntimeError("retrospective verifier requires the current private systemd user bus") from None
    if (
        not runtime.is_dir() or runtime_metadata.st_uid != os.geteuid()
        or stat.S_IMODE(runtime_metadata.st_mode) & 0o077 != 0
        or not stat.S_ISSOCK(bus_metadata.st_mode) or bus_metadata.st_uid != os.geteuid()
    ):
        raise RuntimeError("retrospective verifier systemd bus is not owner-private")
    bwrap = Path("/usr/bin/bwrap")
    if (
        bwrap.is_symlink() or not bwrap.is_file()
        or snapshot.is_symlink() or not snapshot.is_dir()
        or rust_sysroot.is_symlink() or not rust_sysroot.is_dir()
    ):
        raise RuntimeError("retrospective verifier inputs are missing or redirected")
    sandbox = [
        str(bwrap), "--unshare-all", "--unshare-user", "--disable-userns",
        "--assert-userns-disabled", "--die-with-parent", "--new-session", "--unshare-pid",
        "--dir", "/etc", "--ro-bind", "/etc/alternatives", "/etc/alternatives",
        "--ro-bind", "/usr", "/usr", "--symlink", "usr/lib", "/lib",
        "--symlink", "usr/bin", "/bin", "--ro-bind", str(rust_sysroot), "/toolchain",
        "--ro-bind", str(snapshot), "/candidate", "--size", "268435456", "--tmpfs", "/target",
        "--size", "134217728", "--tmpfs", "/tmp", "--proc", "/proc", "--dev", "/dev",
        "--chdir", "/candidate", "--clearenv", "--setenv", "PATH", "/toolchain/bin:/usr/bin:/bin",
        "--setenv", "RUSTC", "/toolchain/bin/rustc", "--setenv", "CARGO_HOME", "/tmp/cargo-home",
        "--setenv", "CARGO_TARGET_DIR", "/target", "--setenv", "CARGO_NET_OFFLINE", "true",
        "--setenv", "CARGO_BUILD_JOBS", "2", "--setenv", "HOME", "/tmp",
        "--setenv", "TMPDIR", "/tmp", "--dir", "/tmp/cargo-home",
        "/usr/bin/prlimit", "--cpu=120", "--as=4294967296", "--nproc=32",
        "--nofile=128", "--fsize=67108864", "--", "/toolchain/bin/cargo",
        "test", "--offline", "--locked",
    ]
    command = [
        "/usr/bin/systemd-run", "--user", "--scope", "--quiet", "--collect",
        "--property=MemoryMax=1073741824", "--property=TasksMax=64",
        "--property=CPUQuota=200%", "--property=RuntimeMaxSec=120s",
        "--property=TimeoutStopSec=1s", "--", *sandbox,
    ]
    try:
        result = subprocess.run(
            command,
            env={
                "PATH": "/usr/bin:/bin",
                "XDG_RUNTIME_DIR": str(runtime),
                "DBUS_SESSION_BUS_ADDRESS": f"unix:path={runtime / 'bus'}",
            },
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=150, check=False,
        )
    except subprocess.TimeoutExpired:
        raise RuntimeError("retrospective verifier exceeded its outer 150 second bound") from None
    if len(result.stdout) > 256 * 1024:
        raise RuntimeError("retrospective verifier exceeded its bounded output")
    return result.returncode, result.stdout


def listener_output_is_loopback(output: str, pid: int) -> bool:
    rows = [line.split() for line in output.splitlines() if line.strip()]
    if not rows:
        return False
    for fields in rows:
        if len(fields) < 5 or fields[3] != "127.0.0.1:3132":
            return False
        row_pids = re.findall(r"(?:^|[,(])pid=(\d+)(?=,|\))", " ".join(fields[5:]))
        if row_pids != [str(pid)]:
            return False
    return True


def branch_protection_matches_policy(protection: dict[str, object]) -> bool:
    return (
        protection.get("branch_name") == "main"
        and protection.get("required_approvals") == 1
        and protection.get("enable_approvals_whitelist") is True
        and protection.get("approvals_whitelist_username") == [REVIEWER]
        and protection.get("enable_merge_whitelist") is True
        and protection.get("merge_whitelist_usernames") == [OWNER]
        and protection.get("merge_whitelist_teams") == []
        and protection.get("apply_to_admins") is True
        and protection.get("dismiss_stale_approvals") is True
        and protection.get("block_on_outdated_branch") is True
        and protection.get("enable_push") is False
    )


def merge_parents_match(parents: list[str], base: str, candidate: str) -> bool:
    return parents == [base, candidate]


def build_rust_merge_policy(reviewer_id: int) -> dict[str, object]:
    if isinstance(reviewer_id, bool) or not isinstance(reviewer_id, int) or reviewer_id <= 0:
        raise RuntimeError("Rust merge policy requires a positive Forgejo reviewer ID")
    versioned_policy_path = (
        Path(__file__).resolve().parents[1]
        / "tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v4.json"
    )
    if versioned_policy_path.is_symlink() or digest(versioned_policy_path.read_bytes()) != POLICY_SHA:
        raise RuntimeError("versioned MISSION-012 policy differs from the admitted fixture")
    try:
        source_policy = json.loads(versioned_policy_path.read_bytes())
        verification = source_policy["verification"]
        criteria = source_policy["candidate_policy"]["protected_unchanged_paths"]
        command = verification["command"]
        revision = source_policy["revision"]
        timeout_ms = verification["timeout_ms"]
        required_approvals = verification["required_approvals"]
        if (
            not isinstance(command, list) or not command
            or any(not isinstance(part, str) or not part for part in command)
            or not isinstance(criteria, list) or not criteria
            or any(not isinstance(path, str) or not path for path in criteria)
            or isinstance(revision, bool) or not isinstance(revision, int) or revision < 1
            or isinstance(timeout_ms, bool) or not isinstance(timeout_ms, int) or timeout_ms < 1
            or isinstance(required_approvals, bool)
            or not isinstance(required_approvals, int) or required_approvals != 1
            or not isinstance(verification.get("environment"), str)
            or not verification["environment"]
        ):
            raise ValueError("invalid versioned verification policy fields")
    except (KeyError, TypeError, ValueError, json.JSONDecodeError):
        raise RuntimeError("versioned MISSION-012 policy cannot be projected to the local merge gate") from None
    return {
        "schema_version": 1,
        "revision": revision,
        "verifier_id": MERGE_GATE_VERIFIER_ID,
        "environment": f"{verification['environment']}; source policy sha256={POLICY_SHA}",
        "acceptance_command": command,
        "commands": [command],
        "protected_criteria": criteria,
        "timeout_seconds": max(1, (timeout_ms + 999) // 1000),
        "merge_approval_policy": {
            "schema_version": 1,
            "required_approvals": required_approvals,
            "authorized_reviewer_ids": [reviewer_id],
        },
    }


def write_private_file(path: Path, content: bytes) -> None:
    if not path.parent.is_dir() or path.parent.is_symlink():
        raise RuntimeError("private merge input parent must be an existing regular directory")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(content)
        stream.flush()
        os.fsync(stream.fileno())


def write_private_json(path: Path, value: object) -> None:
    write_private_file(path, json.dumps(value, sort_keys=True, indent=2).encode() + b"\n")


def stage_private_executable(source: Path, destination: Path) -> Path:
    if source.is_symlink() or not destination.parent.is_dir() or destination.parent.is_symlink():
        raise RuntimeError("Rust merge-gate executable staging path is not a private regular file")
    source_fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(source_fd, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if (
            not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid()
            or not (metadata.st_mode & 0o111)
        ):
            raise RuntimeError("Cargo did not produce an owned regular executable merge-gate artifact")
        content = stream.read(256 * 1024 * 1024 + 1)
    if len(content) > 256 * 1024 * 1024:
        raise RuntimeError("Cargo merge-gate executable exceeds the staging size bound")
    write_private_file(destination, content)
    os.chmod(destination, 0o700, follow_symlinks=False)
    staged = destination.lstat()
    if (
        destination.is_symlink() or not stat.S_ISREG(staged.st_mode)
        or staged.st_uid != os.geteuid() or staged.st_nlink != 1
        or stat.S_IMODE(staged.st_mode) != 0o700
    ):
        raise RuntimeError("staged Rust merge-gate executable is not private and independent")
    return destination


def build_rust_merge_gate_tools(state: Path) -> dict[str, object]:
    toolchain = RUST_TOOLCHAIN.resolve(strict=True)
    cargo = toolchain / "bin/cargo"
    rustc = toolchain / "bin/rustc"
    cargo_home = CARGO_HOME.resolve(strict=True)
    if (
        RUST_TOOLCHAIN.is_symlink() or cargo.is_symlink() or not cargo.is_file()
        or rustc.is_symlink() or not rustc.is_file()
        or CARGO_HOME.is_symlink() or not cargo_home.is_dir()
        or state.is_symlink() or not state.is_dir()
        or state.stat().st_uid != os.geteuid() or stat.S_IMODE(state.stat().st_mode) != 0o700
    ):
        raise RuntimeError("offline Rust merge-gate toolchain or private build state is unavailable")

    target_dir = state / "rust-merge-gate-target"
    target_dir.mkdir(mode=0o700)
    command = [
        str(cargo), "build", "--offline", "--locked", "--manifest-path", str(ROOT / "Cargo.toml"),
        "--target-dir", str(target_dir), "--bin", "gate", "--bin", "forgejo-reconcile-local",
    ]
    env = {
        "PATH": f"{toolchain / 'bin'}:/usr/bin:/bin",
        "HOME": "/nonexistent",
        "TMPDIR": "/tmp",
        "CARGO_HOME": str(cargo_home),
        "CARGO_TARGET_DIR": str(target_dir),
        "CARGO_NET_OFFLINE": "true",
        "RUSTC": str(rustc),
        "LC_ALL": "C",
    }
    try:
        built = subprocess.run(
            command, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300, check=False,
        )
    except subprocess.TimeoutExpired:
        raise RuntimeError("offline Rust merge-gate build exceeded its 300 second deadline") from None
    if len(built.stdout) > 1024 * 1024:
        raise RuntimeError("offline Rust merge-gate build exceeded its output bound")
    write_private_file(state / "rust-merge-gate-build.log", built.stdout)
    if built.returncode:
        raise RuntimeError("offline Rust merge-gate build failed; inspect the private build log")

    cargo_gate_binary = target_dir / "debug/gate"
    cargo_authority_binary = target_dir / "debug/forgejo-reconcile-local"
    private_bin_dir = state / "rust-merge-gate-bin"
    private_bin_dir.mkdir(mode=0o700)
    if private_bin_dir.is_symlink() or private_bin_dir.stat().st_uid != os.geteuid():
        raise RuntimeError("private Rust merge-gate binary directory was redirected")
    gate_binary = stage_private_executable(cargo_gate_binary, private_bin_dir / "gate")
    authority_binary = stage_private_executable(
        cargo_authority_binary, private_bin_dir / "forgejo-reconcile-local",
    )
    source_names = (
        "Cargo.toml", "Cargo.lock", "src/lib.rs", "src/forgejo.rs",
        "src/bin/gate.rs", "src/bin/forgejo-reconcile-local.rs",
    )
    source_hashes = {}
    for name in source_names:
        source = ROOT / name
        if source.is_symlink() or not source.is_file():
            raise RuntimeError("Rust merge-gate build source is missing or redirected")
        source_hashes[name] = digest(source.read_bytes())
    return {
        "gate_binary": gate_binary,
        "authority_binary": authority_binary,
        "record": {
            "build_command": command,
            "build_exit_code": built.returncode,
            "cargo_toolchain": str(cargo),
            "rustc_toolchain": str(rustc),
            "cargo_home": str(cargo_home),
            "offline_locked": True,
            "build_log_sha256": digest(built.stdout),
            "source_sha256": source_hashes,
            "binary_sha256": {
                "gate": digest(gate_binary.read_bytes()),
                "forgejo-reconcile-local": digest(authority_binary.read_bytes()),
            },
        },
    }


def bounded_gate_command(command: list[str], *, timeout: int = 30) -> bytes:
    result = subprocess.run(
        command,
        env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "TMPDIR": "/tmp", "LC_ALL": "C"},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if len(result.stdout) > 256 * 1024 or len(result.stderr) > 64 * 1024:
        raise RuntimeError("Rust merge-gate command exceeded its bounded output")
    if result.returncode:
        raise RuntimeError(
            f"Rust merge-gate command failed with exit {result.returncode}: "
            f"{result.stderr.decode('utf-8', 'replace')[:1024]}"
        )
    return result.stdout


def prepare_rust_merge_inputs(
    *,
    state: Path,
    gate_binary: Path,
    verifier_report: dict[str, object],
    base_commit: str,
    candidate_commit: str,
    reviewer_id: int,
) -> dict[str, object]:
    if (
        verifier_report.get("verdict") != "pass"
        or verifier_report.get("verifier_id") != MERGE_GATE_VERIFIER_ID
        or verifier_report.get("base_commit") != base_commit
        or verifier_report.get("candidate_commit") != candidate_commit
        or verifier_report.get("requirement_sha256") != REQUIREMENT_SHA
        or verifier_report.get("policy_sha256") != POLICY_SHA
        or not _is_full_commit_id(base_commit)
        or not _is_full_commit_id(candidate_commit)
    ):
        raise RuntimeError("Rust merge proof requires a passing verifier report for the exact PR candidate")
    input_dir = state / "rust-merge-gate-inputs"
    input_dir.mkdir(mode=0o700)
    if input_dir.stat().st_uid != os.geteuid() or stat.S_IMODE(input_dir.stat().st_mode) != 0o700:
        raise RuntimeError("Rust merge-gate inputs must remain in a private host-owned directory")
    policy = build_rust_merge_policy(reviewer_id)
    policy_bytes = json.dumps(policy, sort_keys=True, separators=(",", ":")).encode()
    policy_sha256 = digest(policy_bytes)
    verifier = {
        **verifier_report,
        "merge_gate_policy_sha256": policy_sha256,
        "merge_gate_policy_revision": policy["revision"],
    }
    verifier_bytes = json.dumps(verifier, sort_keys=True, indent=2).encode() + b"\n"
    verifier_sha256 = digest(verifier_bytes)
    target = {
        "profile_selector": MERGE_GATE_PROFILE,
        "origin": ORIGIN,
        "owner": OWNER,
        "repository": REPOSITORY,
        "base_branch": "main",
    }
    expected = {
        "requirement_sha256": REQUIREMENT_SHA,
        "policy_sha256": policy_sha256,
        "candidate_commit": candidate_commit,
        "base_commit": base_commit,
        "merge_commit": None,
        "verifier_id": MERGE_GATE_VERIFIER_ID,
    }
    proof_evidence = {
        "schema_version": 1,
        "requirement_sha256": REQUIREMENT_SHA,
        "policy_sha256": policy_sha256,
        "candidate_commit": candidate_commit,
        "base_commit": base_commit,
        "merge_commit": None,
        "verifier_id": MERGE_GATE_VERIFIER_ID,
        "environment": policy["environment"],
        "commands": policy["commands"],
        "artifact_sha256": [verifier_sha256],
        "outcome": "pass",
        "detail": (
            f"Independent acceptance report sha256={verifier_sha256}; "
            f"versioned source policy sha256={POLICY_SHA}; "
            f"host merge policy sha256={policy_sha256}."
        ),
    }
    key_path = input_dir / "verifier.key"
    target_path = input_dir / "target.json"
    expected_path = input_dir / "expected.json"
    policy_path = input_dir / "merge-policy.json"
    evidence_path = input_dir / "proof-evidence.json"
    proof_path = input_dir / "target-bound-proof.json"
    write_private_file(key_path, os.urandom(32))
    write_private_json(target_path, target)
    write_private_json(expected_path, expected)
    write_private_file(policy_path, policy_bytes)
    write_private_json(evidence_path, proof_evidence)
    bounded_gate_command([
        str(gate_binary), "sign-target-v2", str(key_path), str(evidence_path),
        str(target_path), str(proof_path),
    ])
    proof_path.chmod(0o600)
    bounded_gate_command([
        str(gate_binary), "check-target-v2", str(key_path), str(proof_path),
        str(target_path), str(expected_path),
    ])
    proof_document = json.loads(proof_path.read_bytes())
    if (
        proof_path.is_symlink() or proof_path.stat().st_uid != os.geteuid()
        or stat.S_IMODE(proof_path.stat().st_mode) != 0o600
        or set(proof_document) != {"schema_version", "target", "evidence", "target_mac_sha256"}
        or proof_document.get("schema_version") != 2
        or proof_document.get("target") != target
        or proof_document.get("evidence") != proof_evidence
        or not isinstance(proof_document.get("target_mac_sha256"), str)
        or not re.fullmatch(r"[0-9a-f]{64}", proof_document["target_mac_sha256"])
    ):
        raise RuntimeError("Rust target-bound signer returned a proof with unexpected content")
    return {
        "input_dir": input_dir,
        "key": key_path,
        "target": target_path,
        "expected": expected_path,
        "policy": policy_path,
        "proof": proof_path,
        "verifier_report": verifier,
        "verifier_report_sha256": verifier_sha256,
        "policy_sha256": policy_sha256,
        "proof_sha256": digest(proof_path.read_bytes()),
    }


def verifier_cpu_run_evidence(
    stage: str,
    base_commit: str,
    candidate_commit: str,
    tested_commit: str,
    result: dict[str, object],
    *,
    expected_mission_id: str,
) -> dict[str, object]:
    expected_reservation_id = verifier_reservation_id(stage, base_commit, candidate_commit)
    if (
        tested_commit not in {base_commit, candidate_commit}
        or result.get("reservation_id") != expected_reservation_id
        or result.get("policy_sha256") != VERIFIER_CPU_POLICY_SHA256
        or result.get("reserved_cpu_ms") != VERIFIER_CPU_RESERVATION_MS
    ):
        raise RuntimeError("verifier CPU receipt is not bound to this PR stage and commit pair")
    if (
        not isinstance(expected_mission_id, str)
        or re.fullmatch(r"mission-v1-[0-9a-f]{64}", expected_mission_id) is None
        or result.get("mission_id") != expected_mission_id
    ):
        raise RuntimeError("verifier CPU receipt mission identity differs from admission")
    output = result.get("output")
    if not isinstance(output, bytes) or result.get("output_sha256") != digest(output):
        raise RuntimeError("verifier CPU output is not bound to its SHA-256 receipt")
    expected_failure = None
    if stage == "base-red":
        required_output = (
            EXPECTED_BASE_FAILURE["test_name"].encode(),
            EXPECTED_BASE_FAILURE["assertion"].encode(),
            f"left: {EXPECTED_BASE_FAILURE['actual']}".encode(),
            f"right: {EXPECTED_BASE_FAILURE['expected']}".encode(),
            EXPECTED_BASE_FAILURE["summary"].encode(),
        )
        if (
            result.get("exit_code") != 101
            or result.get("timed_out") is not False
            or result.get("output_truncated") is not False
            or any(marker not in output for marker in required_output)
        ):
            raise RuntimeError("base-red verifier did not show the expected acceptance assertion failure")
        expected_failure = dict(EXPECTED_BASE_FAILURE)
    elif (
        result.get("exit_code") != 0
        or result.get("timed_out") is not False
        or result.get("output_truncated") is not False
        or b"2 passed; 0 failed" not in output
    ):
        raise RuntimeError("green verifier did not show both protected acceptance tests passing")
    receipt = {
        "stage": stage,
        "base_commit": base_commit,
        "candidate_commit": candidate_commit,
        "tested_commit": tested_commit,
        "mission_id": expected_mission_id,
        "reservation_id": expected_reservation_id,
        "policy_sha256": result["policy_sha256"],
        "reserved_cpu_ms": result["reserved_cpu_ms"],
        "charged_cpu_upper_bound_ms": result["charged_cpu_ms"],
        "accounting_elapsed_ms": result["accounting_elapsed_ms"],
        "sandbox_elapsed_ms": result["elapsed_ms"],
        "exit_code": result["exit_code"],
        "timed_out": result["timed_out"],
        "output_truncated": result["output_truncated"],
        "output_sha256": result["output_sha256"],
        "expected_failure": expected_failure,
    }
    receipt["output_b64"] = base64.b64encode(output).decode("ascii")
    return receipt


def validate_final_verifier_cpu_ledger(
    ledger: object,
    runs: list[dict[str, object]],
    *,
    expected_mission_id: str,
) -> dict[str, object]:
    if not isinstance(ledger, dict):
        raise RuntimeError("final mission CPU ledger is missing")
    if (
        not isinstance(expected_mission_id, str)
        or re.fullmatch(r"mission-v1-[0-9a-f]{64}", expected_mission_id) is None
        or ledger.get("mission_id") != expected_mission_id
    ):
        raise RuntimeError("final mission CPU ledger mission identity is not admission-bound")
    entries = ledger.get("reservations")
    if not isinstance(entries, list):
        raise RuntimeError("final mission CPU ledger has no reservation list")
    entry_map: dict[str, dict[str, object]] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise RuntimeError("final mission CPU ledger contains a malformed reservation")
        reservation_id = entry.get("reservation_id")
        reserved = entry.get("reserved_cpu_ms")
        charged = entry.get("charged_cpu_ms")
        if (
            not isinstance(reservation_id, str) or reservation_id in entry_map
            or not isinstance(reserved, int) or isinstance(reserved, bool) or reserved <= 0
            or not isinstance(charged, int) or isinstance(charged, bool) or charged <= 0 or charged > reserved
            or not isinstance(entry.get("resolved"), bool)
        ):
            raise RuntimeError("final mission CPU ledger contains an invalid or duplicated reservation")
        entry_map[reservation_id] = entry

    run_ids: set[str] = set()
    for run in runs:
        if run.get("mission_id") != expected_mission_id:
            raise RuntimeError("verifier step mission identity differs from admission")
        reservation_id = run.get("reservation_id")
        if not isinstance(reservation_id, str) or reservation_id in run_ids:
            raise RuntimeError("verifier steps repeat a mission CPU reservation ID")
        run_ids.add(reservation_id)
        entry = entry_map.get(reservation_id)
        if (
            entry is None or entry.get("resolved") is not True
            or entry.get("reserved_cpu_ms") != run.get("reserved_cpu_ms")
            or entry.get("charged_cpu_ms") != run.get("charged_cpu_upper_bound_ms")
        ):
            raise RuntimeError("a PR verifier step is missing its durable CPU resolution")

    if any(entry.get("resolved") is not True for entry in entry_map.values()):
        raise RuntimeError("final mission CPU ledger contains an unresolved reservation")

    total = sum(entry["charged_cpu_ms"] for entry in entry_map.values())
    ledger_charged = ledger.get("charged_cpu_ms")
    ledger_remaining = ledger.get("remaining_cpu_ms")
    if (
        ledger.get("policy_sha256") != VERIFIER_CPU_POLICY_SHA256
        or ledger.get("mission_cpu_budget_ms") != MISSION_VERIFICATION_CPU_BUDGET_MS
        or not isinstance(ledger_charged, int)
        or isinstance(ledger_charged, bool)
        or ledger_charged != total
        or total > MISSION_VERIFICATION_CPU_BUDGET_MS
        or not isinstance(ledger_remaining, int)
        or isinstance(ledger_remaining, bool)
        or ledger_remaining != MISSION_VERIFICATION_CPU_BUDGET_MS - total
    ):
        raise RuntimeError("final mission CPU ledger does not reconcile to the pinned budget")
    return ledger


def run_rust_merge_gate(
    binary: Path,
    *,
    pull_request: int,
    proof: Path,
    expected: Path,
    key: Path,
    policy: Path,
    review_base_binding: Path,
    admission_file: Path,
    mission_journal: Path,
    owner_token: str,
) -> tuple[int, dict[str, object]]:
    if (
        isinstance(pull_request, bool) or not isinstance(pull_request, int) or pull_request <= 0
        or binary.is_symlink() or not binary.is_file()
    ):
        raise RuntimeError("Rust merge authority binary or pull request is invalid")
    for path in (proof, expected, key, policy, review_base_binding, admission_file, mission_journal):
        metadata = path.lstat()
        if (
            path.is_symlink() or not path.is_file() or metadata.st_uid != os.geteuid()
            or metadata.st_nlink != 1 or stat.S_IMODE(metadata.st_mode) != 0o600
        ):
            raise RuntimeError("Rust merge authority inputs must be private regular files")
    try:
        token_bytes = owner_token.encode("ascii")
    except UnicodeEncodeError:
        raise RuntimeError("Forgejo owner credential is invalid") from None
    if not token_bytes or len(token_bytes) > 4096 or any(byte in token_bytes for byte in b"\r\n\t "):
        raise RuntimeError("Forgejo owner credential is invalid")
    read_fd, write_fd = os.pipe()
    process = None
    try:
        offset = 0
        while offset < len(token_bytes):
            count = os.write(write_fd, token_bytes[offset:])
            if count <= 0:
                raise RuntimeError("could not deliver the private Forgejo credential pipe")
            offset += count
        os.close(write_fd)
        write_fd = -1
        command = [
            str(binary), "--merge-local-test", "--target-profile", MERGE_GATE_PROFILE,
            "--mission-admission-file", str(admission_file),
            "--mission-execution-journal", str(mission_journal),
            str(pull_request), str(proof), str(expected), str(key), str(policy),
            str(review_base_binding), MERGE_GATE_EXECUTION_IDENTITY, str(read_fd),
        ]
        if token_bytes.decode("ascii") in command:
            raise RuntimeError("Forgejo credential appeared in Rust merge-gate arguments")
        process = subprocess.Popen(
            command,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "TMPDIR": "/tmp", "LC_ALL": "C"},
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            pass_fds=(read_fd,),
            start_new_session=True,
        )
        os.close(read_fd)
        read_fd = -1
        try:
            stdout, stderr = process.communicate(timeout=120)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate(timeout=5)
            raise RuntimeError("Rust merge authority exceeded its 120 second deadline") from None
    finally:
        if read_fd >= 0:
            os.close(read_fd)
        if write_fd >= 0:
            os.close(write_fd)
    if token_bytes in stdout or token_bytes in stderr:
        raise RuntimeError("Rust merge authority output contained a private Forgejo credential")
    if len(stdout) > 256 * 1024 or len(stderr) > 64 * 1024:
        raise RuntimeError("Rust merge authority exceeded its bounded output")
    try:
        result = json.loads(stdout)
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise RuntimeError("Rust merge authority returned invalid JSON") from None
    if not isinstance(result, dict):
        raise RuntimeError("Rust merge authority returned a non-object result")
    return process.returncode, result


def _is_full_commit_id(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{40}", value) is not None


def validate_rust_merge_gate_result(
    result: dict[str, object], *, pull_request: int, base: str, candidate: str,
    admission: dict[str, object], admission_sha256: str,
) -> None:
    merge_commit = result.get("merge_commit_sha")
    target = result.get("target")
    allocation = admission.get("allocation")
    mission_binding = result.get("mission_admission")
    request = admission.get("request")
    mission_id = allocation.get("mission_id") if isinstance(allocation, dict) else None
    allocation_id = allocation.get("allocation_id") if isinstance(allocation, dict) else None
    generation = allocation.get("execution_generation") if isinstance(allocation, dict) else None
    issue_number = admission.get("issue_number")
    request_sha256 = mission_binding.get("request_sha256") if isinstance(mission_binding, dict) else None
    if (
        result.get("schema_version") != 1
        or result.get("operation") != "local_test_merge_authority"
        or result.get("verdict") != "merged"
        or result.get("pull_request_number") != pull_request
        or result.get("merge_authorized") is not True
        or result.get("merge_attempted") is not True
        or result.get("http_status") != 200
        or result.get("submitted_head_sha") != candidate
        or result.get("validated_base_sha") != base
        or not _is_full_commit_id(merge_commit)
        or result.get("main_head_sha") != merge_commit
        or result.get("postmerge_base_sha") != merge_commit
        or result.get("merge_commit_base_parent_sha") != base
        or result.get("merge_commit_head_parent_sha") != candidate
        or result.get("issue_state") != "closed"
        or not isinstance(mission_binding, dict)
        or mission_binding.get("schema_version") != 1
        or mission_binding.get("admission_sha256") != admission_sha256
        or mission_binding.get("issue_number") != issue_number
        or mission_binding.get("mission_id") != mission_id
        or mission_binding.get("allocation_id") != allocation_id
        or mission_binding.get("execution_generation") != generation
        or mission_binding.get("base_commit") != base
        or mission_binding.get("requirement_sha256") != admission.get("requirement_sha256")
        or not isinstance(request_sha256, str)
        or re.fullmatch(r"[0-9a-f]{64}", request_sha256) is None
        or not isinstance(request, dict)
        or target != {
            "selector": MERGE_GATE_PROFILE,
            "origin": ORIGIN,
            "owner": OWNER,
            "repository": REPOSITORY,
            "base_branch": "main",
        }
    ):
        raise RuntimeError("Rust merge authority did not confirm the exact verified candidate and readback")


def execution_binding_from_admission(admission: dict[str, object]) -> dict[str, object]:
    allocation = admission.get("allocation")
    request = admission.get("request")
    workspace = request.get("workspace") if isinstance(request, dict) else None
    if not isinstance(allocation, dict) or not isinstance(workspace, dict):
        raise RuntimeError("admission lacks the execution binding fields")
    binding = {
        "allocation_id": allocation.get("allocation_id"),
        "base_commit": admission.get("base_commit"),
        "execution_generation": allocation.get("execution_generation"),
        "mission_id": allocation.get("mission_id"),
        "requirement_sha256": admission.get("requirement_sha256"),
        "workspace_id": workspace.get("id"),
    }
    if (
        any(value is None for value in binding.values())
        or not isinstance(binding["mission_id"], str)
        or re.fullmatch(r"mission-v1-[0-9a-f]{64}", binding["mission_id"]) is None
    ):
        raise RuntimeError("admission lacks the execution binding fields")
    return binding


def verify_host_receipt(
    summary: dict[str, object], candidate_root: Path,
    expected_binding: dict[str, object] | None = None,
) -> str:
    try:
        if candidate_root.is_symlink():
            raise RuntimeError("candidate root cannot be a symbolic link")
        root = candidate_root.resolve(strict=True)
        root_metadata = root.stat()
        receipt = Path(str(summary["host_receipt_path"]))
        if not receipt.is_absolute() or receipt.is_symlink() or receipt.name != "host-receipts.jsonl":
            raise RuntimeError("receipt path must be the private host receipt file")
        resolved = receipt.resolve(strict=True)
        metadata = resolved.stat()
        if (
            stat.S_IMODE(root_metadata.st_mode) != 0o700 or resolved.parent != root
            or not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_size > 1024 * 1024
        ):
            raise RuntimeError("receipt is outside the private candidate root or has unsafe metadata")
        raw = resolved.read_bytes()
        receipt_sha = digest(raw)
        if summary.get("receipt_sha256") != receipt_sha:
            raise RuntimeError("receipt hash differs from the model-run summary")
        lines = raw.splitlines()
        if any(not line for line in lines):
            raise RuntimeError("receipt contains an empty event line")
        records = [json.loads(line) for line in lines]
        count = summary.get("receipt_count")
        if not isinstance(count, int) or isinstance(count, bool) or len(records) != count or not records:
            raise RuntimeError("receipt count differs from the model-run summary")
        if not all(isinstance(record, dict) for record in records):
            raise RuntimeError("receipt event is not a JSON object")
        bindings = [record.get("execution_binding") for record in records]
        if (
            not all(isinstance(binding, dict) for binding in bindings)
            or any(binding != bindings[0] for binding in bindings[1:])
        ):
            raise RuntimeError("receipt execution binding is missing or inconsistent")
        if expected_binding is not None and any(
            bindings[0].get(key) != value for key, value in expected_binding.items()
        ):
            raise RuntimeError("receipt execution binding differs from the admitted mission")
        tool_ids = sorted({
            record.get("tool_id") for record in records if record.get("event") == "tool_call"
        })
        if any(not isinstance(tool_id, str) for tool_id in tool_ids) or summary.get("tool_names_from_receipts") != tool_ids:
            raise RuntimeError("receipt tool IDs differ from the model-run summary")
        finalizations = [record for record in records if record.get("event") == "host_finalization"]
        if (
            records[0].get("event") != "execution_started" or len(finalizations) != 1
            or records[-1] is not finalizations[0]
            or finalizations[0].get("outcome") != "candidate_ready"
        ):
            raise RuntimeError("receipt lacks a final candidate_ready host finalization")
        final_result = finalizations[0].get("result")
        if (
            not isinstance(final_result, dict)
            or final_result.get("changed_paths") != summary.get("changed_paths")
        ):
            raise RuntimeError("receipt finalization differs from the candidate summary")
        return receipt_sha
    except (KeyError, OSError, ValueError, TypeError, json.JSONDecodeError) as error:
        raise RuntimeError("host receipt could not be verified") from error


def write_json(path: Path, value: object) -> None:
    data = json.dumps(value, sort_keys=True, indent=2).encode() + b"\n"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def write_premerge_verifier_output(evidence: Path, output: bytes) -> dict[str, str]:
    if (
        evidence.is_symlink()
        or not evidence.is_dir()
        or not isinstance(output, bytes)
        or len(output) > MAX_VERIFIER_OUTPUT_BYTES
    ):
        raise RuntimeError("pre-merge verifier output or evidence directory is invalid")
    path = evidence / PREMERGE_VERIFIER_OUTPUT_FILE
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    except OSError:
        raise RuntimeError("pre-merge verifier output destination is unavailable") from None
    with os.fdopen(fd, "wb") as stream:
        stream.write(output)
        stream.flush()
        os.fsync(stream.fileno())
    return {"output_file": path.name, "output_sha256": digest(output)}


def _verified_admission_provenance(admission_evidence_dir: Path) -> dict[str, object]:
    if (
        admission_evidence_dir.is_symlink()
        or not admission_evidence_dir.is_dir()
        or admission_evidence_dir.stat().st_uid != os.geteuid()
        or stat.S_IMODE(admission_evidence_dir.stat().st_mode) != 0o700
    ):
        raise RuntimeError("admission evidence directory is not a private owner-owned directory")
    manifest_path = admission_evidence_dir / "SHA256SUMS"
    provenance_path = admission_evidence_dir / "provenance.json"
    for path in (manifest_path, provenance_path):
        metadata = path.lstat()
        if (
            path.is_symlink() or not path.is_file() or metadata.st_uid != os.geteuid()
            or metadata.st_nlink != 1 or stat.S_IMODE(metadata.st_mode) != 0o600
        ):
            raise RuntimeError("admission build provenance files are not private regular files")
    manifest = manifest_path.read_text(encoding="ascii")
    entries = {}
    for line in manifest.splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9._-]{1,128})", line)
        if match is None or match.group(2) in entries:
            raise RuntimeError("admission build manifest is malformed")
        entries[match.group(2)] = match.group(1)
    if entries.get("provenance.json") != digest(provenance_path.read_bytes()):
        raise RuntimeError("admission build provenance checksum does not match its manifest")
    provenance = json.loads(provenance_path.read_bytes())
    if not isinstance(provenance, dict):
        raise RuntimeError("admission build provenance is not an object")
    return provenance


def verified_webhook_receiver_sha256(admission_evidence_dir: Path) -> str:
    provenance = _verified_admission_provenance(admission_evidence_dir)
    executable_hashes = provenance.get("built_executable_sha256") if isinstance(provenance, dict) else None
    receiver_sha256 = executable_hashes.get("forgejo-webhook-local") if isinstance(executable_hashes, dict) else None
    if not isinstance(receiver_sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", receiver_sha256):
        raise RuntimeError("admission provenance omitted the Forgejo webhook receiver hash")
    return receiver_sha256


def verified_mcp_host_binary(
    state: Path, admission_evidence_dir: Path,
) -> tuple[Path, str]:
    provenance = _verified_admission_provenance(admission_evidence_dir)
    executable_hashes = provenance.get("built_executable_sha256")
    expected = executable_hashes.get("mission-012-mcp-host") if isinstance(executable_hashes, dict) else None
    binary = state / "build/host-artifacts/mission-012-mcp-host"
    if (
        provenance.get("mcp_host_built") is not True
        or provenance.get("private_state_dir") != str(state)
        or not isinstance(expected, str)
        or not re.fullmatch(r"[0-9a-f]{64}", expected)
        or binary.is_symlink() or not binary.is_file()
        or binary.stat().st_uid != os.geteuid()
        or binary.stat().st_nlink != 1
        or stat.S_IMODE(binary.stat().st_mode) != 0o700
        or digest(binary.read_bytes()) != expected
    ):
        raise RuntimeError("mission CPU verifier binary is missing or differs from admission build provenance")
    return binary, expected


def start_pr_webhook_observer(
    owner: "Api",
    state: Path,
    evidence: Path,
    expected_receiver_sha256: str,
) -> dict[str, object]:
    receiver_binary = state / "build/artifacts/forgejo-webhook-local"
    metadata = receiver_binary.lstat()
    if (
        receiver_binary.is_symlink() or not receiver_binary.is_file()
        or metadata.st_uid != os.geteuid() or metadata.st_nlink != 1
        or stat.S_IMODE(metadata.st_mode) != 0o700
        or digest(receiver_binary.read_bytes()) != expected_receiver_sha256
    ):
        raise RuntimeError("webhook receiver binary differs from the isolated admission build provenance")
    secret_path = state / f"pr-webhook-{evidence.name}.secret"
    journal_path = state / f"pr-webhook-{evidence.name}.jsonl"
    log_path = state / f"pr-webhook-{evidence.name}.log"
    if any(path.exists() or path.is_symlink() for path in (secret_path, journal_path, log_path)):
        raise RuntimeError("PR webhook observer state destinations must be new")
    secret = os.urandom(32).hex().encode("ascii")
    write_private_file(secret_path, secret)
    receiver_log = log_path.open("xb")
    os.chmod(log_path, 0o600)
    receiver_port = webhook_helpers.select_receiver_port()
    receiver_address = f"127.0.0.1:{receiver_port}"
    expected_target = f"http://{receiver_address}/forgejo/webhook"
    observer: dict[str, object] = {
        "receiver_binary": receiver_binary,
        "receiver_binary_sha256": expected_receiver_sha256,
        "secret_path": secret_path,
        "journal_path": journal_path,
        "receiver_port": receiver_port,
        "expected_target": expected_target,
        "receiver_log": receiver_log,
        "receiver_process": None,
        "hook_id": None,
        "hook_deactivated": False,
    }
    try:
        receiver = subprocess.Popen(
            [str(receiver_binary), receiver_address, str(secret_path), str(journal_path)],
            cwd=state,
            env=webhook_helpers.private_command_environment(state),
            stdin=subprocess.DEVNULL,
            stdout=receiver_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
            start_new_session=True,
        )
        observer["receiver_process"] = receiver
        webhook_helpers.wait_for_listener(receiver, receiver_port)
        _, created_hook, _ = owner.expect(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/hooks",
            {
                "type": "gitea",
                "config": {
                    "url": expected_target,
                    "content_type": "json",
                    "secret": secret.decode("ascii"),
                },
                "events": ["pull_request"],
                "active": True,
            },
        )
        hook_id = created_hook.get("id") if isinstance(created_hook, dict) else None
        if not isinstance(hook_id, int) or isinstance(hook_id, bool) or hook_id < 1:
            raise RuntimeError("Forgejo did not return the PR webhook ID")
        observer["hook_id"] = hook_id
        _, hook_api_readback, _ = owner.expect(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
        )
        initial_hook = webhook_events.hook_readback_record(
            hook_id, hook_api_readback, expected_target, ("pull_request",)
        )
        if (
            initial_hook.get("configuration_matches_expected") is not True
            or initial_hook.get("active_after_run") is not True
            or initial_hook.get("secret_value_recorded") is not False
        ):
            raise RuntimeError("Forgejo's active PR webhook readback differs from the expected loopback hook")
        observer["initial_hook_readback"] = initial_hook
        return observer
    except Exception:
        cleanup_pr_webhook_observer(owner, observer)
        raise


def deactivate_pr_webhook_observer(
    owner: "Api", observer: dict[str, object]
) -> dict[str, object]:
    hook_id = observer.get("hook_id")
    expected_target = observer.get("expected_target")
    if not isinstance(hook_id, int) or isinstance(hook_id, bool) or hook_id < 1:
        raise RuntimeError("PR webhook observer has no valid Forgejo hook ID")
    if not isinstance(expected_target, str):
        raise RuntimeError("PR webhook observer has no expected loopback URL")
    hook_path = f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
    patch = {"active": False, "events": ["pull_request"]}
    owner.expect("PATCH", hook_path, patch)
    _, hook_api_readback, _ = owner.expect("GET", hook_path)
    readback = webhook_events.hook_readback_record(
        hook_id, hook_api_readback, expected_target, ("pull_request",)
    )
    if (
        readback.get("configuration_matches_expected") is not True
        or readback.get("active_after_run") is not False
        or readback.get("deactivation_readback_confirmed") is not True
        or readback.get("secret_value_recorded") is not False
    ):
        raise RuntimeError("Forgejo did not confirm the PR webhook configuration and requested active state")
    observer["hook_deactivated"] = True
    observer["hook_readback"] = readback
    return readback

def confirm_open_pr_webhook(
    observer: dict[str, object],
    *,
    pull_request_number: int,
    base_commit: str,
    candidate_commit: str,
) -> dict[str, object]:
    journal = observer.get("journal_path")
    hook_id = observer.get("hook_id")
    if not isinstance(journal, Path) or not isinstance(hook_id, int):
        raise RuntimeError("PR webhook observer state is incomplete before review")
    records = webhook_events.wait_for_journal_count(journal, 1)
    if len(records) != 1:
        raise RuntimeError("Forgejo produced an unexpected count of PR-open webhook events before review")
    opened = webhook_events.parse_live_pull_request_delivery_record(
        records[0], owner=OWNER, repository=REPOSITORY,
        pull_request_number=pull_request_number, expected_action="opened",
    )
    if (
        opened["payload_state"] != "open"
        or opened["payload_merged"] is not False
        or opened["head_sha"] != candidate_commit
        or opened["base_sha"] != base_commit
    ):
        raise RuntimeError("PR-open webhook does not describe the verified candidate before review")
    tasks = webhook_events.wait_for_hook_tasks(journal.parent / "data/forgejo.db", hook_id, 1)
    if len(tasks) != 1:
        raise RuntimeError("Forgejo recorded an unexpected count of PR webhook tasks before review")
    webhook_events.validate_hook_task_bindings(tasks, [opened])
    observer["opened_delivery_receipt"] = opened
    return opened


def cleanup_pr_webhook_observer(
    owner: "Api | None", observer: dict[str, object] | None
) -> dict[str, object]:
    if observer is None:
        return {"started": False, "hook_deactivated": True, "receiver_stopped": True, "receiver_listener_absent": True}
    cleanup_failure = None
    if owner is not None and observer.get("hook_id") is not None and observer.get("hook_deactivated") is not True:
        try:
            deactivate_pr_webhook_observer(owner, observer)
        except Exception as error:
            cleanup_failure = type(error).__name__
    process = observer.get("receiver_process")
    if isinstance(process, subprocess.Popen):
        receiver_stop = webhook_helpers.stop_process(process)
    else:
        receiver_stop = {"started": False, "stopped": True, "returncode": None}
    receiver_log = observer.get("receiver_log")
    if hasattr(receiver_log, "closed") and not receiver_log.closed:
        receiver_log.close()
    port = observer.get("receiver_port")
    listeners = webhook_helpers.listener_addresses_for_port(port) if isinstance(port, int) else []
    result = {
        "started": process is not None,
        "hook_deactivated": observer.get("hook_deactivated") is True,
        "receiver_stopped": receiver_stop.get("stopped") is True and receiver_stop.get("returncode") == 0,
        "receiver_listener_absent": listeners == [],
        "receiver_stop": receiver_stop,
        "receiver_listener_addresses_after_stop": listeners,
        "cleanup_failure": cleanup_failure,
    }
    observer["cleanup"] = result
    return result


def validate_pr_reconciliation_delivery_binding(
    event: dict[str, object], result: dict[str, object]
) -> None:
    requested_delivery_id = event.get("delivery_id")
    returned_delivery_id = result.get("delivery_id")
    if (
        not isinstance(requested_delivery_id, str)
        or not isinstance(returned_delivery_id, str)
        or returned_delivery_id != requested_delivery_id
    ):
        raise RuntimeError("PR reconciliation result is not bound to the requested delivery")


def finish_pr_webhook_observer(
    owner: "Api",
    observer: dict[str, object],
    forgejo_binary: Path,
    reconcile_binary: Path,
    pull_request_number: int,
    base_commit: str,
    candidate_commit: str,
    merge_commit: str,
    merged_pull_request: dict[str, object],
    evidence: Path,
) -> dict[str, object]:
    journal = observer.get("journal_path")
    hook_id = observer.get("hook_id")
    receiver_binary = observer.get("receiver_binary")
    if (
        not isinstance(journal, Path) or not isinstance(hook_id, int)
        or not isinstance(receiver_binary, Path)
    ):
        raise RuntimeError("PR webhook observer state is incomplete")
    records = webhook_events.wait_for_journal_count(journal, 3)
    if len(records) != 3:
        raise RuntimeError("Forgejo emitted an unexpected number of PR webhook events")
    parsed_events = []
    for record in records:
        envelope = record.get("payload") if isinstance(record, dict) else None
        event_name = envelope.get("event") if isinstance(envelope, dict) else None
        payload = envelope.get("payload") if isinstance(envelope, dict) else None
        action = payload.get("action") if isinstance(payload, dict) else None
        parsed_events.append(webhook_events.parse_live_pull_request_delivery_record(
            record,
            owner=OWNER,
            repository=REPOSITORY,
            pull_request_number=pull_request_number,
            expected_action=action,
            expected_event=event_name,
        ))
    opened_matches = [event for event in parsed_events if event["event"] == "pull_request" and event["action"] == "opened"]
    review_matches = [event for event in parsed_events if event["event"] == "pull_request_approved"]
    closed_matches = [event for event in parsed_events if event["event"] == "pull_request" and event["action"] == "closed"]
    if len(opened_matches) != 1 or len(review_matches) != 1 or len(closed_matches) != 1:
        raise RuntimeError("Forgejo did not deliver exactly one opened, approved-review, and closed PR event")
    opened, approved_review, closed = opened_matches[0], review_matches[0], closed_matches[0]
    if (
        len({event["delivery_id"] for event in parsed_events}) != 3
        or len({event["sequence"] for event in parsed_events}) != 3
        or opened["payload_state"] != "open"
        or opened["payload_merged"] is not False
        or opened["head_sha"] != candidate_commit
        or opened["base_sha"] != base_commit
        or approved_review["action"] != "reviewed"
        or approved_review["review_type"] != "pull_request_review_approved"
        or approved_review["payload_state"] != "open"
        or approved_review["payload_merged"] is not False
        or approved_review["head_sha"] != candidate_commit
        or approved_review["base_sha"] != base_commit
        or closed["payload_state"] != "closed"
        or closed["payload_merged"] is not True
        or closed["head_sha"] != candidate_commit
        or closed["base_sha"] != merge_commit
    ):
        raise RuntimeError("Forgejo PR webhook payloads do not describe the expected PR and review events")
    tasks = webhook_events.wait_for_hook_tasks(journal.parent / "data/forgejo.db", hook_id, 3)
    if len(tasks) != 3:
        raise RuntimeError("Forgejo recorded an unexpected count of PR webhook tasks")
    webhook_events.validate_hook_task_bindings(tasks, parsed_events)
    final_head = merged_pull_request.get("head", {}).get("sha")
    final_base = merged_pull_request.get("base", {}).get("sha")
    if (
        merged_pull_request.get("state") != "closed"
        or merged_pull_request.get("merged") is not True
        or merged_pull_request.get("merge_commit_sha") != merge_commit
        or final_head != candidate_commit
        or final_base != merge_commit
    ):
        raise RuntimeError("Forgejo API does not confirm the expected merged PR before reconciliation")
    reader_token = webhook_helpers.create_access_token(
        forgejo_binary,
        journal.parent,
        OWNER,
        f"mission012-pr-reconcile-{evidence.name}",
        "read:user,read:repository,read:issue",
    )
    reconciliation_results = []
    try:
        for event in parsed_events:
            status, result = webhook_helpers.run_reconcile(
                reconcile_binary,
                journal.parent,
                journal,
                event["delivery_id"],
                reader_token,
                target_profile="mission-012-clamp",
            )
            reconciliation_results.append((status, result))
    finally:
        reader_token = ""
    reconciliation_receipts = []
    for event, (status, result) in zip(parsed_events, reconciliation_results, strict=True):
        validate_pr_reconciliation_delivery_binding(event, result)
        resource = result.get("resource", {})
        canonical = result.get("canonical_state", {})
        if (
            status != 0
            or result.get("verdict") != "current_state_read"
            or result.get("upstream_read_attempts") != 1
            or resource.get("kind") != "pull_request"
            or resource.get("owner") != OWNER
            or resource.get("repository") != REPOSITORY
            or resource.get("number") != pull_request_number
            or canonical.get("state") != "closed"
            or canonical.get("merged") is not True
            or canonical.get("head_sha") != final_head
            or canonical.get("base_sha") != final_base
        ):
            raise RuntimeError("PR webhook reconciliation did not return current Forgejo merged state")
        reconciliation_receipts.append({
            "delivery_id": event["delivery_id"],
            "exit_code": status,
            "verdict": result["verdict"],
            "upstream_read_attempts": result["upstream_read_attempts"],
            "resource": {
                "kind": resource["kind"],
                "owner": resource["owner"],
                "repository": resource["repository"],
                "number": resource["number"],
            },
            "canonical_state": {
                "state": canonical["state"],
                "merged": canonical["merged"],
                "head_sha": canonical["head_sha"],
                "base_sha": canonical["base_sha"],
            },
        })
    hook_readback = deactivate_pr_webhook_observer(owner, observer)
    cleanup = cleanup_pr_webhook_observer(owner, observer)
    if (
        cleanup.get("cleanup_failure") is not None
        or cleanup.get("hook_deactivated") is not True
        or cleanup.get("receiver_stopped") is not True
        or cleanup.get("receiver_listener_absent") is not True
    ):
        raise RuntimeError("PR webhook observer cleanup was not confirmed")
    result = {
        "schema_version": 4,
        "status": "verified_current_merged_pr",
        "target_profile": "mission-012-clamp",
        "pull_request_number": pull_request_number,
        "candidate_commit": candidate_commit,
        "merge_commit": merge_commit,
        "hook_id": hook_id,
        "receiver_binary_sha256": digest(receiver_binary.read_bytes()),
        "event_count": len(records),
        "forgejo_task_count": len(tasks),
        "successful_http_202_count": sum(
            1 for task in tasks if task.get("response_status") == 202
        ),
        "opened_delivery_id": opened["delivery_id"],
        "approved_review_delivery_id": approved_review["delivery_id"],
        "closed_delivery_id": closed["delivery_id"],
        "opened_sequence": opened["sequence"],
        "approved_review_sequence": approved_review["sequence"],
        "closed_sequence": closed["sequence"],
        "opened_payload": {
            "state": opened["payload_state"], "merged": opened["payload_merged"],
            "head_sha": opened["head_sha"], "base_sha": opened["base_sha"],
        },
        "closed_payload": {
            "state": closed["payload_state"], "merged": closed["payload_merged"],
            "head_sha": closed["head_sha"], "base_sha": closed["base_sha"],
        },
        "approved_review_notification": {
            "event": approved_review["event"],
            "action": approved_review["action"],
            "review_type": approved_review["review_type"],
            "pull_request_number": approved_review["pull_request_number"],
            "head_sha": approved_review["head_sha"],
            "base_sha": approved_review["base_sha"],
            "payload_state": approved_review["payload_state"],
            "payload_merged": approved_review["payload_merged"],
        },
        "reconciled_delivery_ids": [
            opened["delivery_id"], approved_review["delivery_id"], closed["delivery_id"],
        ],
        "reconciliation_receipts": reconciliation_receipts,
        "all_notifications_reconciled_to_current_state": True,
        "review_notification_is_approval_proof": False,
        "canonical_state": {
            "state": "closed", "merged": True,
            "head_sha": final_head, "base_sha": final_base,
        },
        "hook_readback": hook_readback,
        "cleanup": cleanup,
    }
    write_json(evidence / "pr-webhook-reconciliation.json", result)
    return result


class Api:
    def __init__(self, token: str):
        self.token = token
        self.opener = build_opener(ProxyHandler({}), NoRedirect)

    def request(self, method: str, path: str, value: object | None = None):
        if not path.startswith("/") or path.startswith("//"):
            raise RuntimeError("Forgejo API path escaped the pinned instance")
        headers = {"Accept": "application/json", "Authorization": f"token {self.token}"}
        body = None
        if value is not None:
            headers["Content-Type"] = "application/json"
            body = json.dumps(value, separators=(",", ":")).encode()
        req = Request(ORIGIN + "/api/v1" + path, data=body, headers=headers, method=method)
        try:
            with self.opener.open(req, timeout=8) as response:
                raw = response.read(1024 * 1024 + 1)
                if len(raw) > 1024 * 1024:
                    raise RuntimeError("Forgejo API response exceeded 1 MiB")
                return response.status, json.loads(raw) if raw else None, dict(response.headers.items())
        except HTTPError as error:
            raw = error.read(4096)
            try:
                parsed = json.loads(raw)
            except json.JSONDecodeError:
                parsed = {}
            message = parsed.get("message", "") if isinstance(parsed, dict) else ""
            return error.code, {"message": message} if isinstance(message, str) else {}, dict(error.headers.items())
        except URLError:
            raise RuntimeError(f"Forgejo API transport failed for {path}") from None

    def expect(self, method: str, path: str, value: object | None = None):
        status, body, headers = self.request(method, path, value)
        if status < 200 or status >= 300:
            message = body.get("message", "") if isinstance(body, dict) else ""
            raise RuntimeError(f"Forgejo API refused {method} {path}: HTTP {status} {message[:200]}")
        return status, body, headers


def expect_body(api: Api, method: str, path: str, value: object | None = None):
    status, body, _ = api.expect(method, path, value)
    return status, body


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def advance_main_with_approved_competing_pr(
    owner: Api, reviewer: Api, expected_base: str, evidence_dir: Path,
) -> dict[str, object]:
    """Advance main through an ordinary reviewed PR for the stale-base test mode."""
    if not re.fullmatch(r"[0-9a-f]{40}", expected_base):
        raise RuntimeError("concurrent base-advance fixture requires the admitted full commit")
    repo_path = f"/repos/{OWNER}/{REPOSITORY}"
    suffix = evidence_dir.name[-3:]
    branch = f"concurrent-main-advance-{suffix}"
    owner.expect("POST", repo_path + "/branches", {
        "new_branch_name": branch,
        "old_branch_name": "main",
    })
    _, readme, _ = owner.expect("GET", f"{repo_path}/contents/README.md?ref={branch}")
    original_readme = base64.b64decode(readme["content"], validate=True)
    marker = f"Concurrent base-advance fixture {suffix}.".encode()
    if marker in original_readme:
        raise RuntimeError("concurrent base-advance marker unexpectedly already exists")
    updated_readme = original_readme.rstrip(b"\n") + b"\n\n" + marker + b"\n"
    owner.expect("PUT", repo_path + "/contents/README.md", {
        "message": "Exercise stale-base merge refusal in the isolated fixture",
        "content": base64.b64encode(updated_readme).decode("ascii"),
        "branch": branch,
        "sha": readme["sha"],
    })
    _, competitor_branch, _ = owner.expect("GET", f"{repo_path}/branches/{branch}")
    competitor_head = competitor_branch.get("commit", {}).get("id")
    if not isinstance(competitor_head, str) or not re.fullmatch(r"[0-9a-f]{40}", competitor_head):
        raise RuntimeError("Forgejo did not return the concurrent branch head")
    _, competitor_pr, _ = owner.expect("POST", repo_path + "/pulls", {
        "title": "Advance main for stale-base refusal test",
        "body": "Local fixture only: move main after the MISSION-012 candidate was verified.",
        "head": branch,
        "base": "main",
    })
    competitor_number = competitor_pr.get("number")
    if isinstance(competitor_number, bool) or not isinstance(competitor_number, int) or competitor_number <= 0:
        raise RuntimeError("Forgejo did not return a valid competing pull request number")
    competitor_path = f"{repo_path}/pulls/{competitor_number}"
    competitor_before: dict[str, object] = {}
    for _ in range(60):
        _, competitor_before, _ = owner.expect("GET", competitor_path)
        if competitor_before.get("mergeable") is True:
            break
        time.sleep(0.25)
    if (
        competitor_before.get("mergeable") is not True
        or competitor_before.get("state") != "open"
        or competitor_before.get("base", {}).get("sha") != expected_base
        or competitor_before.get("head", {}).get("sha") != competitor_head
    ):
        raise RuntimeError("competing PR does not bind the reviewed base-advance candidate")
    _, review, _ = reviewer.expect("POST", competitor_path + "/reviews", {
        "body": "Independent fixture review of the README-only base advance.",
        "event": "APPROVED",
        "commit_id": competitor_head,
    })
    if review.get("state") != "APPROVED" or review.get("commit_id") != competitor_head:
        raise RuntimeError("Forgejo did not bind the competing approval to its head")
    merge_status, merge_response, _ = owner.request(
        "POST", competitor_path + "/merge", {"Do": "merge", "head_commit_id": competitor_head}
    )
    if merge_status != 200:
        response_message = merge_response.get("message", "") if isinstance(merge_response, dict) else ""
        safe_message = response_message[:200] if isinstance(response_message, str) else ""
        raise RuntimeError(
            f"Forgejo refused the reviewed local base-advance PR: HTTP {merge_status} {safe_message}"
        )
    _, main_after, _ = owner.expect("GET", f"{repo_path}/branches/main")
    main_after_sha = main_after.get("commit", {}).get("id")
    _, competitor_after, _ = owner.expect("GET", competitor_path)
    _, main_readme, _ = owner.expect("GET", f"{repo_path}/contents/README.md?ref=main")
    merged_readme = base64.b64decode(main_readme["content"], validate=True)
    if (
        not isinstance(main_after_sha, str)
        or not re.fullmatch(r"[0-9a-f]{40}", main_after_sha)
        or main_after_sha == expected_base
        or competitor_after.get("state") != "closed"
        or competitor_after.get("merged") is not True
        or competitor_after.get("merge_commit_sha") != main_after_sha
        or marker not in merged_readme
    ):
        raise RuntimeError("Forgejo read-back did not prove the approved competing PR advanced main")
    result = {
        "schema_version": 1,
        "base_before": expected_base,
        "competing_pull_request": competitor_number,
        "competing_head": competitor_head,
        "review_id": review.get("id"),
        "reviewer_login": review.get("user", {}).get("login"),
        "merge_http_status": merge_status,
        "merged": competitor_after["merged"],
        "merge_commit_sha": competitor_after["merge_commit_sha"],
        "main_after": main_after_sha,
        "readme_marker_sha256": digest(marker),
    }
    write_json(evidence_dir / "concurrent-base-advance.json", result)
    return result


def admin(binary: Path, state: Path, *args: str) -> bytes:
    result = subprocess.run(
        [str(binary), "--work-path", str(state), "--config", str(state / "custom/conf/app.ini"), *args],
        env={"PATH": "/usr/bin:/bin", "HOME": str(state / "home")},
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=20, check=False,
    )
    if result.returncode:
        # Forgejo admin output may contain a newly generated credential.
        raise RuntimeError(f"Forgejo administrative command failed with exit {result.returncode}")
    return result.stdout


def wait_health(process: subprocess.Popen[bytes]) -> None:
    opener = build_opener(ProxyHandler({}), NoRedirect)
    end = time.monotonic() + 30
    while time.monotonic() < end:
        if process.poll() is not None:
            raise RuntimeError(f"private Forgejo exited during startup ({process.returncode})")
        try:
            with opener.open(ORIGIN + "/api/healthz", timeout=1) as response:
                if json.loads(response.read(16385)).get("status") == "pass":
                    return
        except Exception:
            time.sleep(0.2)
    raise RuntimeError("private Forgejo health check timed out")


def verify_listener(process: subprocess.Popen[bytes]) -> None:
    result = subprocess.run(
        ["/usr/bin/ss", "-H", "-ltnp", "sport = :3132"],
        env={"PATH": "/usr/bin:/bin"}, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=5,
    )
    if result.returncode or not listener_output_is_loopback(result.stdout, process.pid):
        raise RuntimeError("loopback Forgejo listener is not owned by this test process")


def git(args: list[str], *, repo: Path, timeout: int = 30) -> bytes:
    result = subprocess.run(
        ["/usr/bin/git", "-C", str(repo), *args],
        env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent",
             "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
             "GIT_TERMINAL_PROMPT": "0"},
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=timeout, check=False,
    )
    if result.returncode:
        raise RuntimeError(f"bounded local Git operation refused (exit {result.returncode})")
    return result.stdout


def materialize(repo: Path, commit: str, target: Path) -> dict[str, bytes]:
    archive = subprocess.run(
        ["/usr/bin/git", "-C", str(repo), "archive", "--format=tar", commit],
        env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent",
             "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"},
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=20, check=False,
    )
    if archive.returncode or len(archive.stdout) > 2 * 1024 * 1024:
        raise RuntimeError("could not read a bounded archive from the Forgejo commit")
    target.mkdir(mode=0o700)
    result: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(archive.stdout), mode="r:") as tar:
        members = tar.getmembers()
        if any(not (item.isfile() or item.isdir()) or item.issym() or item.islnk() for item in members):
            raise RuntimeError("Forgejo commit contains a link or special file")
        files = {item.name for item in members if item.isfile()}
        dirs = {item.name for item in members if item.isdir()}
        if files != EXPECTED_FILES or dirs != {"requirements", "src", "tests"}:
            raise RuntimeError("Forgejo commit tree differs from the bounded MISSION-012 fixture")
        if any(item.size > 1024 * 1024 for item in members):
            raise RuntimeError("Forgejo commit contains an oversized source file")
        tar.extractall(target, filter="data")
    for name in sorted(EXPECTED_FILES):
        result[name] = (target / name).read_bytes()
    return result


def run(args: argparse.Namespace) -> dict[str, object]:
    state_argument = Path(args.state_dir)
    if state_argument.is_symlink():
        raise RuntimeError("state directory cannot be a symbolic link")
    state = Path(args.state_dir).resolve(strict=True)
    evidence = Path(args.evidence_dir).resolve()
    binary = Path(args.forgejo_binary).resolve(strict=True)
    observe_pr_webhooks = getattr(args, "observe_pr_webhooks", False)
    expected_receiver_sha256 = None
    admission_evidence_dir = None
    admission_evidence_argument = getattr(args, "admission_evidence_dir", None)
    if admission_evidence_argument is not None:
        admission_evidence_dir = Path(admission_evidence_argument).resolve(strict=True)
        expected_admission_evidence = evidence.parent / f"{evidence.name}-admission"
        if admission_evidence_dir != expected_admission_evidence:
            raise RuntimeError("PR stage is not bound to this composed run's admission evidence")
        if observe_pr_webhooks:
            expected_receiver_sha256 = verified_webhook_receiver_sha256(admission_evidence_dir)
    elif observe_pr_webhooks:
        raise RuntimeError("PR webhook observation requires the admission build evidence directory")
    if not re.fullmatch(r"live-mcp-[0-9]{3}", args.execution_id):
        raise RuntimeError("execution ID must match the private live-mcp-NNN format")
    candidate_root = state / "project/.runtime" / args.execution_id
    summary_path = candidate_root / "agent-run-summary.json"
    try:
        summary_metadata = summary_path.lstat()
    except OSError:
        raise RuntimeError("agent run summary is unavailable") from None
    if (
        summary_path.is_symlink()
        or not summary_path.is_file()
        or summary_metadata.st_uid != os.geteuid()
        or summary_metadata.st_nlink != 1
        or stat.S_IMODE(summary_metadata.st_mode) != 0o600
    ):
        raise RuntimeError("agent run summary must be a private regular file")
    summary = json.loads(summary_path.read_bytes())
    if not isinstance(summary, dict):
        raise RuntimeError("agent run summary must be a JSON object")
    validate_model_cost(summary, args.budget_cap_usd)
    if admission_evidence_dir is None:
        raise RuntimeError("journaled PR verification requires the admission build evidence directory")
    verifier_binary, verifier_binary_sha256 = verified_mcp_host_binary(state, admission_evidence_dir)
    verifier_lease_id = f"mission012-{args.execution_id}"
    admission_file = Path(args.admission_file) if args.admission_file else state / "project/.runtime/host-admission.json"
    if admission_file.is_symlink() or not admission_file.is_absolute() or admission_file.parent != state / "project/.runtime":
        raise RuntimeError("admission file must be a direct private .runtime child")
    admission_bytes = admission_file.read_bytes()
    admission_sha256 = digest(admission_bytes)
    admission = json.loads(admission_bytes)
    admission_allocation = admission.get("allocation")
    mission_id = (
        admission_allocation.get("mission_id")
        if isinstance(admission_allocation, dict) else None
    )
    if not isinstance(mission_id, str) or re.fullmatch(r"mission-v1-[0-9a-f]{64}", mission_id) is None:
        raise RuntimeError("admission omitted a valid mission identity")
    base_commit = admission.get("base_commit")
    mission_journal = state / "project/.runtime/mission-execution.jsonl"
    generation = admission.get("allocation", {}).get("execution_generation")
    token_file = state / "project/.runtime/forgejo-read.token"
    git_repo = state / "repositories/forge-admin/mission-012-clamp.git"
    trusted_repo = candidate_root / "trusted-repository"
    candidate_source_path = candidate_root / "workspace/candidate/src/lib.rs"
    candidate_acceptance_path = candidate_root / "workspace/candidate/tests/acceptance.rs"
    if candidate_source_path.is_symlink() or candidate_acceptance_path.is_symlink():
        raise RuntimeError("admitted source files cannot be symbolic links")
    candidate_source = candidate_source_path.read_bytes()
    candidate_acceptance = candidate_acceptance_path.read_bytes()
    source_sha = digest(candidate_source)
    branch = f"agent/{evidence.name}"
    local_ref = f"refs/mission012/{evidence.name}"
    expected_evidence_parent = (Path(__file__).resolve().parents[1] / "evidence").resolve(strict=True)
    if (
        state.parent != Path("/tmp") or state.stat().st_mode & 0o777 != 0o700
        or candidate_root.is_symlink() or not candidate_root.is_dir()
        or binary.is_symlink() or digest(binary.read_bytes()) != FORGEJO_SHA
        or summary.get("status") != "candidate_ready_for_independent_verification"
        or not isinstance(generation, int) or isinstance(generation, bool) or generation < 1
        or not isinstance(base_commit, str) or not re.fullmatch(r"[0-9a-f]{40}", base_commit)
        or summary.get("execution_generation") != generation
        or summary.get("base_commit") != base_commit
        or summary.get("admission_sha256") != digest(admission_bytes)
        or summary.get("candidate_file_sha256") != {"src/lib.rs": source_sha}
        or summary.get("changed_paths") != ["src/lib.rs"]
        or admission_file.stat().st_mode & 0o777 != 0o600
        or admission_file.stat().st_nlink != 1
        or mission_journal.is_symlink() or not mission_journal.is_file()
        or mission_journal.stat().st_uid != os.geteuid()
        or mission_journal.stat().st_mode & 0o777 != 0o600
        or mission_journal.stat().st_nlink != 1
        or token_file.is_symlink() or token_file.stat().st_mode & 0o777 != 0o600
        or not git_repo.is_dir() or git_repo.is_symlink()
        or not trusted_repo.is_dir() or trusted_repo.is_symlink()
        or evidence.parent != expected_evidence_parent
        or not re.fullmatch(r"mission-012-live-agent-[a-z0-9-]{1,48}", evidence.name)
    ):
        raise RuntimeError("the candidate or private Forgejo fixture failed pinned preflight")
    verify_pinned_sources(candidate_source, candidate_acceptance)
    host_receipt_sha = verify_host_receipt(
        summary, candidate_root, execution_binding_from_admission(admission)
    )
    if evidence.exists() or evidence.is_symlink():
        raise RuntimeError("evidence destination must be new")
    merge_tools = build_rust_merge_gate_tools(state)
    evidence.mkdir(parents=True, mode=0o755)
    write_json(evidence / "rust-merge-authority-build.json", merge_tools["record"])
    private_state = state
    server_log = (private_state / f"forgejo-agent-pr-e2e-{evidence.name}.log").open("xb")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3132)) == 0:
            raise RuntimeError("127.0.0.1:3132 is already occupied")
    process = subprocess.Popen(
        [str(binary), "--work-path", str(state), "--config", str(state / "custom/conf/app.ini"), "web"],
        cwd=state, env={"PATH": "/usr/bin:/bin", "HOME": str(state / "home"), "TMPDIR": str(state / "tmp")},
        stdin=subprocess.DEVNULL, stdout=server_log, stderr=subprocess.STDOUT,
    )
    owner: Api | None = None
    webhook_observer: dict[str, object] | None = None
    try:
        wait_health(process)
        verify_listener(process)
        owner_token = admin(
            binary, state, "admin", "user", "generate-access-token", "--username", OWNER,
            "--token-name", f"mission012-live-agent-pr-e2e-{evidence.name}-owner", "--raw", "--scopes",
            ",".join(MERGE_GATE_OWNER_SCOPES),
        ).decode().strip()
        reviewer_token = admin(
            binary, state, "admin", "user", "generate-access-token", "--username", REVIEWER,
            "--token-name", f"mission012-live-agent-pr-e2e-{evidence.name}-reviewer", "--raw", "--scopes", "write:repository",
        ).decode().strip()
        if not owner_token or any(c.isspace() for c in owner_token) or not reviewer_token or any(c.isspace() for c in reviewer_token):
            raise RuntimeError("Forgejo did not issue bounded local test credentials")
        owner = Api(owner_token)
        reviewer = Api(reviewer_token)
        owner_token = reviewer_token = ""
        _, repo_info, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}")
        if repo_info.get("private") is not True:
            raise RuntimeError("the test repository is not private")
        _, issue, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/issues/1")
        if issue.get("state") != "open":
            raise RuntimeError("the MISSION-012 issue is not open before PR creation")
        _, main, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
        base = main["commit"]["id"]
        if base != base_commit:
            raise RuntimeError("Forgejo main changed after the host admission")
        _, requirement_response, _ = owner.expect(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/contents/requirements/MISSION-012-CLAMP-001.json?ref=main"
        )
        requirement_bytes = base64.b64decode(requirement_response["content"])
        if digest(requirement_bytes) != REQUIREMENT_SHA:
            raise RuntimeError("Forgejo requirement bytes differ from the admitted revision")
        if digest((Path(__file__).resolve().parents[1] / "tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v4.json").read_bytes()) != POLICY_SHA:
            raise RuntimeError("trusted policy fixture changed from the admitted MISSION-012 policy")

        _, protection_list, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/branch_protections")
        protections = [p for p in protection_list if p.get("rule_name") == "main"]
        if len(protections) != 1:
            raise RuntimeError("Forgejo did not return exactly one main protection rule")
        protection = protections[0]
        if not branch_protection_matches_policy(protection):
            raise RuntimeError("Forgejo main protection differs from reviewer and merger policy")

        if observe_pr_webhooks and not getattr(args, "inject_concurrent_base_advance", False):
            if expected_receiver_sha256 is None:
                raise RuntimeError("PR webhook receiver provenance was not verified")
            webhook_observer = start_pr_webhook_observer(
                owner, state, evidence, expected_receiver_sha256
            )

        owner.expect("POST", f"/repos/{OWNER}/{REPOSITORY}/branches", {
            "new_branch_name": branch, "old_branch_name": "main",
        })
        encoded_branch = branch.replace("/", "%2F")
        _, initial_source_doc, _ = owner.expect(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref={encoded_branch}"
        )
        initial_source = base64.b64decode(initial_source_doc["content"])
        if digest(initial_source) != BROKEN_SOURCE_SHA:
            raise RuntimeError("new Forgejo branch does not point at the admitted broken source")
        owner.expect("PUT", f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs", {
            "message": "Fix upper clamp boundary from admitted MISSION-012 agent candidate",
            "content": base64.b64encode(candidate_source).decode("ascii"), "branch": branch,
            "sha": initial_source_doc["sha"],
        })
        _, candidate_readback, _ = owner.expect(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref={encoded_branch}"
        )
        if digest(base64.b64decode(candidate_readback["content"])) != source_sha:
            raise RuntimeError("Forgejo content read-back differs from the admitted agent candidate")
        _, branch_info, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{encoded_branch}")
        head = branch_info["commit"]["id"]
        pr_status, pr, _ = owner.expect("POST", f"/repos/{OWNER}/{REPOSITORY}/pulls", {
            "title": "MISSION-012: correct clamp upper boundary",
            "body": "Closes #1. Corrects the admitted upper-bound defect; only src/lib.rs changes.",
            "head": branch, "base": "main",
        })
        number = int(pr["number"])
        pr_path = f"/repos/{OWNER}/{REPOSITORY}/pulls/{number}"
        _, current_pr, _ = owner.expect("GET", pr_path)
        if (
            current_pr.get("state") != "open" or current_pr.get("base", {}).get("sha") != base
            or current_pr.get("head", {}).get("sha") != head
        ):
            raise RuntimeError("Forgejo PR read-back differs from the tested base and candidate")
        if webhook_observer is not None:
            confirm_open_pr_webhook(
                webhook_observer,
                pull_request_number=number,
                base_commit=base,
                candidate_commit=head,
            )
        _, files, file_headers = owner.expect("GET", pr_path + "/files?limit=50&page=1")
        changed_paths = sorted(item["filename"] for item in files)
        if changed_paths != ["src/lib.rs"] or int(file_headers.get("X-Total-Count", "1")) != 1:
            raise RuntimeError("Forgejo PR file list contains an unauthorized path")

        git(["fetch", "--no-tags", str(git_repo), f"refs/heads/{branch}:{local_ref}"], repo=trusted_repo)
        fetched_head = git(["rev-parse", local_ref], repo=trusted_repo).decode().strip()
        if fetched_head != head or git(["rev-parse", "refs/heads/main"], repo=trusted_repo).decode().strip() != base:
            raise RuntimeError("local read-only Forgejo fetch does not match the API PR commits")
        base_tree_dir = private_state / "verify-base-snapshot"
        candidate_tree_dir = private_state / "verify-candidate-snapshot"
        base_files = materialize(trusted_repo, base, base_tree_dir)
        candidate_files = materialize(trusted_repo, head, candidate_tree_dir)
        actual_changes = sorted(k for k in EXPECTED_FILES if base_files[k] != candidate_files[k])
        if actual_changes != ["src/lib.rs"] or digest(base_files["tests/acceptance.rs"]) != digest(candidate_files["tests/acceptance.rs"]):
            raise RuntimeError("candidate commit changed protected acceptance criteria")
        rust_sysroot = Path("/home/pc6/.rustup/toolchains/1.95.0-aarch64-unknown-linux-gnu").resolve(strict=True)
        verify_pinned_sources(base_files["src/lib.rs"], base_files["tests/acceptance.rs"], base=True)
        verify_pinned_sources(candidate_files["src/lib.rs"], candidate_files["tests/acceptance.rs"])
        base_reservation_id = verifier_reservation_id("base-red", base, head)
        red_measurement = verify_and_run_base(
            base_tree_dir,
            rust_sysroot,
            host_binary=verifier_binary,
            expected_host_sha256=verifier_binary_sha256,
            mission_journal=mission_journal,
            lease_id=verifier_lease_id,
            reservation_id=base_reservation_id,
            target_dir=private_state / "verifier-target-base-red",
        )
        red_status = red_measurement["exit_code"]
        red_output = red_measurement["output"]
        (evidence / "base-red-cargo-test.log").write_bytes(red_output)
        if (
            red_status == 0 or red_measurement["timed_out"] or red_measurement["output_truncated"]
            or b"values_outside_the_range_clamp_to_the_nearest_boundary" not in red_output
            or b"1 passed; 1 failed" not in red_output
        ):
            raise RuntimeError("independent base test did not show the expected actionable failure")
        cpu_runs = [verifier_cpu_run_evidence(
            "base-red", base, head, base, red_measurement,
            expected_mission_id=mission_id,
        )]
        green_reservation_id = verifier_reservation_id("candidate-green", base, head)
        green_measurement = verify_and_run_candidate(
            candidate_tree_dir,
            rust_sysroot,
            host_binary=verifier_binary,
            expected_host_sha256=verifier_binary_sha256,
            mission_journal=mission_journal,
            lease_id=verifier_lease_id,
            reservation_id=green_reservation_id,
            target_dir=private_state / "verifier-target-candidate-green",
        )
        green_status = green_measurement["exit_code"]
        green_output = green_measurement["output"]
        (evidence / "candidate-green-cargo-test.log").write_bytes(green_output)
        if (
            green_status or green_measurement["timed_out"] or green_measurement["output_truncated"]
            or b"2 passed; 0 failed" not in green_output
        ):
            raise RuntimeError("independent isolated Cargo verifier did not pass the admitted PR candidate")
        cpu_runs.append(verifier_cpu_run_evidence(
            "candidate-green", base, head, head, green_measurement,
            expected_mission_id=mission_id,
        ))
        verifier = {
            "schema_version": 1, "verdict": "pass",
            "verifier_id": MISSION_VERIFIER_ID,
            "source": "Forgejo API branch ref fetched into a host-owned trusted Git clone",
            "base_commit": base, "candidate_commit": head,
            "requirement_sha256": REQUIREMENT_SHA, "policy_sha256": POLICY_SHA,
            "acceptance_sha256": digest(candidate_files["tests/acceptance.rs"]),
            "candidate_source_sha256": digest(candidate_files["src/lib.rs"]),
            "changed_paths": changed_paths,
            "base": {"exit_code": red_status, "output_sha256": digest(red_output), "expected_failure": True},
            "candidate": {"exit_code": green_status, "output_sha256": digest(green_output), "acceptance_pass": True},
            "environment": "trusted Rust host; systemd CPU/memory/task/deadline scope; Bubblewrap user/mount/PID/network namespaces with nested user namespaces disabled; commit snapshot read-only; fresh tmpfs Cargo target; host home and credentials absent; offline locked Cargo",
            "toolchain": "/home/pc6/.rustup/toolchains/1.95.0-aarch64-unknown-linux-gnu/bin/cargo test --offline --locked",
            "limitation": "This minimal Forgejo fixture does not version policy/active.json or policy v4, so the verifier binds the pinned requirement and policy hashes from the host-owned admission fixture while testing the exact PR commits. CPU accounting is a conservative upper bound derived from the enforced cgroup quota and elapsed wall time, not direct cgroup CPU telemetry.",
        }
        refusal_status, refusal, _ = owner.request("POST", pr_path + "/merge", {"Do": "merge", "head_commit_id": head})
        refusal_message = refusal.get("message", "") if isinstance(refusal, dict) else ""
        if refusal_status != 405 or "approval" not in refusal_message.lower():
            raise RuntimeError("Forgejo did not refuse an owner merge before the required review")
        _, still_open, _ = owner.expect("GET", pr_path)
        if still_open.get("state") != "open" or still_open.get("merged") is not False:
            raise RuntimeError("pre-review merge refusal altered PR state")
        write_json(evidence / "pre-review-merge-refusal.json", {
            "http_status": refusal_status, "reason": refusal_message,
            "state_after_refusal": still_open.get("state"), "merged": still_open.get("merged"),
        })

        _, review_basis, _ = owner.expect("GET", pr_path)
        if review_basis.get("base", {}).get("sha") != base or review_basis.get("head", {}).get("sha") != head:
            raise RuntimeError("PR changed since verification; approval is refused")
        _, review, _ = reviewer.expect("POST", pr_path + "/reviews", {
            "body": f"Independent sandbox verifier passed for requirement {REQUIREMENT_SHA[:12]} and head {head}.",
            "event": "APPROVED", "commit_id": head,
        })
        if review.get("state") != "APPROVED" or review.get("commit_id") != head:
            raise RuntimeError("Forgejo did not bind the reviewer approval to the verified head")
        reviewer_merge_status, reviewer_merge, _ = reviewer.request(
            "POST", pr_path + "/merge", {"Do": "merge", "head_commit_id": head}
        )
        if reviewer_merge_status != 405 or "not allowed to merge" not in str(reviewer_merge.get("message", "")).lower():
            raise RuntimeError("Forgejo did not refuse the reviewer's non-whitelisted merge")
        _, after_reviewer_attempt, _ = owner.expect("GET", pr_path)
        if after_reviewer_attempt.get("state") != "open" or after_reviewer_attempt.get("merged") is not False:
            raise RuntimeError("reviewer merge refusal altered the PR")
        _, reviews, review_headers = owner.expect("GET", pr_path + "/reviews?limit=50&page=1")
        if int(review_headers.get("X-Total-Count", "-1")) != 1 or len(reviews) != 1:
            raise RuntimeError("Forgejo review read-back is incomplete or contains unexpected reviews")
        approval = reviews[0]
        if (
            approval.get("state") != "APPROVED" or approval.get("commit_id") != head
            or approval.get("stale") is not False or approval.get("dismissed") is not False
            or approval.get("official") is not True or approval.get("user", {}).get("login") != REVIEWER
        ):
            raise RuntimeError("Forgejo's current approval is not official, current, or from verifier-bot")
        _, premerge, _ = owner.expect("GET", pr_path)
        _, protections_now, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/branch_protections")
        if (
            premerge.get("state") != "open" or premerge.get("merged") is not False
            or premerge.get("base", {}).get("sha") != base or premerge.get("head", {}).get("sha") != head
            or protections_now != protection_list
        ):
            raise RuntimeError("PR or branch protection changed before merge")
        # Repeat the independent check immediately before the authority call.
        fresh_reservation_id = verifier_reservation_id("premerge-green", base, head)
        fresh_measurement = verify_and_run_candidate(
            candidate_tree_dir,
            rust_sysroot,
            host_binary=verifier_binary,
            expected_host_sha256=verifier_binary_sha256,
            mission_journal=mission_journal,
            lease_id=verifier_lease_id,
            reservation_id=fresh_reservation_id,
            target_dir=private_state / "verifier-target-premerge-green",
        )
        fresh_status = fresh_measurement["exit_code"]
        fresh_output = fresh_measurement["output"]
        if (
            fresh_status or fresh_measurement["timed_out"] or fresh_measurement["output_truncated"]
            or b"2 passed; 0 failed" not in fresh_output
        ):
            raise RuntimeError("pre-merge candidate re-verification failed")
        cpu_runs.append(verifier_cpu_run_evidence(
            "premerge-green", base, head, head, fresh_measurement,
            expected_mission_id=mission_id,
        ))
        cpu_ledger = validate_final_verifier_cpu_ledger(
            fresh_measurement.get("ledger"), cpu_runs,
            expected_mission_id=mission_id,
        )
        verifier["cpu_accounting"] = {
            "source": "mission execution journal; reservations and resolutions produced by the admission-provenance MCP host binary",
            "verifier_binary_sha256": verifier_binary_sha256,
            "policy_sha256": VERIFIER_CPU_POLICY_SHA256,
            "mission_cpu_budget_ms": MISSION_VERIFICATION_CPU_BUDGET_MS,
            "charged_cpu_measurement": "conservative upper bound from wall time multiplied by enforced CPUQuota, plus host margin, capped at reservation",
            "steps": cpu_runs,
            "final_ledger": cpu_ledger,
        }
        verifier_cpu_accounting = verifier["cpu_accounting"]
        premerge_output_receipt = write_premerge_verifier_output(evidence, fresh_output)
        write_json(evidence / "premerge-reverification.json", {
            "verdict": "pass", "candidate_commit": head,
            **premerge_output_receipt,
            "policy_sha256": POLICY_SHA, "requirement_sha256": REQUIREMENT_SHA,
            "cpu_accounting": cpu_runs[-1],
        })
        # Reduce the base/head TOCTOU window by reading all merge gates after
        # the fresh test run. Forgejo's ordinary merge API still has no atomic
        # expected-base argument, so this cannot exclude a non-cooperating writer.
        _, latest_pr = expect_body(owner, "GET", pr_path)
        _, latest_main = expect_body(owner, "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
        _, latest_protections = expect_body(owner, "GET", f"/repos/{OWNER}/{REPOSITORY}/branch_protections")
        _, latest_reviews, latest_review_headers = owner.expect(
            "GET", pr_path + "/reviews?limit=50&page=1"
        )
        latest_approvals = [
            item for item in latest_reviews
            if item.get("state") == "APPROVED" and item.get("commit_id") == head
            and item.get("stale") is False and item.get("dismissed") is False
            and item.get("official") is True and item.get("user", {}).get("login") == REVIEWER
        ]
        if (
            latest_pr.get("state") != "open" or latest_pr.get("merged") is not False
            or latest_pr.get("base", {}).get("sha") != base
            or latest_pr.get("head", {}).get("sha") != head
            or latest_main.get("commit", {}).get("id") != base
            or latest_protections != protection_list
            or int(latest_review_headers.get("X-Total-Count", "-1")) != 1
            or len(latest_reviews) != 1 or len(latest_approvals) != 1
        ):
            raise RuntimeError("merge base, head, branch policy, or approval changed after verification")
        reviewer_id = approval.get("user", {}).get("id")
        if isinstance(reviewer_id, bool) or not isinstance(reviewer_id, int) or reviewer_id <= 0:
            raise RuntimeError("Forgejo approval omitted a valid reviewer identity")
        gate_binary = merge_tools["gate_binary"]
        authority_binary = merge_tools["authority_binary"]
        if not isinstance(gate_binary, Path) or not isinstance(authority_binary, Path):
            raise RuntimeError("offline Rust merge-gate build did not return executable paths")
        merge_inputs = prepare_rust_merge_inputs(
            state=state,
            gate_binary=gate_binary,
            verifier_report=verifier,
            base_commit=base,
            candidate_commit=head,
            reviewer_id=reviewer_id,
        )
        verifier = merge_inputs["verifier_report"]
        if not isinstance(verifier, dict):
            raise RuntimeError("Rust merge proof did not retain its independent verifier report")
        write_json(evidence / "independent-verifier.json", verifier)
        review_binding_path = Path(merge_inputs["input_dir"]) / "review-base-binding.json"
        write_private_json(review_binding_path, {
            "schema_version": 1,
            "review_id": approval["id"],
            "reviewer_id": reviewer_id,
            "reviewer_login": REVIEWER,
            "pull_request_number": number,
            "base_branch": "main",
            "base_commit": base,
            "candidate_commit": head,
            "mission_admission_sha256": admission_sha256,
            "mission_source_issue_number": admission.get("issue_number"),
            "source": "Forgejo current official approval read back after independent verification of this exact candidate",
        })
        concurrent_advance = None
        if args.inject_concurrent_base_advance:
            concurrent_advance = advance_main_with_approved_competing_pr(
                owner, reviewer, base, evidence,
            )
        merge_gate_exit, merge_gate_result = run_rust_merge_gate(
            authority_binary,
            pull_request=number,
            proof=Path(merge_inputs["proof"]),
            expected=Path(merge_inputs["expected"]),
            key=Path(merge_inputs["key"]),
            policy=Path(merge_inputs["policy"]),
            review_base_binding=review_binding_path,
            admission_file=admission_file,
            mission_journal=mission_journal,
            owner_token=owner.token,
        )
        write_json(evidence / "rust-merge-authority-result.json", merge_gate_result)
        if args.inject_concurrent_base_advance:
            error = merge_gate_result.get("error")
            error_code = error.get("code") if isinstance(error, dict) else None
            if (
                concurrent_advance is None
                or merge_gate_exit == 0
                or merge_gate_result.get("verdict") != "rejected"
                or merge_gate_result.get("merge_authorized") is not False
                or merge_gate_result.get("merge_attempted") is not False
                or error_code not in {
                    "stale_base", "not_mergeable", "invalid_review_base_binding",
                }
            ):
                raise RuntimeError("Rust merge authority did not refuse the concurrent base advance before merge")
            _, target_after, _ = owner.expect("GET", pr_path)
            _, main_after, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
            _, issue_after, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/issues/1")
            _, main_source, _ = owner.expect(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=main"
            )
            main_after_sha = main_after.get("commit", {}).get("id")
            target_source_sha = digest(base64.b64decode(main_source["content"], validate=True))
            target_base_after_sha = target_after.get("base", {}).get("sha")
            if (
                target_after.get("state") != "open"
                or target_after.get("merged") is not False
                or target_base_after_sha not in {base, main_after_sha}
                or target_after.get("head", {}).get("sha") != head
                or main_after_sha != concurrent_advance["main_after"]
                or main_after_sha == base
                or target_source_sha != BROKEN_SOURCE_SHA
                or issue_after.get("state") != "open"
            ):
                raise RuntimeError("stale-base refusal read-back shows a target PR or issue was merged or changed")
            refusal_result = {
                "schema_version": 1,
                "status": "refused_after_concurrent_base_advance_before_merge",
                "pull_request": number,
                "base_commit": base,
                "requirement_sha256": REQUIREMENT_SHA,
                "policy_sha256": POLICY_SHA,
                "candidate_commit": head,
                "candidate_sha256": source_sha,
                "changed_paths": changed_paths,
                "verifier": verifier["verdict"],
                "verifier_cpu_accounting": verifier_cpu_accounting,
                "reviewer": REVIEWER,
                "review_id": approval["id"],
                "pre_review_merge_refusal_http": refusal_status,
                "reviewer_merge_refusal_http": reviewer_merge_status,
                "issue_state": issue_after["state"],
                "target_pull_request_state": target_after["state"],
                "target_pull_request_merged": target_after["merged"],
                "target_pull_request_base_sha_after_advance": target_base_after_sha,
                "target_merge_commit": None,
                "main_head_before_advance": base,
                "main_head_after_advance": main_after_sha,
                "main_source_sha256_after_refusal": target_source_sha,
                "competing_pull_request": concurrent_advance["competing_pull_request"],
                "competing_pull_request_merged": concurrent_advance["merged"],
                "competing_merge_http_status": concurrent_advance["merge_http_status"],
                "merge_authority": "forgejo-reconcile-local-rust",
                "merge_gate_policy_sha256": merge_inputs["policy_sha256"],
                "merge_gate_binary_sha256": merge_tools["record"]["binary_sha256"]["forgejo-reconcile-local"],
                "merge_gate_verdict": merge_gate_result["verdict"],
                "merge_authorized": merge_gate_result["merge_authorized"],
                "merge_attempted": merge_gate_result["merge_attempted"],
                "merge_gate_error_code": error_code,
                "concurrent_base_advance": concurrent_advance,
            }
            write_json(evidence / "forgejo-result.json", refusal_result)
            return refusal_result
        if merge_gate_exit != 0:
            raise RuntimeError(f"Rust merge authority returned exit {merge_gate_exit}")
        validate_rust_merge_gate_result(
            merge_gate_result, pull_request=number, base=base, candidate=head,
            admission=admission, admission_sha256=admission_sha256,
        )
        merge_status = merge_gate_result["http_status"]
        merge_response = merge_gate_result
        _, merged_pr, _ = owner.expect("GET", pr_path)
        _, merged_main, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
        _, merged_issue, _ = owner.expect("GET", f"/repos/{OWNER}/{REPOSITORY}/issues/1")
        _, merged_source_doc, _ = owner.expect(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=main"
        )
        merged_source = base64.b64decode(merged_source_doc["content"])
        merge_commit = merged_pr.get("merge_commit_sha")
        if not isinstance(merge_commit, str) or not re.fullmatch(r"[0-9a-f]{40}", merge_commit):
            raise RuntimeError("Forgejo did not return the expected merge commit")
        merged_ref = f"refs/mission012/merged-{evidence.name}"
        git(["fetch", "--no-tags", str(git_repo), f"refs/heads/main:{merged_ref}"], repo=trusted_repo)
        merge_parents = git(["show", "-s", "--format=%P", merge_commit], repo=trusted_repo).decode().split()
        merge_tree = git(["rev-parse", f"{merge_commit}^{{tree}}"], repo=trusted_repo).decode().strip()
        candidate_tree = git(["rev-parse", f"{head}^{{tree}}"], repo=trusted_repo).decode().strip()
        if (
            merged_pr.get("state") != "closed" or merged_pr.get("merged") is not True
            or merged_pr.get("head", {}).get("sha") != head
            or merged_main.get("commit", {}).get("id") != merge_commit
            or not merge_parents_match(merge_parents, base, head)
            or merge_tree != candidate_tree
            or digest(merged_source) != source_sha or merged_issue.get("state") != "closed"
        ):
            raise RuntimeError("Forgejo post-merge read-back does not match the independently verified candidate")
        webhook_reconciliation = None
        if webhook_observer is not None:
            webhook_reconciliation = finish_pr_webhook_observer(
                owner,
                webhook_observer,
                binary,
                Path(merge_tools["authority_binary"]),
                number,
                base,
                head,
                merge_commit,
                merged_pr,
                evidence,
            )
        write_json(evidence / "forgejo-result.json", {
            "schema_version": 1, "forgejo_version": "17.0.0+gitea-1.22.0",
            "forgejo_binary_sha256": FORGEJO_SHA, "origin": ORIGIN, "repository_private": True,
            "issue_number": 1, "issue_state_after_merge": merged_issue["state"],
            "pull_request_number": number, "pull_request_state_after_merge": merged_pr["state"],
            "merged": merged_pr["merged"], "merge_http_status": merge_status,
            "base_commit": base, "requirement_sha256": REQUIREMENT_SHA,
            "policy_sha256": POLICY_SHA, "candidate_commit": head,
            "merge_commit": merged_pr.get("merge_commit_sha"),
            "main_head_after_merge": merged_main["commit"]["id"],
            "candidate_source_sha256": source_sha, "changed_paths": changed_paths,
            "pre_review_merge_refusal_http": refusal_status,
            "non_whitelisted_reviewer_merge_refusal_http": reviewer_merge_status,
            "reviewer": REVIEWER, "approval_commit": approval["commit_id"],
            "independent_verdict": verifier["verdict"],
            "merge_authority": "forgejo-reconcile-local-rust",
            "merge_gate_policy_sha256": merge_inputs["policy_sha256"],
            "merge_gate_binary_sha256": merge_tools["record"]["binary_sha256"]["forgejo-reconcile-local"],
            "merge_gate_proof_sha256": merge_inputs["proof_sha256"],
            "merge_gate_result_sha256": digest(json.dumps(merge_gate_result, sort_keys=True, indent=2).encode() + b"\n"),
            "model": summary["model"], "model_effort": summary["effort"],
            "model_cost_usd": summary["reported_cost_usd"],
            "host_receipt_sha256": host_receipt_sha,
            "webhook_reconciliation": webhook_reconciliation,
            "model_usage_provenance": (
                "deterministic local test adapter; no model was invoked; model cost is zero"
                if summary["agent_adapter"] == "local-test-adapter"
                else "model, effort, and estimated cost are reported by the local Claude CLI; not independently attested"
            ),
            "authority": "forgejo-reconcile-local Rust merge authority; local owner credential passed through a private inherited pipe FD; not production merge authority",
            "limitation": verifier["limitation"],
        })
        return {
            "status": "merged_in_private_sandbox", "pull_request": number,
            "base_commit": base, "requirement_sha256": REQUIREMENT_SHA,
            "policy_sha256": POLICY_SHA,
            "candidate_commit": head, "merge_commit": merged_pr.get("merge_commit_sha"),
            "candidate_sha256": source_sha, "changed_paths": changed_paths,
            "verifier": verifier["verdict"], "reviewer": REVIEWER,
            "verifier_cpu_accounting": verifier_cpu_accounting,
            "pre_review_merge_refusal_http": refusal_status,
            "reviewer_merge_refusal_http": reviewer_merge_status,
            "issue_state": merged_issue["state"], "evidence_dir": str(evidence),
            "merge_authority": "forgejo-reconcile-local-rust",
            "merge_gate_policy_sha256": merge_inputs["policy_sha256"],
            "merge_gate_binary_sha256": merge_tools["record"]["binary_sha256"]["forgejo-reconcile-local"],
            "webhook_reconciliation": webhook_reconciliation,
        }
    finally:
        webhook_cleanup = cleanup_pr_webhook_observer(owner, webhook_observer)
        if webhook_observer is not None and (
            webhook_cleanup.get("cleanup_failure") is not None
            or webhook_cleanup.get("hook_deactivated") is not True
            or webhook_cleanup.get("receiver_stopped") is not True
            or webhook_cleanup.get("receiver_listener_absent") is not True
        ) and sys.exc_info()[0] is None:
            raise RuntimeError("PR webhook observer cleanup was not confirmed")
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        server_log.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument("--execution-id", default="live-mcp-005")
    parser.add_argument("--budget-cap-usd", required=True)
    parser.add_argument("--admission-file")
    parser.add_argument("--admission-evidence-dir")
    parser.add_argument("--observe-pr-webhooks", action="store_true")
    parser.add_argument("--inject-concurrent-base-advance", action="store_true")
    args = parser.parse_args()
    try:
        print(json.dumps(run(args), sort_keys=True))
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
