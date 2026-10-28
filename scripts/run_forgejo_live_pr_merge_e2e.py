#!/usr/bin/env python3
"""Drive one isolated Forgejo v17 PR from issue through a protected merge.

This is a local integration-test driver, not a production merge authority.
The Forgejo database and generated credentials stay under a private /tmp
directory; raw tokens are kept in process memory only.
"""

from __future__ import annotations

import argparse
import base64
from contextlib import redirect_stdout
import hashlib
import hmac
import io
import json
import os
from pathlib import Path
import re
import selectors
import secrets
import shutil
import socket
import subprocess
import sys
import threading
import time
from typing import Any, BinaryIO, Callable
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

import run_forgejo_mission_delivery_process_e2e as process_e2e
import mission_012_sandboxed_test_adapter as sandbox_probe
import mission_012_test_agent_adapter as test_agent_adapter
import semantic_merge_canary


ORIGIN = "http://127.0.0.1:3130"
OWNER = "forge-admin"
REPOSITORY = "agent-forge-core"
REVIEWER = "forge-reviewer"
MERGER = "forge-merger"
POLICY_READER = "forge-policy-reader"
TEST_OUTPUT_LIMIT = 256 * 1024

REQUIREMENT = (
    "Implement clamp_inclusive(value, min, max) in Rust. For min <= max: "
    "values below min return min; above max return max; values inside the "
    "inclusive interval remain unchanged. Acceptance tests are host-owned "
    "and immutable during execution."
)


def build_stdio_mcp_candidate_fixture() -> tuple[bytes, bytes, bytes, tuple[str, str]]:
    """Return a live-Forgejo fixture accepted by the deterministic MCP adapter."""
    bug_marker = test_agent_adapter.SOURCE_BUG.encode("utf-8")
    fix_marker = test_agent_adapter.SOURCE_FIX.encode("utf-8")
    initial_source = (
        "pub fn clamp_inclusive(value: i32, minimum: i32, maximum: i32) -> i32 {\n"
        "    if value < minimum {\n"
        "        minimum\n"
        "    } else if value > maximum {\n"
        f"        {test_agent_adapter.SOURCE_BUG}\n"
        "    } else {\n"
        "        value\n"
        "    }\n"
        "}\n"
    ).encode("utf-8")
    if initial_source.count(bug_marker) != 1:
        raise RuntimeError("stdio MCP fixture has no unique adapter correction marker")
    expected_source = initial_source.replace(bug_marker, fix_marker, 1)
    acceptance_source = (
        "use mission012_egress_fixture::clamp_inclusive;\n\n"
        "#[test]\n"
        "fn values_outside_the_range_clamp_to_the_nearest_boundary() {\n"
        "    assert_eq!(clamp_inclusive(12, -2, 10), 10);\n"
        "}\n"
    ).encode("utf-8")
    return initial_source, expected_source, acceptance_source, ("left: -2", "right: 10")


def linux_process_binding(pid: int, *, role: str = "process") -> dict[str, int | str]:
    if isinstance(pid, bool) or not isinstance(pid, int) or pid <= 0:
        raise RuntimeError("supervisor process binding requires a positive PID")
    stat_path = Path(f"/proc/{pid}/stat")
    try:
        first_stat = stat_path.read_text(encoding="ascii")
        close_paren = first_stat.rfind(")")
        if close_paren < 0:
            raise ValueError("malformed process stat")
        start_time_ticks = int(first_stat[close_paren + 1 :].split()[19])
        pid_namespace = os.readlink(f"/proc/{pid}/ns/pid")
        network_namespace = os.readlink(f"/proc/{pid}/ns/net")
        second_stat = stat_path.read_text(encoding="ascii")
        second_close_paren = second_stat.rfind(")")
        second_start_time_ticks = int(second_stat[second_close_paren + 1 :].split()[19])
    except (OSError, ValueError, IndexError) as error:
        raise RuntimeError(
            f"cannot read a stable {role} process binding for pid {pid} ({type(error).__name__})"
        ) from error
    if (
        start_time_ticks <= 0
        or second_start_time_ticks != start_time_ticks
        or re.fullmatch(r"pid:\[[1-9][0-9]*\]", pid_namespace) is None
        or re.fullmatch(r"net:\[[1-9][0-9]*\]", network_namespace) is None
    ):
        raise RuntimeError("supervisor process binding changed or has invalid namespaces")
    return {
        "pid": pid,
        "start_time_ticks": start_time_ticks,
        "pid_namespace": pid_namespace,
        "network_namespace": network_namespace,
    }


def build_supervisor_capability_packet(
    *, gate_pid: int, forgejo_pid: int, target_profile: str, pull_request_number: int
) -> tuple[bytes, dict[str, Any]]:
    if target_profile != "merge-014-agent-forge-core":
        raise ValueError("the live Forgejo fixture only issues its configured merge target")
    if isinstance(pull_request_number, bool) or not isinstance(pull_request_number, int) or pull_request_number <= 0:
        raise ValueError("supervisor capability pull request number is invalid")
    target = {
        "profile_selector": target_profile,
        "origin": ORIGIN,
        "owner": OWNER,
        "repository": REPOSITORY,
        "pull_request_number": pull_request_number,
        "base_branch": "main",
    }
    supervisor = linux_process_binding(os.getpid(), role="supervisor")
    gate = linux_process_binding(gate_pid, role="gate")
    forgejo = linux_process_binding(forgejo_pid, role="Forgejo")
    if (
        supervisor["pid_namespace"] != gate["pid_namespace"]
        or gate["pid_namespace"] != forgejo["pid_namespace"]
        or gate["network_namespace"] != forgejo["network_namespace"]
    ):
        raise RuntimeError("test supervisor, gate, and Forgejo namespaces do not match")
    envelope = {
        "schema_version": 1,
        "target": target,
        "supervisor": supervisor,
        "supervisor_uid": os.geteuid(),
        "gate": gate,
        "forgejo": forgejo,
        "expires_at_monotonic_ns": time.clock_gettime_ns(time.CLOCK_BOOTTIME) + 60_000_000_000,
        "nonce": secrets.token_hex(32),
    }
    packet = json.dumps(envelope, sort_keys=True, separators=(",", ":")).encode("ascii")
    if len(packet) > 8192:
        raise RuntimeError("supervisor capability packet exceeds its bound")
    record = {
        "schema_version": 1,
        "target": target,
        "supervisor": supervisor,
        "supervisor_uid": os.geteuid(),
        "gate": gate,
        "forgejo": forgejo,
        "expires_at_monotonic_ns": envelope["expires_at_monotonic_ns"],
        "capability_packet_sha256": hashlib.sha256(packet).hexdigest(),
        "capability_packet_bytes": len(packet),
        "gate_socket_peer_pid_expected": os.getpid(),
        "supervisor_policy_independently_verified": False,
    }
    return packet, record


def validate_initial_candidate_verifier_result(
    status: int, output: bytes, *, stdio_mcp: bool
) -> None:
    if stdio_mcp:
        failed_markers = (
            b"above_max_returns_max ... FAILED",
            b"i32_max_clamps ... FAILED",
        )
        passed_markers = (
            b"below_min_returns_min ... ok",
            b"in_range_is_preserved ... ok",
            b"equal_lower_bound_is_preserved ... ok",
            b"equal_upper_bound_is_preserved ... ok",
            b"singleton_interval_clamps ... ok",
            b"negative_interval_clamps ... ok",
            b"i32_min_clamps ... ok",
        )
        summary = b"7 passed; 2 failed"
    else:
        failed_markers = (
            b"below_min_returns_min ... FAILED",
            b"above_max_returns_max ... FAILED",
            b"singleton_interval_clamps ... FAILED",
            b"i32_min_clamps ... FAILED",
            b"i32_max_clamps ... FAILED",
        )
        passed_markers = (
            b"in_range_is_preserved ... ok",
            b"equal_lower_bound_is_preserved ... ok",
            b"equal_upper_bound_is_preserved ... ok",
            b"negative_interval_clamps ... ok",
        )
        summary = b"4 passed; 5 failed"
    if (
        status == 0
        or not all(marker in output for marker in failed_markers)
        or not all(marker in output for marker in passed_markers)
        or summary not in output
    ):
        raise RuntimeError("initial candidate did not produce the expected actionable failure")


def build_policy_bytes(reviewer_id: int) -> bytes:
    policy = {
        "schema_version": 1,
        "revision": 1,
        "verifier_id": "mission-012-verify-local",
        "environment": "local Forgejo v17.0.0 and Bubblewrap test verifier",
        "acceptance_command": ["rustc", "--edition=2024", "src/lib.rs"],
        "commands": [["rustc", "--edition=2024", "src/lib.rs"]],
        "protected_criteria": ["src/lib.rs"],
        "timeout_seconds": 90,
        "merge_approval_policy": {
            "schema_version": 1,
            "required_approvals": 1,
            "authorized_reviewer_ids": [reviewer_id],
        },
    }
    return json.dumps(policy, sort_keys=True, separators=(",", ":")).encode()
WORKER_RS = b"""#[path = "/workspace/candidate.rs"]
mod candidate;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        std::process::exit(64);
    }
    let parse = |index: usize| -> i32 {
        args[index].parse().unwrap_or_else(|_| std::process::exit(65))
    };
    let value = parse(1);
    let min = parse(2);
    let max = parse(3);
    println!("{}", candidate::clamp_inclusive(value, min, max));
}
"""
FAKE_SUCCESS_RS = (
    b"pub fn clamp_inclusive(_value: i32, _min: i32, _max: i32) -> i32 {\n"
    b"    std::process::exit(0)\n"
    b"}\n"
)


class ApiFailure(RuntimeError):
    def __init__(self, status: int, path: str, body: str):
        self.status = status
        self.path = path
        self.body = body
        super().__init__(f"Forgejo API returned HTTP {status} for {path}: {body[:500]}")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def bounded_process(
    command: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout: int = 60,
    output_limit: int = TEST_OUTPUT_LIMIT,
    pass_fds: tuple[int, ...] = (),
    after_spawn: Callable[[subprocess.Popen[bytes]], None] | None = None,
) -> tuple[int, bytes]:
    process = subprocess.Popen(
        command,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        pass_fds=pass_fds,
    )
    output = bytearray()
    overflow = threading.Event()

    def drain() -> None:
        assert process.stdout is not None
        while True:
            chunk = process.stdout.read(65536)
            if not chunk:
                return
            if len(output) + len(chunk) > output_limit:
                overflow.set()
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
            elif not overflow.is_set():
                output.extend(chunk)

    reader = threading.Thread(target=drain)
    reader.start()
    try:
        if after_spawn is not None:
            after_spawn(process)
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        raise RuntimeError(f"command exceeded {timeout} seconds: {command[0]}")
    except BaseException:
        if process.poll() is None:
            process.kill()
            process.wait()
        raise
    finally:
        reader.join(timeout=5)
        if process.stdout is not None:
            process.stdout.close()
    if overflow.is_set():
        raise RuntimeError(f"command exceeded {output_limit} output bytes: {command[0]}")
    return status, bytes(output)


def write_private(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    fd = os.open(path, flags, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(content)
        handle.flush()
        os.fsync(handle.fileno())


def write_json(path: Path, value: Any, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    data = json.dumps(value, sort_keys=True, indent=2).encode() + b"\n"
    write_private(path, data)
    path.chmod(mode)


def call_admin(binary: Path, state: Path, args: list[str]) -> bytes:
    command = [
        str(binary),
        "--work-path",
        str(state),
        "--config",
        str(state / "custom/conf/app.ini"),
        *args,
    ]
    process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if process.returncode:
        # Forgejo CLI output can include generated credentials. Never echo it.
        raise RuntimeError(f"Forgejo administrative command failed with exit {process.returncode}")
    return process.stdout


def make_api(token: str | None):
    # The bearer token must never be sent through an environment-configured
    # proxy, even when localhost is missing from NO_PROXY.
    opener = build_opener(ProxyHandler({}), NoRedirect)

    def api(method: str, path: str, data: Any = None) -> tuple[int, Any, dict[str, str]]:
        if not path.startswith("/") or path.startswith("//"):
            raise RuntimeError("API path is outside the configured Forgejo origin")
        headers = {"Accept": "application/json"}
        if token is not None:
            headers["Authorization"] = f"token {token}"
        payload = None
        if data is not None:
            headers["Content-Type"] = "application/json"
            payload = json.dumps(data).encode()
        req = Request(ORIGIN + "/api/v1" + path, data=payload, headers=headers, method=method)
        try:
            with opener.open(req, timeout=5) as response:
                body = response.read(1024 * 1024 + 1)
                if len(body) > 1024 * 1024:
                    raise RuntimeError("Forgejo API response exceeded 1 MiB")
                return (
                    response.status,
                    json.loads(body) if body else None,
                    {key.lower(): value for key, value in response.headers.items()},
                )
        except HTTPError as error:
            body = error.read(2048).decode("utf-8", "replace")
            raise ApiFailure(error.code, path, body) from None
        except URLError as error:
            raise RuntimeError(f"Forgejo API transport failed for {path}: {error.reason}") from None

    return api


def api_error_message(error: ApiFailure) -> str:
    """Extract only Forgejo's structured refusal reason for evidence checks."""
    try:
        document = json.loads(error.body)
    except json.JSONDecodeError:
        return ""
    message = document.get("message") if isinstance(document, dict) else None
    return message if isinstance(message, str) else ""


def assert_forgejo_listener(process: subprocess.Popen, port: int = 3130) -> None:
    """After health succeeds, fail closed unless ss attributes the port to us."""
    ss = Path("/usr/bin/ss")
    if not ss.is_file() or ss.is_symlink():
        raise RuntimeError("ss is required to verify local Forgejo port ownership")
    result = subprocess.run(
        [str(ss), "-H", "-ltnp", f"sport = :{port}"],
        cwd="/",
        env={"PATH": "/usr/bin:/bin"},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=5,
    )
    if result.returncode:
        raise RuntimeError("could not verify the owner of the local Forgejo listener")
    pids = {int(value) for value in re.findall(r"\bpid=(\d+)\b", result.stdout)}
    if pids != {process.pid}:
        raise RuntimeError(
            "local health endpoint is not served only by the Forgejo process started by this run"
        )


def read_changed_files(api, pr_path: str) -> list[dict[str, Any]]:
    """Read the complete paginated PR file list and validate its count header."""
    files: list[dict[str, Any]] = []
    expected_total: int | None = None
    for page in range(1, 101):
        _, page_files, headers = api(
            "GET", f"{pr_path}/files?limit=100&page={page}"
        )
        if not isinstance(page_files, list) or any(
            not isinstance(item, dict) or not isinstance(item.get("filename"), str)
            for item in page_files
        ):
            raise RuntimeError("Forgejo PR-files endpoint returned an invalid page")
        count_header = headers.get("x-total-count")
        if count_header is not None:
            try:
                page_total = int(count_header)
            except ValueError:
                raise RuntimeError("Forgejo PR-files endpoint returned an invalid total count") from None
            if expected_total is not None and page_total != expected_total:
                raise RuntimeError("Forgejo PR file count changed during pagination")
            expected_total = page_total
        files.extend(page_files)
        if expected_total is not None:
            if len(files) > expected_total:
                raise RuntimeError("Forgejo PR-files pagination exceeded its reported total")
            if len(files) == expected_total:
                break
        if len(page_files) < 100:
            break
    else:
        raise RuntimeError("Forgejo PR-files pagination exceeded 100 pages")
    if expected_total is not None and len(files) != expected_total:
        raise RuntimeError("Forgejo PR-files pagination was incomplete")
    if len({item["filename"] for item in files}) != len(files):
        raise RuntimeError("Forgejo PR-files pagination returned duplicate paths")
    return files


def create_app_ini(
    state: Path,
    *,
    allow_loopback_webhooks: bool = False,
    actions_enabled: bool = False,
) -> None:
    for name in ("data", "repositories", "custom/conf", "log", "tmp", "home", "evidence"):
        (state / name).mkdir(parents=True, exist_ok=True, mode=0o700)
    config = state / "custom/conf/app.ini"
    lines = [
        "[database]",
        "DB_TYPE = sqlite3",
        f"PATH = {state / 'data/forgejo.db'}",
        "",
        "[server]",
        f"APP_DATA_PATH = {state / 'data'}",
        "DOMAIN = 127.0.0.1",
        "HTTP_ADDR = 127.0.0.1",
        "HTTP_PORT = 3130",
        f"ROOT_URL = {ORIGIN}/",
        "PROTOCOL = http",
        "DISABLE_SSH = true",
        "START_SSH_SERVER = false",
        "LFS_START_SERVER = false",
        "",
        "[repository]",
        f"ROOT = {state / 'repositories'}",
        "DEFAULT_BRANCH = main",
        "",
        "[service]",
        "DISABLE_REGISTRATION = true",
        "REQUIRE_SIGNIN_VIEW = false",
        "",
        "[security]",
        "INSTALL_LOCK = true",
        f"SECRET_KEY = {secrets.token_urlsafe(48)}",
        f"INTERNAL_TOKEN = {secrets.token_hex(64)}",
        f"LFS_JWT_SECRET = {secrets.token_hex(32)}",
        "",
        "[log]",
        "MODE = file",
        "LEVEL = Warn",
        f"ROOT_PATH = {state / 'log'}",
        "",
        "[actions]",
        f"ENABLED = {'true' if actions_enabled else 'false'}",
        "",
        "[session]",
        "PROVIDER = memory",
        "",
        "[cache]",
        "ADAPTER = memory",
        "",
    ]
    if allow_loopback_webhooks:
        lines.extend(["[webhook]", "ALLOWED_HOST_LIST = 127.0.0.1", ""])
    write_private(config, "\n".join(lines).encode())


def wait_for_health(process: subprocess.Popen, timeout: int = 30) -> None:
    deadline = time.monotonic() + timeout
    opener = build_opener(ProxyHandler({}), NoRedirect)
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Forgejo exited during startup with code {process.returncode}")
        try:
            with opener.open(ORIGIN + "/api/healthz", timeout=1) as response:
                body = response.read(16 * 1024 + 1)
                if len(body) > 16 * 1024:
                    raise RuntimeError("Forgejo health response exceeded 16 KiB")
                status = response.status
                health = json.loads(body)
            if status == 200 and health.get("status") == "pass":
                return
        except Exception:
            pass
        time.sleep(0.2)
    raise RuntimeError("Forgejo health endpoint did not become ready within 30 seconds")


def run_candidate_sandbox(
    source: bytes, phase_dir: Path, *, bwrap: str, rust_sysroot: Path
) -> tuple[int, bytes, str]:
    phase_dir.mkdir(mode=0o700)
    write_private(phase_dir / "candidate.rs", source)
    write_private(phase_dir / "worker.rs", WORKER_RS)
    rustc = rust_sysroot / "bin/rustc"
    if not rustc.is_file():
        raise RuntimeError("rustc is missing from the discovered toolchain sysroot")
    build_helper = r'''import os, shutil, subprocess, sys
build = subprocess.run(
    ["/toolchain/bin/rustc", "--edition=2024", "/workspace/worker.rs", "-o", "/build/worker"],
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=45)
if build.stdout:
    sys.stderr.buffer.write(build.stdout[-65536:])
    sys.stderr.buffer.flush()
if build.returncode:
    raise SystemExit(build.returncode)
size = os.path.getsize("/build/worker")
if size > 67108864:
    raise SystemExit("candidate worker exceeded 64 MiB artifact limit")
shutil.copyfile("/build/worker", "/artifacts/candidate-worker")
sys.stdout.buffer.write(f"ARTIFACT candidate-worker {size}\n".encode())
with open("/artifacts/candidate-worker", "rb") as worker:
    shutil.copyfileobj(worker, sys.stdout.buffer)
sys.stdout.buffer.flush()
'''
    worker_path = phase_dir / "candidate-worker"
    build_command = [
        bwrap,
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
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--size",
        "67108864",
        "--tmpfs",
        "/home",
        "--size",
        "67108864",
        "--tmpfs",
        "/tmp",
        "--size",
        "67108864",
        "--tmpfs",
        "/artifacts",
        "--size",
        "134217728",
        "--tmpfs",
        "/build",
        "--dir",
        "/workspace",
        "--ro-bind",
        str(phase_dir / "candidate.rs"),
        "/workspace/candidate.rs",
        "--ro-bind",
        str(phase_dir / "worker.rs"),
        "/workspace/worker.rs",
        "--dir",
        "/toolchain",
        "--ro-bind",
        str(rust_sysroot),
        "/toolchain",
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
        "RUSTC",
        "/toolchain/bin/rustc",
        "--chdir",
        "/workspace",
        "/usr/bin/prlimit",
        "--cpu=60",
        "--as=2147483648",
        "--nproc=64",
        "--nofile=64",
        "--fsize=67108864",
        "--",
        "/usr/bin/python3",
        "-c",
        build_helper,
    ]
    build_log = io.StringIO()
    with redirect_stdout(build_log):
        process_e2e.run_artifact_build(
            build_command,
            cwd=phase_dir,
            artifacts={"candidate-worker": worker_path},
        )
    write_private(phase_dir / "candidate-build.log", build_log.getvalue().encode())
    transcript = bytearray()

    # The trusted acceptance table stays in this host process. The candidate
    # worker runs separately, once per case, with no source tree or test code
    # mounted in its namespace; process::exit(0) therefore cannot skip tests.
    cases = (
        ("below_min_returns_min", -3, 0, 7, 0),
        ("above_max_returns_max", 9, 0, 7, 7),
        ("in_range_is_preserved", 4, 0, 7, 4),
        ("equal_lower_bound_is_preserved", -7, -7, 7, -7),
        ("equal_upper_bound_is_preserved", 7, -7, 7, 7),
        ("singleton_interval_clamps", 42, 5, 5, 5),
        ("negative_interval_clamps", -4, -9, -2, -4),
        ("i32_min_clamps", -2147483648, -7, 7, -7),
        ("i32_max_clamps", 2147483647, -7, 7, 7),
    )
    failed = 0
    for name, value, minimum, maximum, expected in cases:
        command = [
            bwrap,
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
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--size",
            "33554432",
            "--tmpfs",
            "/home",
            "--size",
            "33554432",
            "--tmpfs",
            "/tmp",
            "--ro-bind",
            str(worker_path),
            "/worker",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
            "--setenv",
            "HOME",
            "/tmp",
            "--setenv",
            "TMPDIR",
            "/tmp",
            "--chdir",
            "/tmp",
            "/usr/bin/prlimit",
            "--cpu=5",
            "--as=268435456",
            "--nproc=16",
            "--nofile=32",
            "--fsize=1048576",
            "--",
            "/worker",
            str(value),
            str(minimum),
            str(maximum),
        ]
        try:
            status, output = bounded_process(command, timeout=10, output_limit=4096)
        except RuntimeError as error:
            status, output = 124, str(error).encode("utf-8", errors="replace")
        if status == 0 and output == f"{expected}\n".encode():
            transcript.extend(f"{name} ... ok\n".encode())
        else:
            failed += 1
            transcript.extend(
                f"{name} ... FAILED (exit={status}, expected={expected}, output={output[:128]!r})\n".encode()
            )
        if len(transcript) > TEST_OUTPUT_LIMIT:
            raise RuntimeError("candidate verifier transcript exceeded output budget")
    transcript.extend(f"{len(cases) - failed} passed; {failed} failed\n".encode())
    return (1 if failed else 0), bytes(transcript), process_e2e.sha256_file(worker_path)


def run_semantic_acceptance_sandbox(
    source: bytes,
    acceptance_source: bytes,
    phase_dir: Path,
    *,
    bwrap: str,
    rust_sysroot: Path,
) -> tuple[int, bytes]:
    """Run host-owned Rust acceptance against exact Forgejo-read candidate bytes."""
    phase_dir.mkdir(mode=0o700)
    write_private(phase_dir / "candidate.rs", source)
    test_source = acceptance_source.replace(b"@CANDIDATE@", b"/workspace/candidate.rs")
    write_private(phase_dir / "acceptance.rs", test_source)
    helper = r'''import subprocess, sys
build = subprocess.run(
    ["/toolchain/bin/rustc", "--edition=2024", "--test", "/workspace/acceptance.rs", "-o", "/build/acceptance"],
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
if build.stdout:
    sys.stdout.buffer.write(b"BUILD\n" + build.stdout[-65536:])
    sys.stdout.buffer.flush()
if build.returncode:
    raise SystemExit(build.returncode)
test = subprocess.run(["/build/acceptance"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
sys.stdout.buffer.write(b"RUN\n" + test.stdout[-65536:])
sys.stdout.buffer.flush()
raise SystemExit(test.returncode)
'''
    command = [
        bwrap,
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
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--size",
        "67108864",
        "--tmpfs",
        "/home",
        "--size",
        "67108864",
        "--tmpfs",
        "/tmp",
        "--size",
        "67108864",
        "--tmpfs",
        "/build",
        "--dir",
        "/workspace",
        "--ro-bind",
        str(phase_dir / "candidate.rs"),
        "/workspace/candidate.rs",
        "--ro-bind",
        str(phase_dir / "acceptance.rs"),
        "/workspace/acceptance.rs",
        "--dir",
        "/toolchain",
        "--ro-bind",
        str(rust_sysroot),
        "/toolchain",
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
        "--chdir",
        "/workspace",
        "/usr/bin/prlimit",
        "--cpu=45",
        "--as=1073741824",
        "--nproc=32",
        "--nofile=64",
        "--fsize=67108864",
        "--",
        "/usr/bin/python3",
        "-c",
        helper,
    ]
    status, output = bounded_process(command, timeout=50, output_limit=TEST_OUTPUT_LIMIT)
    return status, output


def make_semantic_target_proof(
    *,
    state: Path,
    gate: Path,
    key_path: Path,
    evidence: dict[str, Any],
    outcome: str,
    name: str,
) -> tuple[Path, Path, Path, Path]:
    if outcome not in {"pass", "fail"} or evidence.get("outcome") != outcome:
        raise ValueError("semantic proof outcome must match the observed verifier result")
    target = {
        "profile_selector": "merge-014-agent-forge-core",
        "origin": ORIGIN,
        "owner": OWNER,
        "repository": REPOSITORY,
        "base_branch": "main",
    }
    target_path = state / "evidence" / f"{name}-target.json"
    evidence_path = state / "evidence" / f"{name}-evidence.json"
    proof_path = state / "evidence" / f"{name}-proof-v2.json"
    expected_path = state / "evidence" / f"{name}-expected.json"
    write_json(target_path, target)
    write_json(evidence_path, evidence)
    write_json(
        expected_path,
        {
            "requirement_sha256": evidence["requirement_sha256"],
            "policy_sha256": evidence["policy_sha256"],
            "candidate_commit": evidence["candidate_commit"],
            "base_commit": evidence["base_commit"],
            "merge_commit": None,
            "verifier_id": evidence["verifier_id"],
        },
    )
    sign = subprocess.run(
        [str(gate), "sign-target-v2", str(key_path), str(evidence_path), str(target_path), str(proof_path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=10,
        check=False,
    )
    write_private(state / "evidence" / f"{name}-sign.log", sign.stdout)
    if sign.returncode != 0 or not proof_path.is_file():
        raise RuntimeError(f"could not sign semantic {outcome} proof")
    proof_path.chmod(0o600)
    check = subprocess.run(
        [str(gate), "check-target-v2", str(key_path), str(proof_path), str(target_path), str(expected_path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=10,
        check=False,
    )
    write_private(state / "evidence" / f"{name}-self-check.log", check.stdout)
    if outcome == "pass":
        if check.returncode != 0 or b"PASS:" not in check.stdout:
            raise RuntimeError("semantic passing proof failed its independent signature/binding check")
    elif check.returncode == 0 or b"OutcomeFailed" not in check.stdout:
        raise RuntimeError("semantic failing proof was not cryptographically valid with an OutcomeFailed verdict")
    return proof_path, expected_path, target_path, evidence_path


def read_forgejo_file(api, path: str, ref: str) -> tuple[bytes, str]:
    _, document, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/contents/{path}?ref={quote(ref, safe='')}")
    if (
        not isinstance(document, dict)
        or document.get("encoding") != "base64"
        or not isinstance(document.get("content"), str)
        or not isinstance(document.get("sha"), str)
    ):
        raise RuntimeError(f"Forgejo did not return base64 content and a blob SHA for {path} at {ref}")
    try:
        content = base64.b64decode(document["content"])
    except (ValueError, TypeError):
        raise RuntimeError(f"Forgejo returned invalid base64 content for {path} at {ref}") from None
    return content, document["sha"]


def write_forgejo_file(api, *, path: str, ref: str, content: bytes, message: str) -> dict[str, Any]:
    current, blob_sha = read_forgejo_file(api, path, ref)
    _, updated, _ = api(
        "PUT",
        f"/repos/{OWNER}/{REPOSITORY}/contents/{path}",
        {
            "message": message,
            "content": base64.b64encode(content).decode("ascii"),
            "branch": ref,
            "sha": blob_sha,
        },
    )
    if not isinstance(updated, dict) or not isinstance(updated.get("content"), dict):
        raise RuntimeError(f"Forgejo did not confirm the write of {path} to {ref}")
    written_sha = updated["content"].get("sha")
    readback, readback_blob_sha = read_forgejo_file(api, path, ref)
    if readback != content or readback_blob_sha != written_sha:
        raise RuntimeError(f"Forgejo content read-back differs after writing {path} to {ref}")
    if current == content:
        raise RuntimeError(f"refusing to record a no-op semantic canary write for {path} at {ref}")
    return updated["content"]


def make_semantic_evidence(
    *,
    requirement: bytes,
    policy_bytes: bytes,
    candidate_commit: str,
    base_commit: str,
    source: bytes,
    acceptance_source: bytes,
    output: bytes,
    outcome: str,
    detail: str,
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "requirement_sha256": hashlib.sha256(requirement).hexdigest(),
        "policy_sha256": hashlib.sha256(policy_bytes).hexdigest(),
        "candidate_commit": candidate_commit,
        "base_commit": base_commit,
        "merge_commit": None,
        "verifier_id": "mission-012-verify-local",
        "environment": "local Forgejo v17.0.0, Bubblewrap, host-owned Rust acceptance",
        "commands": [
            ["rustc", "--edition=2024", "--test", "/workspace/acceptance.rs", "-o", "/build/acceptance"],
            ["/build/acceptance"],
        ],
        "artifact_sha256": [
            hashlib.sha256(source).hexdigest(),
            hashlib.sha256(acceptance_source).hexdigest(),
            hashlib.sha256(output).hexdigest(),
            hashlib.sha256(b"src/semantic.rs").hexdigest(),
        ],
        "outcome": outcome,
        "detail": detail,
    }


def create_review_binding(
    *,
    api,
    reviewer_api,
    pr_path: str,
    pull_number: int,
    base_sha: str,
    head_sha: str,
    reviewer_id: int,
    evidence_dir: Path,
    name: str,
) -> Path:
    _, before, _ = api("GET", pr_path)
    if (
        before.get("state") != "open"
        or before.get("merged") is not False
        or before.get("base", {}).get("sha") != base_sha
        or before.get("head", {}).get("sha") != head_sha
    ):
        raise RuntimeError("semantic PR changed before independent review; a new proof is required")
    _, review, _ = reviewer_api(
        "POST",
        pr_path + "/reviews",
        {
            "body": f"Reviewed the exact semantic candidate {head_sha} against base {base_sha}.",
            "event": "APPROVED",
            "commit_id": head_sha,
        },
    )
    if (
        review.get("state") != "APPROVED"
        or review.get("commit_id") != head_sha
        or review.get("user", {}).get("id") != reviewer_id
    ):
        raise RuntimeError("Forgejo did not record the semantic approval against the exact head")
    path = evidence_dir / f"{name}-review-binding.json"
    write_json(
        path,
        {
            "schema_version": 1,
            "review_id": review.get("id"),
            "reviewer_id": reviewer_id,
            "reviewer_login": REVIEWER,
            "pull_request_number": pull_number,
            "base_branch": "main",
            "base_commit": base_sha,
            "candidate_commit": head_sha,
            "source": "separate reviewer API approval read back before trusted merge gate",
        },
    )
    write_json(evidence_dir / f"{name}-review.json", review)
    return path


def run_semantic_clean_merge_canary(
    *,
    state: Path,
    api,
    reviewer_api,
    client: Path,
    gate: Path,
    forgejo_pid: int,
    key_path: Path,
    policy_bytes: bytes,
    merge_token: str,
    policy_token: str | None,
    reviewer_id: int,
    bwrap: str,
    rust_sysroot: Path,
) -> dict[str, Any]:
    """Exercise a clean branch merge, protected refusal, correction and gate merge on local Forgejo."""
    evidence_dir = state / "evidence" / "semantic-clean-merge-live"
    evidence_dir.mkdir(mode=0o700)
    fixture = semantic_merge_canary.build_semantic_canary_sources()
    upper_acceptance = semantic_merge_canary.semantic_acceptance_source("upper")
    lower_acceptance = semantic_merge_canary.semantic_acceptance_source("lower")
    protected_acceptance = semantic_merge_canary.semantic_acceptance_source("protected")
    for name, content in (
        ("acceptance-upper.rs", upper_acceptance),
        ("acceptance-lower.rs", lower_acceptance),
        ("acceptance-protected.rs", protected_acceptance),
    ):
        write_private(evidence_dir / name, content)

    policy_document = json.loads(policy_bytes)
    policy_document.update(
        {
            "environment": "local Forgejo v17.0.0, Bubblewrap, host-owned Rust acceptance",
            "acceptance_command": [
                "rustc",
                "--edition=2024",
                "--test",
                "/workspace/acceptance.rs",
            ],
            "commands": [
                ["rustc", "--edition=2024", "--test", "/workspace/acceptance.rs", "-o", "/build/acceptance"],
                ["/build/acceptance"],
            ],
            "protected_criteria": ["src/semantic.rs"],
            "timeout_seconds": 90,
        }
    )
    semantic_policy_bytes = json.dumps(
        policy_document, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    policy_path = evidence_dir / "policy.json"
    write_private(policy_path, semantic_policy_bytes)

    def create_branch(branch: str, old_branch: str) -> None:
        api(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/branches",
            {"new_branch_name": branch, "old_branch_name": old_branch},
        )

    _, initial_main, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
    initial_main_sha = initial_main.get("commit", {}).get("id")
    if not isinstance(initial_main_sha, str):
        raise RuntimeError("semantic fixture could not read the exact main base")
    base_branch = "fixture/semantic-base"
    lower_branch = "agent/semantic-lower"
    upper_branch = "agent/semantic-upper"
    create_branch(base_branch, "main")
    api(
        "POST",
        f"/repos/{OWNER}/{REPOSITORY}/contents/src/semantic.rs",
        {
            "message": "Add the shared semantic canary baseline",
            "content": base64.b64encode(fixture["base"]).decode("ascii"),
            "branch": base_branch,
        },
    )
    base_source, _ = read_forgejo_file(api, "src/semantic.rs", base_branch)
    if base_source != fixture["base"]:
        raise RuntimeError("Forgejo baseline branch differs from the semantic canary fixture")
    _, base_branch_snapshot, _ = api(
        "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{quote(base_branch, safe='')}"
    )
    common_commit = base_branch_snapshot.get("commit", {}).get("id")
    if not isinstance(common_commit, str):
        raise RuntimeError("semantic branches do not have a recorded common base commit")
    create_branch(lower_branch, base_branch)
    create_branch(upper_branch, base_branch)
    write_forgejo_file(
        api,
        path="src/semantic.rs",
        ref=lower_branch,
        content=fixture["lower"],
        message="Implement the exclusive lower bound",
    )
    write_forgejo_file(
        api,
        path="src/semantic.rs",
        ref=upper_branch,
        content=fixture["upper"],
        message="Implement the exclusive upper bound",
    )
    lower_branch_source, _ = read_forgejo_file(api, "src/semantic.rs", lower_branch)
    upper_branch_source, _ = read_forgejo_file(api, "src/semantic.rs", upper_branch)
    if lower_branch_source != fixture["lower"] or upper_branch_source != fixture["upper"]:
        raise RuntimeError("semantic branch source differs from the independently prepared changes")

    lower_status, lower_output = run_semantic_acceptance_sandbox(
        lower_branch_source,
        lower_acceptance,
        state / "semantic-verify-lower-branch",
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
    )
    upper_status, upper_output = run_semantic_acceptance_sandbox(
        upper_branch_source,
        upper_acceptance,
        state / "semantic-verify-upper-branch",
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
    )
    write_private(evidence_dir / "lower-branch-acceptance.log", lower_output)
    write_private(evidence_dir / "upper-branch-acceptance.log", upper_output)
    if (
        lower_status != 0
        or upper_status != 0
        or b"1 passed; 0 failed" not in lower_output
        or b"1 passed; 0 failed" not in upper_output
    ):
        raise RuntimeError("an independent semantic branch check failed before composition")

    requirement_upper = (
        b"Requirement revision 1: exclusive upper bound on adjacent values keeps the lower value admissible."
    )
    requirement_lower = (
        b"Requirement revision 1: exclusive lower bound on adjacent values keeps the upper value admissible; "
        b"the combined adjacent interval with both bounds exclusive is empty and must return EmptyRange."
    )
    write_private(evidence_dir / "requirements-upper.md", requirement_upper)
    write_private(evidence_dir / "requirements-lower.md", requirement_lower)
    _, issue, _ = api(
        "POST",
        f"/repos/{OWNER}/{REPOSITORY}/issues",
        {
            "title": "Handle empty ranges after combining exclusive bounds",
            "body": requirement_lower.decode("utf-8"),
        },
    )
    issue_number = issue.get("number")
    if not isinstance(issue_number, int) or issue_number <= 0:
        raise RuntimeError("Forgejo did not return the semantic canary issue number")
    _, upper_pr, _ = api(
        "POST",
        f"/repos/{OWNER}/{REPOSITORY}/pulls",
        {
            "title": "Implement the upper bound for the semantic canary",
            "body": f"Implements the isolated upper-bound criterion for issue #{issue_number}.",
            "head": upper_branch,
            "base": "main",
        },
    )
    _, lower_pr, _ = api(
        "POST",
        f"/repos/{OWNER}/{REPOSITORY}/pulls",
        {
            "title": "Implement and integrate exclusive lower bounds",
            "body": f"Closes #{issue_number}. Includes the combined protected interval criterion.",
            "head": lower_branch,
            "base": "main",
        },
    )
    upper_number = upper_pr.get("number")
    lower_number = lower_pr.get("number")
    if not all(isinstance(number, int) and number > 0 for number in (upper_number, lower_number)):
        raise RuntimeError("Forgejo did not return both semantic pull request numbers")
    upper_path = f"/repos/{OWNER}/{REPOSITORY}/pulls/{upper_number}"
    lower_path = f"/repos/{OWNER}/{REPOSITORY}/pulls/{lower_number}"
    _, lower_branch_snapshot, _ = api(
        "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{quote(lower_branch, safe='')}"
    )
    _, upper_branch_snapshot, _ = api(
        "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{quote(upper_branch, safe='')}"
    )
    lower_initial_head = lower_branch_snapshot.get("commit", {}).get("id")
    upper_head = upper_branch_snapshot.get("commit", {}).get("id")
    if not isinstance(lower_initial_head, str) or not isinstance(upper_head, str):
        raise RuntimeError("Forgejo did not expose the semantic branch commits")
    upper_pr, upper_observations = wait_for_updated_pr_mergeability(
        api,
        upper_path,
        expected_head_sha=upper_head,
        expected_base_sha=initial_main_sha,
    )
    lower_pr, lower_observations = wait_for_updated_pr_mergeability(
        api,
        lower_path,
        expected_head_sha=lower_initial_head,
        expected_base_sha=initial_main_sha,
    )
    write_json(
        evidence_dir / "initial-pr-snapshots.json",
        {
            "common_fixture_commit": common_commit,
            "main_base_commit": initial_main_sha,
            "upper_pr": upper_pr,
            "upper_mergeability_observations": upper_observations,
            "lower_pr": lower_pr,
            "lower_mergeability_observations": lower_observations,
            "upper_branch_individual_test": "passed",
            "lower_branch_individual_test": "passed",
        },
    )
    if upper_pr.get("mergeable") is not True or lower_pr.get("mergeable") is not True:
        raise RuntimeError("Forgejo did not mark both individual semantic changes mergeable")

    def make_evidence(
        *,
        requirement: bytes,
        candidate: str,
        base: str,
        source: bytes,
        acceptance: bytes,
        output: bytes,
        outcome: str,
        detail: str,
    ) -> dict[str, Any]:
        return make_semantic_evidence(
            requirement=requirement,
            policy_bytes=semantic_policy_bytes,
            candidate_commit=candidate,
            base_commit=base,
            source=source,
            acceptance_source=acceptance,
            output=output,
            outcome=outcome,
            detail=detail,
        )

    upper_evidence = make_evidence(
        requirement=requirement_upper,
        candidate=upper_head,
        base=initial_main_sha,
        source=upper_branch_source,
        acceptance=upper_acceptance,
        output=upper_output,
        outcome="pass",
        detail="Host verifier passed the isolated upper-bound acceptance against the exact Forgejo PR head.",
    )
    upper_proof, upper_expected, _, _ = make_semantic_target_proof(
        state=evidence_dir,
        gate=gate,
        key_path=key_path,
        evidence=upper_evidence,
        outcome="pass",
        name="upper-proof",
    )
    upper_review_binding = create_review_binding(
        api=api,
        reviewer_api=reviewer_api,
        pr_path=upper_path,
        pull_number=upper_number,
        base_sha=initial_main_sha,
        head_sha=upper_head,
        reviewer_id=reviewer_id,
        evidence_dir=evidence_dir,
        name="upper",
    )
    upper_gate_status, upper_gate = run_local_test_merge_authority(
        client,
        number=upper_number,
        forgejo_pid=forgejo_pid,
        target_profile="merge-014-agent-forge-core",
        proof=upper_proof,
        expected=upper_expected,
        key=key_path,
        policy=policy_path,
        review_base_binding=upper_review_binding,
        execution_identity="local-test-executor",
        host_token=merge_token,
        policy_token=policy_token,
    )
    write_json(evidence_dir / "upper-merge-gate.json", upper_gate)
    if (
        upper_gate_status != 0
        or upper_gate.get("verdict") != "merged"
        or upper_gate.get("merge_attempted") is not True
        or upper_gate.get("submitted_head_sha") != upper_head
        or upper_gate.get("validated_base_sha") != initial_main_sha
    ):
        raise RuntimeError("the trusted gate did not merge the independently passing upper branch")
    _, upper_after_merge, _ = api("GET", upper_path)
    _, main_after_upper, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
    upper_merge_commit = upper_gate.get("merge_commit_sha")
    upper_base_sha = main_after_upper.get("commit", {}).get("id")
    if (
        upper_after_merge.get("state") != "closed"
        or upper_after_merge.get("merged") is not True
        or not isinstance(upper_merge_commit, str)
        or upper_base_sha != upper_merge_commit
    ):
        raise RuntimeError("upper semantic PR merge did not match Forgejo main read-back")

    update_status, update_response, _ = api(
        "POST", lower_path + "/update", {"style": "merge"}
    )
    write_json(
        evidence_dir / "lower-clean-merge-update.json",
        {
            "http_status": update_status,
            "response": update_response,
            "old_candidate_head": lower_initial_head,
            "base_commit_after_upper_merge": upper_base_sha,
        },
    )
    _, lower_branch_updated, _ = api(
        "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{quote(lower_branch, safe='')}"
    )
    lower_preview_head = lower_branch_updated.get("commit", {}).get("id")
    if not isinstance(lower_preview_head, str) or lower_preview_head == lower_initial_head:
        raise RuntimeError("Forgejo did not create a new candidate head for the clean merge preview")
    lower_pr, updated_observations = wait_for_updated_pr_mergeability(
        api,
        lower_path,
        expected_head_sha=lower_preview_head,
        expected_base_sha=upper_base_sha,
    )
    combined_source, _ = read_forgejo_file(api, "src/semantic.rs", lower_branch)
    if (
        update_status != 200
        or lower_pr.get("mergeable") is not True
        or lower_pr.get("base", {}).get("sha") != upper_base_sha
        or combined_source != fixture["combined"]
    ):
        raise RuntimeError("Forgejo clean merge preview is not the exact expected combined source")
    write_json(
        evidence_dir / "clean-merge-preview-binding.json",
        {
            "common_fixture_commit": common_commit,
            "upper_branch_commit": upper_head,
            "lower_preupdate_commit": lower_initial_head,
            "upper_merge_commit": upper_merge_commit,
            "preview_base_commit": upper_base_sha,
            "preview_candidate_commit": lower_preview_head,
            "pull_request_mergeable": lower_pr.get("mergeable"),
            "preview_source_sha256": hashlib.sha256(combined_source).hexdigest(),
            "expected_combined_source_sha256": hashlib.sha256(fixture["combined"]).hexdigest(),
            "source_exactly_matches_branch_union": True,
            "mergeability_observations": updated_observations,
        },
    )

    combined_lower_status, combined_lower_output = run_semantic_acceptance_sandbox(
        combined_source,
        lower_acceptance,
        state / "semantic-verify-combined-lower",
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
    )
    combined_upper_status, combined_upper_output = run_semantic_acceptance_sandbox(
        combined_source,
        upper_acceptance,
        state / "semantic-verify-combined-upper",
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
    )
    red_status, red_output = run_semantic_acceptance_sandbox(
        combined_source,
        protected_acceptance,
        state / "semantic-verify-protected-red",
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
    )
    write_private(evidence_dir / "combined-lower-acceptance.log", combined_lower_output)
    write_private(evidence_dir / "combined-upper-acceptance.log", combined_upper_output)
    write_private(evidence_dir / "protected-red-acceptance.log", red_output)
    if (
        combined_lower_status != 0
        or combined_upper_status != 0
        or b"1 passed; 0 failed" not in combined_lower_output
        or b"1 passed; 0 failed" not in combined_upper_output
        or red_status == 0
        or b"min > max" not in red_output
    ):
        raise RuntimeError("the composed branch checks or expected protected failure did not reproduce")

    no_review_binding = evidence_dir / "lower-no-review-binding.json"
    write_json(
        no_review_binding,
        {
            "schema_version": 1,
            "review_id": 1,
            "reviewer_id": reviewer_id,
            "reviewer_login": REVIEWER,
            "pull_request_number": lower_number,
            "base_branch": "main",
            "base_commit": upper_base_sha,
            "candidate_commit": lower_preview_head,
            "source": "no approval submitted for the failing semantic canary",
        },
    )
    fail_evidence = make_evidence(
        requirement=requirement_lower,
        candidate=lower_preview_head,
        base=upper_base_sha,
        source=combined_source,
        acceptance=protected_acceptance,
        output=red_output,
        outcome="fail",
        detail="The independent host-owned acceptance panicked with min > max on the exact Forgejo clean-merge preview.",
    )
    failed_proof, failed_expected, _, _ = make_semantic_target_proof(
        state=evidence_dir,
        gate=gate,
        key_path=key_path,
        evidence=fail_evidence,
        outcome="fail",
        name="combined-failing-proof",
    )
    fail_gate_status, fail_gate = run_local_test_merge_authority(
        client,
        number=lower_number,
        forgejo_pid=forgejo_pid,
        target_profile="merge-014-agent-forge-core",
        proof=failed_proof,
        expected=failed_expected,
        key=key_path,
        policy=policy_path,
        review_base_binding=no_review_binding,
        execution_identity="local-test-executor",
        host_token=merge_token,
        policy_token=policy_token,
    )
    write_json(evidence_dir / "combined-failure-gate-refusal.json", fail_gate)
    _, lower_after_refusal, _ = api("GET", lower_path)
    _, main_after_refusal, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
    main_after_refusal_sha = main_after_refusal.get("commit", {}).get("id")
    if (
        fail_gate_status == 0
        or fail_gate.get("verdict") != "rejected"
        or fail_gate.get("merge_attempted") is not False
        or fail_gate.get("error", {}).get("code") != "verification_failed"
        or lower_after_refusal.get("state") != "open"
        or lower_after_refusal.get("merged") is not False
        or main_after_refusal_sha != upper_base_sha
    ):
        raise RuntimeError("the merge gate did not refuse the exact failing preview while leaving main unchanged")
    write_json(
        evidence_dir / "combined-failure-state-after-refusal.json",
        {
            "pull_request_state": lower_after_refusal.get("state"),
            "merged": lower_after_refusal.get("merged"),
            "main_commit_before_refusal": upper_base_sha,
            "main_commit_after_refusal": main_after_refusal_sha,
        },
    )

    corrected_source = fixture["corrected"]
    write_forgejo_file(
        api,
        path="src/semantic.rs",
        ref=lower_branch,
        content=corrected_source,
        message="Reject an empty effective range before clamping",
    )
    corrected_branch_source, _ = read_forgejo_file(api, "src/semantic.rs", lower_branch)
    _, corrected_branch, _ = api(
        "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{quote(lower_branch, safe='')}"
    )
    corrected_head = corrected_branch.get("commit", {}).get("id")
    lower_pr, corrected_observations = wait_for_updated_pr_mergeability(
        api,
        lower_path,
        expected_head_sha=corrected_head,
        expected_base_sha=upper_base_sha,
    )
    green_status, green_output = run_semantic_acceptance_sandbox(
        corrected_branch_source,
        protected_acceptance,
        state / "semantic-verify-protected-green",
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
    )
    write_private(evidence_dir / "protected-green-acceptance.log", green_output)
    if (
        corrected_branch_source != corrected_source
        or lower_pr.get("mergeable") is not True
        or green_status != 0
        or b"1 passed; 0 failed" not in green_output
    ):
        raise RuntimeError("the correction did not pass protected acceptance on the refreshed PR head")
    write_json(
        evidence_dir / "corrected-pr-binding.json",
        {
            "base_commit": upper_base_sha,
            "candidate_commit": corrected_head,
            "source_sha256": hashlib.sha256(corrected_branch_source).hexdigest(),
            "protected_acceptance_exit_code": green_status,
            "mergeable": lower_pr.get("mergeable"),
            "mergeability_observations": corrected_observations,
        },
    )

    final_evidence = make_evidence(
        requirement=requirement_lower,
        candidate=corrected_head,
        base=upper_base_sha,
        source=corrected_branch_source,
        acceptance=protected_acceptance,
        output=green_output,
        outcome="pass",
        detail="The independent protected acceptance passed on the corrected and refreshed Forgejo PR head.",
    )
    final_proof, final_expected, _, _ = make_semantic_target_proof(
        state=evidence_dir,
        gate=gate,
        key_path=key_path,
        evidence=final_evidence,
        outcome="pass",
        name="corrected-proof",
    )
    final_review_binding = create_review_binding(
        api=api,
        reviewer_api=reviewer_api,
        pr_path=lower_path,
        pull_number=lower_number,
        base_sha=upper_base_sha,
        head_sha=corrected_head,
        reviewer_id=reviewer_id,
        evidence_dir=evidence_dir,
        name="corrected",
    )
    final_gate_status, final_gate = run_local_test_merge_authority(
        client,
        number=lower_number,
        forgejo_pid=forgejo_pid,
        target_profile="merge-014-agent-forge-core",
        proof=final_proof,
        expected=final_expected,
        key=key_path,
        policy=policy_path,
        review_base_binding=final_review_binding,
        execution_identity="local-test-executor",
        host_token=merge_token,
        policy_token=policy_token,
    )
    write_json(evidence_dir / "corrected-merge-gate.json", final_gate)
    if (
        final_gate_status != 0
        or final_gate.get("verdict") != "merged"
        or final_gate.get("merge_attempted") is not True
        or final_gate.get("submitted_head_sha") != corrected_head
        or final_gate.get("validated_base_sha") != upper_base_sha
    ):
        raise RuntimeError("trusted gate did not merge the corrected semantic candidate")
    _, final_pr, _ = api("GET", lower_path)
    _, final_main, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
    _, final_issue, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}")
    postmerge_source, _ = read_forgejo_file(api, "src/semantic.rs", "main")
    postmerge_status, postmerge_output = run_semantic_acceptance_sandbox(
        postmerge_source,
        protected_acceptance,
        state / "semantic-verify-protected-postmerge",
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
    )
    write_private(evidence_dir / "postmerge-protected-acceptance.log", postmerge_output)
    if (
        final_pr.get("state") != "closed"
        or final_pr.get("merged") is not True
        or final_pr.get("head", {}).get("sha") != corrected_head
        or final_pr.get("merge_commit_sha") != final_gate.get("merge_commit_sha")
        or final_main.get("commit", {}).get("id") != final_gate.get("merge_commit_sha")
        or final_issue.get("state") != "closed"
        or postmerge_source != corrected_branch_source
        or postmerge_status != 0
        or b"1 passed; 0 failed" not in postmerge_output
    ):
        raise RuntimeError("corrected semantic PR read-back or post-merge behavior did not match the verified candidate")

    result = {
        "schema_version": 1,
        "local_fixture_only": True,
        "llm_invoked": False,
        "forgejo_version": "17.0.0+gitea-1.22.0",
        "issue_number": issue_number,
        "common_fixture_commit": common_commit,
        "initial_main_commit": initial_main_sha,
        "upper_pull_request": upper_number,
        "upper_candidate_commit": upper_head,
        "upper_merge_commit": upper_merge_commit,
        "lower_pull_request": lower_number,
        "lower_initial_candidate_commit": lower_initial_head,
        "clean_preview_base_commit": upper_base_sha,
        "clean_preview_candidate_commit": lower_preview_head,
        "clean_preview_tree_source_sha256": hashlib.sha256(combined_source).hexdigest(),
        "independent_branch_checks": {"lower": "passed", "upper": "passed"},
        "combined_branch_checks": {"lower": "passed", "upper": "passed"},
        "protected_failure": {
            "exit_code": red_status,
            "actionable_marker": "min > max",
            "gate_verdict": fail_gate.get("verdict"),
            "gate_error": fail_gate.get("error", {}).get("code"),
            "merge_attempted": fail_gate.get("merge_attempted"),
            "pull_request_remained_open": lower_after_refusal.get("state") == "open",
            "main_unchanged": main_after_refusal_sha == upper_base_sha,
        },
        "correction": {
            "source_sha256": hashlib.sha256(corrected_branch_source).hexdigest(),
            "protected_acceptance": "passed",
            "gate_verdict": final_gate.get("verdict"),
            "merge_commit": final_gate.get("merge_commit_sha"),
            "postmerge_acceptance": "passed",
            "issue_closed": final_issue.get("state") == "closed",
        },
        "limitations": [
            "Fixture verifier and local Forgejo only; no LLM, AgentOS, conventional CI, or productivity benchmark.",
            "The semantic test case is deterministic and does not establish general improvement.",
            "Forgejo process/build source commit is inherited from the pinned local binary record; no Forgejo source was modified.",
        ],
    }
    write_json(evidence_dir / "result.json", result)
    return result


def build_local_binaries(
    repo: Path,
    state: Path,
    evidence_dir: Path,
    *,
    bwrap: str,
    rust_sysroot: Path,
    include_webhook_receiver: bool = False,
) -> tuple[Path, Path]:
    """Compile trusted local executables in a no-network sandbox."""
    if repo != process_e2e.REPO.resolve(strict=True):
        raise RuntimeError("isolated build helper must target the runner's own repository")
    source_stage = state / "cargo-source"
    registry_stage = state / "cargo-registry"
    artifacts_dir = state / "rust-artifacts"
    artifacts_dir.mkdir(mode=0o700)
    process_e2e.copy_source_tree(source_stage)
    cargo_home = Path(
        os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
    ).resolve(strict=True)
    process_e2e.copy_locked_registry(cargo_home / "registry", registry_stage)

    binary_names = ["gate", "forgejo-reconcile-local"]
    if include_webhook_receiver:
        binary_names.append("forgejo-webhook-local")

    helper = r'''import json, os, shutil, subprocess, sys
names = json.loads(sys.argv[1])
os.makedirs("/tmp/cargo-home/registry", exist_ok=True)
shutil.copytree("/bootstrap/registry/index", "/tmp/cargo-home/registry/index")
shutil.copytree("/bootstrap/registry/cache", "/tmp/cargo-home/registry/cache")
build_command = ["/toolchain/bin/cargo", "build", "--offline", "--locked"]
for name in names:
    build_command.extend(["--bin", name])
process = subprocess.Popen(
    build_command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
)
tail = bytearray()
while chunk := process.stdout.read(65536):
    tail.extend(chunk)
    if len(tail) > 65536:
        del tail[:-65536]
status = process.wait()
if tail:
    sys.stderr.buffer.write(tail)
    sys.stderr.buffer.flush()
if status:
    raise SystemExit(status)
total = 0
for name in names:
    source = f"/build/target/debug/{name}"
    if not os.path.isfile(source) or not os.access(source, os.X_OK):
        raise SystemExit(f"Cargo binary target missing or not executable: {name}")
    size = os.path.getsize(source)
    if size > 67108864:
        raise SystemExit(f"binary target exceeds per-file artifact limit: {name}")
    total += size
    shutil.copyfile(source, f"/artifacts/{name}")
if total > 134217728:
    raise SystemExit("total exported binaries exceed 128 MiB")
for name in names:
    path = f"/artifacts/{name}"
    size = os.path.getsize(path)
    sys.stdout.buffer.write(f"ARTIFACT {name} {size}\n".encode())
    with open(path, "rb") as source:
        shutil.copyfileobj(source, sys.stdout.buffer)
    sys.stdout.buffer.flush()
'''
    command = process_e2e.sandbox_prefix(
        Path(bwrap),
        source_stage,
        rust_sysroot,
        Path("/home"),
        registry_stage,
    )
    command.extend(
        [
            "--size",
            "134217728",
            "--tmpfs",
            "/artifacts",
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
            json.dumps(binary_names),
        ]
    )
    artifacts = {
        "gate": artifacts_dir / "gate",
        "forgejo-reconcile-local": artifacts_dir / "forgejo-reconcile-local",
    }
    if include_webhook_receiver:
        artifacts["forgejo-webhook-local"] = artifacts_dir / "forgejo-webhook-local"
    build_log = io.StringIO()
    try:
        with redirect_stdout(build_log):
            process_e2e.run_artifact_build(command, cwd=repo, artifacts=artifacts)
    except Exception as error:
        write_private(evidence_dir / "cargo-build.log", build_log.getvalue().encode())
        raise RuntimeError("offline isolated build of the trusted Forgejo executables failed") from error
    write_private(evidence_dir / "cargo-build.log", build_log.getvalue().encode())
    version_env = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(state / "home"),
        "TMPDIR": str(state / "tmp"),
    }
    versions = {}
    for name, executable in (
        ("rustc", rust_sysroot / "bin/rustc"),
        ("cargo", rust_sysroot / "bin/cargo"),
    ):
        version = subprocess.run(
            [str(executable), "--version"],
            cwd="/",
            env=version_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=10,
        )
        if version.returncode:
            raise RuntimeError(f"could not record the staged toolchain version for {name}")
        versions[name] = version.stdout.strip()
    write_json(
        evidence_dir / "trusted-build-provenance.json",
        {
            "schema_version": 1,
            "build_policy": "Bubblewrap no-network namespace, --offline --locked, isolated staged inputs",
            "runner_sha256": process_e2e.sha256_file(Path(__file__).resolve()),
            "process_harness_sha256": process_e2e.sha256_file(Path(process_e2e.__file__).resolve()),
            "staged_source_tree_sha256": process_e2e.sha256_tree(source_stage),
            "staged_registry_tree_sha256": process_e2e.sha256_tree(registry_stage),
            "gate_sha256": process_e2e.sha256_file(artifacts["gate"]),
            "forgejo_client_sha256": process_e2e.sha256_file(artifacts["forgejo-reconcile-local"]),
            "forgejo_webhook_receiver_sha256": (
                process_e2e.sha256_file(artifacts["forgejo-webhook-local"])
                if include_webhook_receiver
                else None
            ),
            "toolchain_versions": versions,
            "bubblewrap_version": subprocess.run(
                [bwrap, "--version"],
                cwd="/",
                env=version_env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=10,
                check=True,
            ).stdout.strip(),
        },
    )
    return artifacts["gate"], artifacts["forgejo-reconcile-local"]


def make_proof(
    *,
    state: Path,
    gate: Path,
    key_path: Path,
    requirement_sha: str,
    policy_sha: str,
    base_sha: str,
    head_sha: str,
    source: bytes,
    changed_paths: list[str],
    worker_sha: str,
    verification_status: int,
    verification_output: bytes,
    bwrap: str,
    name: str,
) -> tuple[Path, Path]:
    required_results = (
        b"below_min_returns_min ... ok",
        b"above_max_returns_max ... ok",
        b"in_range_is_preserved ... ok",
        b"equal_lower_bound_is_preserved ... ok",
        b"equal_upper_bound_is_preserved ... ok",
        b"singleton_interval_clamps ... ok",
        b"negative_interval_clamps ... ok",
        b"i32_min_clamps ... ok",
        b"i32_max_clamps ... ok",
        b"9 passed; 0 failed",
    )
    if verification_status != 0 or any(
        marker not in verification_output for marker in required_results
    ):
        raise RuntimeError("refusing to sign an incomplete or failed acceptance result")
    evidence_path = state / "evidence" / f"{name}-evidence.json"
    expected_path = state / "evidence" / f"{name}-expected.json"
    proof_path = state / "evidence" / f"{name}-proof.json"
    evidence = {
        "schema_version": 1,
        "requirement_sha256": requirement_sha,
        "policy_sha256": policy_sha,
        "candidate_commit": head_sha,
        "base_commit": base_sha,
        "merge_commit": None,
        "verifier_id": "mission-012-verify-local",
        "environment": "local Forgejo v17.0.0 and Bubblewrap test verifier",
        "commands": [
            ["rustc", "--edition=2024", "/workspace/worker.rs", "-o", "/build/worker"],
            ["run", "/worker VALUE MIN MAX in a fresh Bubblewrap namespace for each of nine cases"],
        ],
        "artifact_sha256": [
            hashlib.sha256(source).hexdigest(),
            worker_sha,
            hashlib.sha256(verification_output).hexdigest(),
            hashlib.sha256("\n".join(sorted(changed_paths)).encode()).hexdigest(),
        ],
        "outcome": "pass",
        "detail": (
            "All nine host-owned acceptance cases passed against source fetched by the pinned Forgejo PR head commit; "
            "the candidate worker ran separately from the host verifier. The complete PR-files API read-back listed: "
            + ", ".join(sorted(changed_paths))
            + "."
        ),
    }
    expected = {
        "requirement_sha256": requirement_sha,
        "policy_sha256": policy_sha,
        "candidate_commit": head_sha,
        "base_commit": base_sha,
        "merge_commit": None,
        "verifier_id": "mission-012-verify-local",
    }
    write_json(evidence_path, evidence)
    write_json(expected_path, expected)
    helper = r'''import os, shutil, subprocess, sys
signed = subprocess.run(
    ["/gate", "sign", "/key", "/evidence.json", "/artifacts/proof.json"],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
if signed.returncode:
    sys.stderr.write("sandboxed proof signing failed\n")
    raise SystemExit(signed.returncode)
checked = subprocess.run(
    ["/gate", "check", "/key", "/artifacts/proof.json", "/expected.json"],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
if checked.returncode:
    sys.stderr.write("sandboxed proof self-check failed\n")
    raise SystemExit(checked.returncode)
size = os.path.getsize("/artifacts/proof.json")
if size > 1048576:
    raise SystemExit("signed proof exceeds 1 MiB")
sys.stdout.buffer.write(f"ARTIFACT proof {size}\n".encode())
with open("/artifacts/proof.json", "rb") as source:
    shutil.copyfileobj(source, sys.stdout.buffer)
sys.stdout.buffer.flush()
'''
    sign_command = [
        bwrap,
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
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--size",
        "33554432",
        "--tmpfs",
        "/home",
        "--size",
        "33554432",
        "--tmpfs",
        "/tmp",
        "--size",
        "2097152",
        "--tmpfs",
        "/artifacts",
        "--ro-bind",
        str(gate),
        "/gate",
        "--ro-bind",
        str(key_path),
        "/key",
        "--ro-bind",
        str(evidence_path),
        "/evidence.json",
        "--ro-bind",
        str(expected_path),
        "/expected.json",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/tmp",
        "--setenv",
        "TMPDIR",
        "/tmp",
        "--chdir",
        "/tmp",
        "/usr/bin/prlimit",
        "--cpu=5",
        "--as=268435456",
        "--nproc=16",
        "--nofile=32",
        "--fsize=1048576",
        "--",
        "/usr/bin/python3",
        "-c",
        helper,
    ]
    with redirect_stdout(io.StringIO()):
        process_e2e.run_artifact_build(
            sign_command,
            cwd=state,
            artifacts={"proof": proof_path},
        )
    proof_path.chmod(0o600)
    proof_document = json.loads(proof_path.read_bytes())
    if (
        set(proof_document) != {"evidence", "mac_sha256"}
        or proof_document.get("evidence") != evidence
        or not isinstance(proof_document.get("mac_sha256"), str)
        or len(proof_document["mac_sha256"]) != 64
    ):
        raise RuntimeError("sandboxed signer returned a malformed or mismatched proof")
    canonical = json.dumps(
        evidence, ensure_ascii=False, separators=(",", ":")
    ).encode("utf-8")
    expected_mac = hmac.new(key_path.read_bytes(), canonical, hashlib.sha256).hexdigest()
    if not hmac.compare_digest(proof_document["mac_sha256"], expected_mac):
        raise RuntimeError("sandboxed signer returned an invalid proof MAC")
    return proof_path, expected_path


def make_target_bound_proof(
    *,
    state: Path,
    gate: Path,
    key_path: Path,
    v1_proof_path: Path,
    expected_path: Path,
    name: str,
) -> tuple[Path, Path]:
    """Convert host-verified evidence into the target-bound V2 merge proof."""
    v1_document = json.loads(v1_proof_path.read_bytes())
    evidence = v1_document.get("evidence")
    if not isinstance(evidence, dict):
        raise RuntimeError("the independent verifier did not produce evidence for V2 signing")
    target = {
        "profile_selector": "merge-014-agent-forge-core",
        "origin": ORIGIN,
        "owner": OWNER,
        "repository": REPOSITORY,
        "base_branch": "main",
    }
    target_path = state / "evidence" / f"{name}-target.json"
    proof_path = state / "evidence" / f"{name}-proof-v2.json"
    write_json(target_path, target)
    helper = r'''import os, shutil, subprocess, sys
signed = subprocess.run(
    ["/gate", "sign-target-v2", "/key", "/evidence.json", "/target.json", "/artifacts/proof.json"],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
if signed.returncode:
    sys.stderr.write("sandboxed V2 proof signing failed\n")
    raise SystemExit(signed.returncode)
checked = subprocess.run(
    ["/gate", "check-target-v2", "/key", "/artifacts/proof.json", "/target.json", "/expected.json"],
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
if checked.returncode:
    sys.stderr.write("sandboxed V2 proof self-check failed\n")
    raise SystemExit(checked.returncode)
size = os.path.getsize("/artifacts/proof.json")
if size > 1048576:
    raise SystemExit("signed V2 proof exceeds 1 MiB")
sys.stdout.buffer.write(f"ARTIFACT proof {size}\n".encode())
with open("/artifacts/proof.json", "rb") as source:
    shutil.copyfileobj(source, sys.stdout.buffer)
sys.stdout.buffer.flush()
'''
    sign_command = [
        "/usr/bin/bwrap",
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
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--size",
        "33554432",
        "--tmpfs",
        "/home",
        "--size",
        "33554432",
        "--tmpfs",
        "/tmp",
        "--size",
        "2097152",
        "--tmpfs",
        "/artifacts",
        "--ro-bind",
        str(gate),
        "/gate",
        "--ro-bind",
        str(key_path),
        "/key",
        "--ro-bind",
        str(v1_proof_path.parent / f"{name}-evidence.json"),
        "/evidence.json",
        "--ro-bind",
        str(target_path),
        "/target.json",
        "--ro-bind",
        str(expected_path),
        "/expected.json",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/tmp",
        "--setenv",
        "TMPDIR",
        "/tmp",
        "--chdir",
        "/tmp",
        "/usr/bin/prlimit",
        "--cpu=5",
        "--as=268435456",
        "--nproc=16",
        "--nofile=32",
        "--fsize=1048576",
        "--",
        "/usr/bin/python3",
        "-c",
        helper,
    ]
    with redirect_stdout(io.StringIO()):
        process_e2e.run_artifact_build(
            sign_command,
            cwd=state,
            artifacts={"proof": proof_path},
        )
    proof_path.chmod(0o600)
    proof_document = json.loads(proof_path.read_bytes())
    if (
        set(proof_document) != {"schema_version", "target", "evidence", "target_mac_sha256"}
        or proof_document.get("schema_version") != 2
        or proof_document.get("target") != target
        or proof_document.get("evidence") != evidence
        or not isinstance(proof_document.get("target_mac_sha256"), str)
        or len(proof_document["target_mac_sha256"]) != 64
    ):
        raise RuntimeError("sandboxed signer returned a malformed or mismatched V2 proof")
    payload = {
        "schema_version": 2,
        "target": proof_document["target"],
        "evidence": proof_document["evidence"],
    }
    canonical = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    expected_mac = hmac.new(
        key_path.read_bytes(),
        b"agent-forge/target-bound-proof/v2\0" + canonical,
        hashlib.sha256,
    ).hexdigest()
    if not hmac.compare_digest(proof_document["target_mac_sha256"], expected_mac):
        raise RuntimeError("sandboxed signer returned an invalid V2 target MAC")
    return proof_path, target_path


def run_preflight(
    client: Path,
    *,
    number: int,
    proof: Path,
    expected: Path,
    key: Path,
) -> tuple[int, dict[str, Any]]:
    status, output = bounded_process(
        [
            str(client),
            "--preflight",
            str(number),
            str(proof),
            str(expected),
            str(key),
            "agent-fixture",
        ],
        timeout=40,
    )
    try:
        result = json.loads(output)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"preflight returned invalid JSON: {error}") from None
    return status, result


def wait_for_updated_pr_mergeability(
    api: Callable[[str, str], tuple[int, dict[str, Any], dict[str, str]]],
    pr_path: str,
    *,
    expected_head_sha: str,
    expected_base_sha: str,
    timeout_seconds: float = 30,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Wait through Forgejo's asynchronous Checking → Mergeable transition."""
    started = time.monotonic()
    stable_since: float | None = None
    saw_pending_state = False
    observations: list[dict[str, Any]] = []
    last_pr: dict[str, Any] = {}
    while time.monotonic() - started < timeout_seconds:
        _, current_pr, _ = api("GET", pr_path)
        last_pr = current_pr
        current_head = current_pr.get("head", {}).get("sha")
        current_base = current_pr.get("base", {}).get("sha")
        mergeable = current_pr.get("mergeable")
        observations.append(
            {
                "elapsed_ms": round((time.monotonic() - started) * 1000),
                "head_sha": current_head,
                "base_sha": current_base,
                "mergeable": mergeable,
            }
        )
        if (
            current_pr.get("state") == "open"
            and current_head == expected_head_sha
            and current_base == expected_base_sha
        ):
            if mergeable is False:
                saw_pending_state = True
                stable_since = None
            elif mergeable is True:
                if stable_since is None:
                    stable_since = time.monotonic()
                required_stability = 0.5 if saw_pending_state else 3.0
                if time.monotonic() - stable_since >= required_stability:
                    return current_pr, observations
            else:
                stable_since = None
        else:
            stable_since = None
        time.sleep(0.25)
    raise RuntimeError(
        "Forgejo did not produce a stable mergeable read-back for the updated PR head "
        f"(head={last_pr.get('head', {}).get('sha')!r}, "
        f"base={last_pr.get('base', {}).get('sha')!r}, "
        f"mergeable={last_pr.get('mergeable')!r})"
    )


def wait_for_pull_request_webhook(
    journal: Path,
    *,
    pull_request_number: int,
    action: str,
    expected_merged: bool,
    expected_head_sha: str,
    expected_merge_commit_sha: str | None = None,
    timeout_seconds: float = 20,
) -> dict[str, Any]:
    """Read one Forgejo-emitted PR event from the local receiver inbox."""
    import run_forgejo_live_webhook_reconcile_emitted as webhook_e2e

    started = time.monotonic()
    while time.monotonic() - started < timeout_seconds:
        records = webhook_e2e.read_journal_records(journal)
        matches: list[dict[str, Any]] = []
        for record in records:
            envelope = record.get("payload") if isinstance(record, dict) else None
            payload = envelope.get("payload") if isinstance(envelope, dict) else None
            pull = payload.get("pull_request") if isinstance(payload, dict) else None
            if (
                record.get("kind") == "forgejo.webhook.v1.pull_request"
                and isinstance(payload, dict)
                and payload.get("action") == action
                and isinstance(pull, dict)
                and pull.get("number") == pull_request_number
            ):
                parsed = webhook_e2e.parse_live_pull_request_delivery_record(
                    record,
                    owner=OWNER,
                    repository=REPOSITORY,
                    pull_request_number=pull_request_number,
                    expected_action=action,
                    expected_merged=expected_merged,
                )
                if parsed.get("head_sha") != expected_head_sha:
                    raise RuntimeError("Forgejo PR webhook head differs from the protected candidate")
                if expected_merge_commit_sha is not None and (
                    parsed.get("merge_commit_sha") != expected_merge_commit_sha
                ):
                    raise RuntimeError("Forgejo PR webhook merge commit differs from gate read-back")
                matches.append(parsed)
        if len(matches) > 1:
            raise RuntimeError("Forgejo emitted duplicate matching PR webhook records")
        if matches:
            return matches[0]
        time.sleep(0.1)
    raise RuntimeError(f"Forgejo did not deliver the expected pull_request/{action} event")


def wait_for_webhook_task_receipt(
    state: Path, hook_id: int, delivery_id: str, *, timeout_seconds: float = 10
) -> dict[str, Any]:
    """Bind the receiver UUID to Forgejo's successful 202 task read-back."""
    import run_forgejo_live_webhook_reconcile_emitted as webhook_e2e

    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        tasks = webhook_e2e.hook_task_readback(state / "data/forgejo.db", hook_id)
        matches = [task for task in tasks if task.get("uuid") == delivery_id]
        if len(matches) > 1:
            raise RuntimeError("Forgejo has duplicate hook-task rows for one delivery UUID")
        if matches:
            task = matches[0]
            if (
                task.get("is_delivered") is True
                and task.get("is_succeed") is True
                and task.get("response_status") == 202
                and task.get("event_type") == "pull_request"
            ):
                return task
        time.sleep(0.1)
    raise RuntimeError("Forgejo did not record successful HTTP 202 for the expected delivery")


def validate_webhook_replay_options(
    *, observe: bool, replay: bool, restart_before_replay: bool
) -> None:
    if replay and not observe:
        raise RuntimeError("--replay-pr-webhook requires --observe-pr-webhook")
    if restart_before_replay and not replay:
        raise RuntimeError("--restart-pr-webhook-before-replay requires --replay-pr-webhook")


def validate_semantic_canary_options(
    *,
    semantic: bool,
    observe: bool,
    replay: bool,
    restart_before_replay: bool,
    sandbox_egress: bool,
    stdio_mcp_egress: bool,
) -> None:
    if semantic and any(
        (observe, replay, restart_before_replay, sandbox_egress, stdio_mcp_egress)
    ):
        raise RuntimeError(
            "--semantic-clean-merge-canary cannot be combined with webhook or egress compositions"
        )


def stop_webhook_receiver_for_restart(
    receiver: subprocess.Popen[bytes], *, timeout_seconds: float = 10
) -> dict[str, int | str | bool]:
    if receiver.poll() is not None:
        raise RuntimeError("webhook receiver exited before its controlled restart")
    process_id = receiver.pid
    receiver.terminate()
    try:
        return_code = receiver.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        try:
            receiver.kill()
        except ProcessLookupError:
            pass
        receiver.wait(timeout=5)
        raise RuntimeError("webhook receiver did not stop gracefully before restart") from None
    if return_code != 0:
        raise RuntimeError(f"webhook receiver returned {return_code} during graceful restart")
    return {
        "stopped": True,
        "stop_signal": "SIGTERM",
        "exit_code": return_code,
        "process_id": process_id,
    }


def restart_webhook_receiver_for_replay(
    receiver: subprocess.Popen[bytes],
    receiver_log: BinaryIO,
    *,
    binary: Path,
    state: Path,
    address: str,
    secret_path: Path,
    attestation_seed_path: Path,
    journal: Path,
    delivery_id: str,
) -> tuple[subprocess.Popen[bytes], BinaryIO, dict[str, Any]]:
    """Restart the receiver over its durable inbox before replaying a native event."""
    import run_forgejo_live_webhook_reconcile_emitted as webhook_e2e

    if not re.fullmatch(r"127\.0\.0\.1:[0-9]{1,5}", address):
        raise RuntimeError("webhook receiver restart requires the exact loopback address")
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", delivery_id):
        raise RuntimeError("webhook receiver restart delivery ID is invalid")
    before = journal.read_bytes()
    before_records = webhook_e2e.read_journal_records(journal)
    matches = [
        record for record in before_records
        if isinstance(record.get("payload"), dict)
        and record["payload"].get("delivery_id") == delivery_id
    ]
    if len(matches) != 1:
        raise RuntimeError("native delivery is not unique in the journal before restart")

    stopped = stop_webhook_receiver_for_restart(receiver)
    receiver_log.flush()
    receiver_log.close()
    restarted_log_path = state / "log/webhook-receiver-after-restart.log"
    restarted_log = restarted_log_path.open("xb")
    restarted: subprocess.Popen[bytes] | None = None
    try:
        port = int(address.rsplit(":", 1)[1])
        restarted = subprocess.Popen(
            [str(binary), address, str(secret_path), str(journal), str(attestation_seed_path)],
            cwd=state,
            env=webhook_e2e.helpers.private_command_environment(state),
            stdin=subprocess.DEVNULL,
            stdout=restarted_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        webhook_e2e.helpers.wait_for_listener(restarted, port)
        after = journal.read_bytes()
        after_records = webhook_e2e.read_journal_records(journal)
        after_matches = [
            record for record in after_records
            if isinstance(record.get("payload"), dict)
            and record["payload"].get("delivery_id") == delivery_id
        ]
        if after != before or len(after_records) != len(before_records) or len(after_matches) != 1:
            raise RuntimeError("receiver restart altered or lost its accepted journal delivery")
        if restarted.pid == stopped["process_id"]:
            raise RuntimeError("receiver restart did not create a distinct process")
        return restarted, restarted_log, {
            "before_process": stopped,
            "after_process_id": restarted.pid,
            "same_loopback_address": True,
            "same_private_secret_path": True,
            "journal_reused": True,
            "journal_unchanged_during_restart": True,
            "journal_sha256_before": hashlib.sha256(before).hexdigest(),
            "journal_sha256_after": hashlib.sha256(after).hexdigest(),
            "journal_record_count_before": len(before_records),
            "journal_record_count_after": len(after_records),
            "delivery_id_recovered_once": True,
            "delivery_id": delivery_id,
        }
    except Exception:
        if restarted is not None and restarted.poll() is None:
            try:
                restarted.terminate()
                restarted.wait(timeout=10)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    restarted.kill()
                except ProcessLookupError:
                    pass
                restarted.wait(timeout=5)
        restarted_log.close()
        raise


def run_single_webhook_reconciliation(
    client: Path,
    state: Path,
    journal: Path,
    delivery_id: str,
    read_only_credential: str,
    trusted_attestation_public_key: str,
) -> tuple[int, dict[str, Any]]:
    """Reconcile one real webhook using a private credential pipe."""
    if not re.fullmatch(r"[0-9a-f]{64}", trusted_attestation_public_key):
        raise ValueError("trusted receiver attestation key must be canonical lowercase Ed25519 hex")
    encoded = read_only_credential.encode("utf-8")
    read_fd, write_fd = os.pipe()
    try:
        view = memoryview(encoded)
        while view:
            written = os.write(write_fd, view)
            if written <= 0:
                raise OSError("short write to reconciliation credential pipe")
            view = view[written:]
    finally:
        os.close(write_fd)
    try:
        completed = subprocess.run(
            [
                str(client),
                "--target-profile",
                "merge-014-agent-forge-core",
                "--trusted-webhook-attestation-public-key-hex",
                trusted_attestation_public_key,
                "--host-credential-fd",
                str(read_fd),
                str(journal),
                delivery_id,
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={
                "PATH": "/usr/bin:/bin",
                "HOME": str(state / "home"),
                "TMPDIR": str(state / "tmp"),
            },
            pass_fds=(read_fd,),
            timeout=20,
            check=False,
        )
    finally:
        os.close(read_fd)
    if encoded in completed.stdout or encoded in completed.stderr:
        raise RuntimeError("webhook reconciliation CLI exposed its private credential")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError:
        raise RuntimeError("webhook reconciliation CLI did not return structured JSON") from None
    if not isinstance(value, dict):
        raise RuntimeError("webhook reconciliation CLI returned a non-object result")
    return completed.returncode, value


def build_signed_webhook_duplicate_replay(
    record: dict[str, Any], secret: str
) -> dict[str, Any]:
    """Build an in-memory, re-signed delivery from one accepted journal record."""
    if not isinstance(secret, str) or not secret:
        raise ValueError("webhook secret must be non-empty")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal record sequence is invalid")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal record envelope is invalid")
    event_name = envelope.get("event")
    delivery_id = envelope.get("delivery_id")
    payload = envelope.get("payload")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or event_name != "pull_request"
        or not isinstance(delivery_id, str)
        or re.fullmatch(r"[A-Za-z0-9_-]{1,128}", delivery_id) is None
        or record.get("kind") != "forgejo.webhook.v1.pull_request"
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
        or not isinstance(payload, dict)
    ):
        raise ValueError("journal record delivery ID binding or pull-request payload is invalid")
    try:
        secret_bytes = secret.encode("ascii")
        raw_body = json.dumps(
            payload,
            ensure_ascii=False,
            allow_nan=False,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
    except (UnicodeEncodeError, ValueError, TypeError):
        raise ValueError("webhook secret or journal payload cannot be encoded") from None
    if not secret_bytes:
        raise ValueError("webhook secret must be non-empty")
    if len(raw_body) > 512 * 1024:
        raise ValueError("journaled webhook payload exceeds the receiver body limit")
    signature = hmac.new(secret_bytes, raw_body, hashlib.sha256).hexdigest()
    return {
        "raw_body": raw_body,
        "event_name": event_name,
        "delivery_id": delivery_id,
        "sequence": sequence,
        "signature": signature,
    }


def replay_accepted_pr_webhook_duplicate(
    journal: Path,
    receiver_target: str,
    delivery_id: str,
    secret: str,
    *,
    pull_request_number: int,
    expected_head_sha: str,
    expected_merge_commit_sha: str,
) -> dict[str, Any]:
    """Replay the accepted native payload and require an idempotent 202."""
    import run_forgejo_live_webhook_reconcile_emitted as webhook_e2e

    parsed_target = urlsplit(receiver_target)
    if (
        parsed_target.scheme != "http"
        or parsed_target.hostname != "127.0.0.1"
        or parsed_target.port is None
        or parsed_target.path != "/forgejo/webhook"
        or parsed_target.username is not None
        or parsed_target.password is not None
        or parsed_target.query
        or parsed_target.fragment
    ):
        raise RuntimeError("duplicate replay target must be the local Forgejo webhook receiver")

    before = journal.read_bytes()
    records = webhook_e2e.read_journal_records(journal)
    matches = [
        record
        for record in records
        if isinstance(record.get("payload"), dict)
        and record["payload"].get("delivery_id") == delivery_id
    ]
    if len(matches) != 1:
        raise RuntimeError("accepted webhook delivery is absent or duplicated in the receiver journal")
    record = matches[0]
    parsed = webhook_e2e.parse_live_pull_request_delivery_record(
        record,
        owner=OWNER,
        repository=REPOSITORY,
        pull_request_number=pull_request_number,
        expected_action="closed",
        expected_merged=True,
    )
    if (
        parsed.get("delivery_id") != delivery_id
        or parsed.get("head_sha") != expected_head_sha
        or parsed.get("merge_commit_sha") != expected_merge_commit_sha
    ):
        raise RuntimeError("journaled native event differs from the accepted PR merge binding")

    replay = build_signed_webhook_duplicate_replay(record, secret)
    opener = build_opener(ProxyHandler({}), NoRedirect)
    request = Request(
        receiver_target,
        data=replay["raw_body"],
        headers={
            "Accept": "application/json",
            "Content-Type": "application/json",
            "X-Forgejo-Delivery": replay["delivery_id"],
            "X-Forgejo-Event": replay["event_name"],
            "X-Forgejo-Signature": replay["signature"],
        },
        method="POST",
    )
    try:
        with opener.open(request, timeout=5) as response:
            status = response.status
            response_body = response.read(16 * 1024 + 1)
    except HTTPError as error:
        raise RuntimeError(f"duplicate webhook replay returned HTTP {error.code}") from None
    except URLError:
        raise RuntimeError("duplicate webhook replay did not reach the local receiver") from None
    if len(response_body) > 16 * 1024:
        raise RuntimeError("duplicate replay response exceeded the size limit")
    try:
        result = json.loads(response_body)
    except json.JSONDecodeError:
        raise RuntimeError("duplicate replay receiver response was not JSON") from None

    after = journal.read_bytes()
    after_records = webhook_e2e.read_journal_records(journal)
    if after != before or len(after_records) != len(records):
        raise RuntimeError("idempotent webhook replay changed the receiver journal")
    if (
        status != 202
        or not isinstance(result, dict)
        or result.get("accepted") is not True
        or result.get("duplicate") is not True
        or result.get("delivery_id") != delivery_id
        or result.get("sequence") != replay["sequence"]
        or result.get("reconcile_required") is not True
    ):
        raise RuntimeError("receiver did not acknowledge the same delivery as an idempotent duplicate")
    return {
        "delivery_id": delivery_id,
        "sequence": replay["sequence"],
        "http_status": status,
        "accepted": True,
        "duplicate": True,
        "reconcile_required": True,
        "journal_unchanged": True,
        "journal_record_count": len(after_records),
        "payload_sha256": hashlib.sha256(replay["raw_body"]).hexdigest(),
        "secret_recorded": False,
        "replay_kind": "test-driver re-signing of the accepted native Forgejo payload",
    }


def run_local_test_merge_authority(
    client: Path,
    *,
    number: int,
    forgejo_pid: int,
    target_profile: str,
    proof: Path,
    expected: Path,
    key: Path,
    policy: Path,
    review_base_binding: Path,
    execution_identity: str,
    host_token: str,
    policy_token: str | None = None,
) -> tuple[int, dict[str, Any]]:
    """Invoke the gate with private credentials and a process-bound test capability."""
    tokens = [("host", host_token)]
    if policy_token is not None:
        tokens.append(("policy", policy_token))
    descriptors: list[tuple[str, int, int, bytes]] = []
    supervisor_socket, gate_socket = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    start_read_fd, start_write_fd = os.pipe2(os.O_CLOEXEC)
    launch_record: dict[str, Any] | None = None
    try:
        for label, token in tokens:
            token_bytes = token.encode("ascii")
            if not token_bytes or len(token_bytes) > 4096 or b"\n" in token_bytes:
                raise RuntimeError(f"Forgejo returned an invalid private {label} token")
            read_fd, write_fd = os.pipe()
            descriptors.append((label, read_fd, write_fd, token_bytes))
            offset = 0
            while offset < len(token_bytes):
                written = os.write(write_fd, token_bytes[offset:])
                if written <= 0:
                    raise RuntimeError(f"could not deliver the private {label} token")
                offset += written
            os.close(write_fd)
            descriptors[-1] = (label, read_fd, -1, token_bytes)
        command = [
            str(client),
            "--merge-local-test",
            "--writer-admission-profile",
            "local_single_writer_sandbox",
            "--supervisor-capability-fd",
            str(gate_socket.fileno()),
            "--target-profile",
            target_profile,
            str(number),
            str(proof),
            str(expected),
            str(key),
            str(policy),
            str(review_base_binding),
            execution_identity,
            str(descriptors[0][1]),
        ]
        pass_fds = [descriptors[0][1]]
        pass_fds.append(gate_socket.fileno())
        pass_fds.append(start_read_fd)
        if policy_token is not None:
            command.extend(["--policy-credential-fd", str(descriptors[1][1])])
            pass_fds.append(descriptors[1][1])
        if any(token in command for _, token in tokens):
            raise RuntimeError("private credential was placed in CLI arguments")

        exec_after_capability = (
            "import os,sys; fd=int(sys.argv[1]); "
            "ready=os.read(fd,1); os.close(fd); "
            "sys.exit(125) if ready != b'1' else None; "
            "argv=sys.argv[2:]; "
            "os.execve(argv[0],argv,{'PATH':'/usr/bin:/bin','HOME':'/nonexistent',"
            "'TMPDIR':'/tmp','LC_ALL':'C'})"
        )
        launch_command = [
            "/usr/bin/python3",
            "-I",
            "-c",
            exec_after_capability,
            str(start_read_fd),
            *command,
        ]

        def issue_supervisor_capability(process: subprocess.Popen[bytes]) -> None:
            nonlocal launch_record, start_write_fd
            packet, record = build_supervisor_capability_packet(
                gate_pid=process.pid,
                forgejo_pid=forgejo_pid,
                target_profile=target_profile,
                pull_request_number=number,
            )
            gate_socket.close()
            if supervisor_socket.send(packet) != len(packet):
                raise RuntimeError("supervisor capability packet was not sent atomically")
            if os.write(start_write_fd, b"1") != 1:
                raise RuntimeError("supervisor did not release the capability-bound gate")
            os.close(start_write_fd)
            start_write_fd = -1
            record["capability_sent_atomically"] = True
            record["capability_sent_before_gate_exec"] = True
            launch_record = record

        status, output = bounded_process(
            launch_command,
            env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "TMPDIR": "/tmp", "LC_ALL": "C"},
            timeout=60,
            pass_fds=tuple(pass_fds),
            after_spawn=issue_supervisor_capability,
        )
    finally:
        for _, read_fd, write_fd, _ in descriptors:
            os.close(read_fd)
            if write_fd >= 0:
                os.close(write_fd)
        gate_socket.close()
        supervisor_socket.close()
        os.close(start_read_fd)
        if start_write_fd >= 0:
            os.close(start_write_fd)
    if any(token_bytes in output for _, _, _, token_bytes in descriptors):
        raise RuntimeError("Rust merge authority exposed a private credential in output")
    try:
        result = json.loads(output)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"local merge authority returned invalid JSON: {error}") from None
    admission = result.get("writer_admission") if isinstance(result, dict) else None
    if (
        launch_record is None
        or not isinstance(admission, dict)
        or admission.get("profile") != "local_single_writer_sandbox"
        or admission.get("target") != launch_record["target"]
        or admission.get("supervisor") != launch_record["supervisor"]
        or admission.get("supervisor_uid") != launch_record["supervisor_uid"]
        or admission.get("gate") != launch_record["gate"]
        or admission.get("forgejo") != launch_record["forgejo"]
        or admission.get("supervisor_policy_independently_verified") is not False
        or admission.get("pre_post_revalidated") is not (result.get("merge_attempted") is True)
    ):
        raise RuntimeError("Rust merge authority did not attest the issued live test capability")
    result["test_host_supervisor_issuance"] = launch_record
    return status, result


def read_sandbox_probe_event(process: subprocess.Popen[bytes]) -> dict[str, Any]:
    if process.stdout is None:
        raise RuntimeError("held sandbox probe has no status pipe")
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        if not selector.select(10):
            raise RuntimeError("held sandbox probe did not emit its bounded status event")
    line = process.stdout.readline(64 * 1024)
    if not line or len(line) >= 64 * 1024 or not line.endswith(b"\n"):
        raise RuntimeError("held sandbox probe returned an invalid status frame")
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        raise RuntimeError("held sandbox probe returned invalid JSON") from None
    if not isinstance(event, dict):
        raise RuntimeError("held sandbox probe status was not an object")
    return event


def close_sandbox_probe(process: subprocess.Popen[bytes]) -> None:
    sandbox_probe.stop_process_group(process)
    for stream in (process.stdin, process.stdout, process.stderr):
        if stream is not None:
            stream.close()


def validate_direct_route_probe_errors(value: object) -> None:
    """Reject route probes that report failure after connecting to the host."""
    expected_names = ["api", "web", "git_receive_pack"]
    if not isinstance(value, list) or len(value) != len(expected_names):
        raise RuntimeError("route probe diagnostics must cover all direct routes")
    expected_keys = {"name", "phase", "error_category", "errno"}
    for expected_name, diagnostic in zip(expected_names, value):
        if (
            not isinstance(diagnostic, dict)
            or set(diagnostic) != expected_keys
            or diagnostic.get("name") != expected_name
        ):
            raise RuntimeError("route probe diagnostics must identify all direct routes")
        if diagnostic.get("phase") != "connect":
            raise RuntimeError("direct route must be blocked at connect stage")
        code = diagnostic.get("errno")
        if (
            diagnostic.get("error_category") != "os_error"
            or type(code) is not int
            or code <= 0
        ):
            raise RuntimeError("connect-stage refusal must include a positive OS error number")


def start_sandbox_probe_during_merge(
    state: Path,
    *,
    owner: str,
    repository: str,
    pull_number: int,
) -> tuple[subprocess.Popen[bytes], dict[str, Any]]:
    if not sandbox_probe.BWRAP.is_file() or not sandbox_probe.PYTHON.is_file():
        raise RuntimeError("local Bubblewrap or Python runtime is unavailable")
    credential_path = state / "sandbox-egress-synthetic-canary.txt"
    write_private(credential_path, sandbox_probe.SYNTHETIC_CREDENTIAL)
    loopback_probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        loopback_probe.bind(("127.0.0.1", 0))
        loopback_port = loopback_probe.getsockname()[1]
    finally:
        loopback_probe.close()

    command = sandbox_probe.bubblewrap_command(
        credential_path,
        loopback_port,
        owner=owner,
        repository=repository,
        pull_number=pull_number,
        probe_only=True,
        probe_hold_open=True,
    )
    process = subprocess.Popen(
        command,
        cwd="/",
        env=sandbox_probe.HOST_ENV,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=0,
        close_fds=True,
        start_new_session=True,
    )
    try:
        event = read_sandbox_probe_event(process)
        expected_routes = [
            {"name": name, "method": method, "path": path}
            for name, method, path in test_agent_adapter.forgejo_route_probe_specs(
                owner, repository, pull_number
            )
        ]
        checks = event.get("sandbox_checks")
        if not isinstance(checks, dict):
            raise RuntimeError("held sandbox probe omitted its capability observations")
        validate_direct_route_probe_errors(checks.get("direct_forgejo_route_probe_errors"))
        comparable_checks = {
            key: value
            for key, value in checks.items()
            if key != "direct_forgejo_route_probe_errors"
        }
        if (
            event.get("type") != "sandbox_egress_probe_ready"
            or comparable_checks != {
                "credential_visible": False,
                "network_connected": False,
                "host_systemd_bus_visible": False,
                "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
                "open_fds": [0, 1, 2],
            }
            or event.get("route_probes") != expected_routes
            or event.get("open_fds") != [0, 1, 2]
            or not isinstance(event.get("network_namespace"), str)
            or event.get("network_namespace") == os.readlink("/proc/self/ns/net")
            or process.poll() is not None
        ):
            raise RuntimeError("held sandbox probe did not establish the expected isolation")
        return process, event
    except BaseException:
        close_sandbox_probe(process)
        raise


def finish_sandbox_probe_after_merge(
    process: subprocess.Popen[bytes], ready_event: dict[str, Any]
) -> dict[str, Any]:
    if process.stdin is None:
        raise RuntimeError("held sandbox probe has no command pipe")
    process.stdin.write(b"reprobe\n")
    process.stdin.flush()
    event = read_sandbox_probe_event(process)
    if (
        event.get("type") != "sandbox_egress_probe_after_gate"
        or event.get("sandbox_checks") != ready_event.get("sandbox_checks")
        or event.get("route_probes") != ready_event.get("route_probes")
        or event.get("network_namespace") != ready_event.get("network_namespace")
        or process.poll() is not None
    ):
        raise RuntimeError("sandbox route access changed or process exited after the gate")
    process.stdin.write(b"release\n")
    process.stdin.flush()
    released = read_sandbox_probe_event(process)
    if released != {"type": "sandbox_egress_probe_released"}:
        raise RuntimeError("held sandbox probe did not acknowledge release")
    if process.wait(timeout=5) != 0:
        raise RuntimeError("held sandbox probe did not exit cleanly")
    if process.stderr is None:
        raise RuntimeError("held sandbox probe has no diagnostic pipe")
    stderr = process.stderr.read(64 * 1024 + 1)
    if len(stderr) > 64 * 1024 or stderr:
        raise RuntimeError("held sandbox probe emitted unexpected diagnostics")
    return event


def run_gate_with_active_sandbox_probe(
    *,
    state: Path,
    owner: str,
    repository: str,
    pull_number: int,
    host_route_check: Callable[[], tuple[int, dict[str, Any], dict[str, str]]],
    gate_call: Callable[[], tuple[int, dict[str, Any]]],
    evidence_path: Path,
) -> tuple[int, dict[str, Any]]:
    process, ready = start_sandbox_probe_during_merge(
        state,
        owner=owner,
        repository=repository,
        pull_number=pull_number,
    )
    summary: dict[str, Any] = {
        "schema_version": 1,
        "target": {"owner": owner, "repository": repository, "pull_number": pull_number},
        "sandbox_ready": ready,
        "synthetic_credential_only": True,
        "actual_gate_credential_passed_to_sandbox": False,
        "host_route_control": None,
        "process_alive_before_gate": process.poll() is None,
        "process_alive_after_gate": None,
        "gate": None,
        "after_gate": None,
        "probe_released": False,
        "probe_exit_code": None,
        "probe_stderr_empty": None,
    }
    gate_error: BaseException | None = None
    probe_error: BaseException | None = None
    gate_status: int | None = None
    gate_result: dict[str, Any] | None = None
    try:
        host_status, host_pr, _ = host_route_check()
        summary["host_route_control"] = {
            "status": host_status,
            "pull_request_number": host_pr.get("number"),
            "accessible": host_status == 200 and host_pr.get("number") == pull_number,
        }
        if host_status != 200 or host_pr.get("number") != pull_number:
            raise RuntimeError("host route positive control failed while the sandbox probe was held")
        gate_status, gate_result = gate_call()
        summary["process_alive_after_gate"] = process.poll() is None
        summary["gate"] = {
            "process_exit_code": gate_status,
            "verdict": gate_result.get("verdict"),
            "merge_attempted": gate_result.get("merge_attempted"),
            "merge_commit_sha": gate_result.get("merge_commit_sha"),
        }
        if process.poll() is not None:
            raise RuntimeError("sandbox probe exited while the trusted merge gate ran")
    except BaseException as error:
        gate_error = error

    try:
        after_gate = finish_sandbox_probe_after_merge(process, ready)
        summary["after_gate"] = after_gate
        summary["probe_released"] = True
        summary["probe_exit_code"] = process.returncode
        summary["probe_stderr_empty"] = True
    except BaseException as error:
        probe_error = error
        summary["probe_error_type"] = type(error).__name__
    finally:
        if process.poll() is not None and process.stderr is not None:
            summary["probe_exit_code"] = process.returncode
        close_sandbox_probe(process)
    write_json(evidence_path, summary)
    if probe_error is not None:
        raise probe_error
    if gate_error is not None:
        raise gate_error
    if gate_status is None or gate_result is None:
        raise RuntimeError("trusted merge gate produced no result")
    return gate_status, gate_result


def run_gate_with_active_stdio_mcp_task(
    *,
    state: Path,
    evidence_path: Path,
    owner: str,
    repository: str,
    pull_number: int,
    branch: str,
    branch_before: str,
    expected_head_sha: str,
    expected_base_sha: str,
    gate_token: str,
    gate_token_scope: str,
    host_route_check: Callable[[], tuple[int, dict[str, Any], dict[str, str]]],
    gate_call: Callable[[], tuple[int, dict[str, Any]]],
    gate_state_provider: Callable[[], tuple[str, str]] | None = None,
    initial_source: str | None = None,
    acceptance_source: str | None = None,
    expected_fixed_source: str | None = None,
    actionable_failure_markers: tuple[str, ...] | None = None,
    webhook_attestation_seed_path: Path | None = None,
) -> tuple[int, dict[str, Any], dict[str, Any]]:
    """Hold the real stdio-MCP task at its post-acceptance barrier across merge."""
    import run_mission_012_stdio_mcp_egress_gate_e2e as stdio_mcp_e2e

    task_state = state / "stdio-mcp-merge-task"
    task_state.mkdir(mode=0o700)
    task_evidence = evidence_path.parent / "stdio-mcp-task"
    task_evidence.mkdir(mode=0o700)
    fixture_arguments: dict[str, Any] = {}
    if initial_source is not None:
        fixture_arguments["initial_source"] = initial_source
    if acceptance_source is not None:
        fixture_arguments["acceptance_source"] = acceptance_source
    workspace, cargo_home, task_home, calls = stdio_mcp_e2e.prepare_fixture(
        task_state, **fixture_arguments
    )
    cargo_path = Path(
        subprocess.run(
            ["rustup", "which", "cargo"],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=5,
        ).stdout.strip()
    ).resolve(strict=True)
    rustc_path = Path(
        subprocess.run(
            ["rustup", "which", "rustc"],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=5,
        ).stdout.strip()
    ).resolve(strict=True)
    credential_fd = os.memfd_create("forgejo-stdio-mcp-merge-credential", flags=os.MFD_CLOEXEC)
    credential_bytes = gate_token.encode("ascii")
    gate_outcome: tuple[int, dict[str, Any]] | None = None
    try:
        offset = 0
        while offset < len(credential_bytes):
            written = os.write(credential_fd, credential_bytes[offset:])
            if written <= 0:
                raise RuntimeError("trusted merge credential memfd write was incomplete")
            offset += written
        credential_probe_path = f"/proc/{os.getpid()}/fd/{credential_fd}"

        def read_target_branch() -> str:
            status, current_branch, _ = make_api(gate_token)(
                "GET", f"/repos/{owner}/{repository}/branches/{branch}"
            )
            commit = current_branch.get("commit") if isinstance(current_branch, dict) else None
            sha = commit.get("id") if isinstance(commit, dict) else None
            if status != 200 or not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{40,64}", sha):
                raise RuntimeError("trusted merge credential could not read the intended base branch")
            return sha

        def merge_while_task_waits() -> dict[str, Any]:
            nonlocal gate_outcome
            gate_head_sha, gate_base_sha = (
                gate_state_provider()
                if gate_state_provider is not None
                else (expected_head_sha, expected_base_sha)
            )
            host_status, current_pr, _ = host_route_check()
            if (
                host_status != 200
                or current_pr.get("number") != pull_number
                or current_pr.get("state") != "open"
                or current_pr.get("merged") is not False
                or current_pr.get("head", {}).get("sha") != gate_head_sha
                or current_pr.get("base", {}).get("sha") != gate_base_sha
            ):
                raise RuntimeError("trusted host PR read-back changed before the merge while MCP task waited")
            status, result = gate_call()
            if (
                status != 0
                or result.get("verdict") != "merged"
                or result.get("merge_authorized") is not True
                or result.get("merge_attempted") is not True
                or result.get("submitted_head_sha") != gate_head_sha
                or result.get("validated_base_sha") != gate_base_sha
                or result.get("main_head_sha") != result.get("merge_commit_sha")
                or result.get("postmerge_base_sha") != result.get("main_head_sha")
            ):
                raise RuntimeError("trusted merge authority failed while stdio-MCP task waited")
            gate_outcome = (status, result)
            return {
                "process_exit_code": status,
                "verdict": result.get("verdict"),
                "merge_attempted": result.get("merge_attempted"),
                "http_status": result.get("http_status"),
                "submitted_head_sha": result.get("submitted_head_sha"),
                "validated_base_sha": result.get("validated_base_sha"),
                "merge_commit_sha": result.get("merge_commit_sha"),
                "main_head_sha": result.get("main_head_sha"),
            }

        task_result = stdio_mcp_e2e.run_task_at_barrier(
            state=task_state,
            evidence=task_evidence,
            workspace=workspace,
            cargo_home=cargo_home,
            task_home=task_home,
            calls=calls,
            gate_token=gate_token,
            gate_token_scope=gate_token_scope,
            credential_probe_path=credential_probe_path,
            branch=branch,
            branch_before=branch_before,
            cargo=cargo_path,
            toolchain_bin=rustc_path.parent,
            wait_at_gate_barrier=True,
            gate_callback=merge_while_task_waits,
            expected_fixed_source=(
                expected_fixed_source
                if expected_fixed_source is not None
                else stdio_mcp_e2e.FIXED_SOURCE
            ),
            actionable_failure_markers=(
                actionable_failure_markers
                if actionable_failure_markers is not None
                else ("left: 12", "right: 10")
            ),
            branch_reader=read_target_branch,
            probe_owner=owner,
            probe_repository=repository,
            probe_pull_number=pull_number,
            private_file_probes=(
                [("webhook_attestation_seed", webhook_attestation_seed_path)]
                if webhook_attestation_seed_path is not None
                else None
            ),
        )
        if not task_result.get("mcp_barrier", {}).get("released_after_gate_operation"):
            raise RuntimeError("the post-acceptance MCP gate barrier did not release after merge")
        if task_result.get("trusted_gate_operation", {}).get("verdict") != "merged":
            raise RuntimeError("the actual trusted gate POST was not recorded at the MCP barrier")
        if gate_outcome is None:
            raise RuntimeError("the merge gate callback returned no authoritative result")
        write_json(evidence_path, task_result)
        return gate_outcome[0], gate_outcome[1], task_result
    finally:
        try:
            os.ftruncate(credential_fd, 0)
        finally:
            os.close(credential_fd)


def run(args: argparse.Namespace) -> dict[str, Any]:
    binary_argument = Path(args.forgejo_binary)
    if binary_argument.is_symlink():
        raise RuntimeError("--forgejo-binary must not be a symlink")
    binary = binary_argument.resolve(strict=True)
    repo = Path(args.repo_root).resolve(strict=True)
    state = Path(args.state_dir)
    if not state.is_absolute() or state.parent != Path("/tmp"):
        raise RuntimeError("--state-dir must be a new direct child of /tmp")
    if state.exists():
        raise RuntimeError(f"refusing to reuse existing test state: {state}")
    observe_pr_webhook = getattr(args, "observe_pr_webhook", False)
    replay_pr_webhook = getattr(args, "replay_pr_webhook", False)
    restart_pr_webhook_before_replay = getattr(
        args, "restart_pr_webhook_before_replay", False
    )
    validate_webhook_replay_options(
        observe=observe_pr_webhook,
        replay=replay_pr_webhook,
        restart_before_replay=restart_pr_webhook_before_replay,
    )
    semantic_clean_merge_canary = getattr(args, "semantic_clean_merge_canary", False)
    validate_semantic_canary_options(
        semantic=semantic_clean_merge_canary,
        observe=observe_pr_webhook,
        replay=replay_pr_webhook,
        restart_before_replay=restart_pr_webhook_before_replay,
        sandbox_egress=getattr(args, "sandbox_egress_during_merge", False),
        stdio_mcp_egress=getattr(args, "stdio_mcp_egress_during_merge", False),
    )
    if not binary.is_file():
        raise RuntimeError("--forgejo-binary must resolve to a regular local file")
    if not (repo / "Cargo.toml").is_file():
        raise RuntimeError("--repo-root is not the Rust project")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3130)) == 0:
            raise RuntimeError("127.0.0.1:3130 is already in use; refusing to start")

    state.mkdir(mode=0o700)
    create_app_ini(state, allow_loopback_webhooks=observe_pr_webhook)
    evidence_dir = state / "evidence"
    version_run = subprocess.run(
        [str(binary), "--version"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=10,
    )
    if version_run.returncode or "17.0.0+gitea-1.22.0" not in version_run.stdout:
        raise RuntimeError("this harness requires the locally verified Forgejo 17.0.0 binary")
    rustc_program = shutil.which("rustc")
    if rustc_program is None:
        raise RuntimeError("rustc is required; candidate code was not executed")
    rust_env = {
        "PATH": f"{Path(rustc_program).parent}:/usr/bin:/bin",
        "HOME": str(Path.home()),
        "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
    }
    if os.environ.get("RUSTUP_TOOLCHAIN"):
        rust_env["RUSTUP_TOOLCHAIN"] = os.environ["RUSTUP_TOOLCHAIN"]
    sysroot_output = subprocess.run(
        [rustc_program, "--print", "sysroot"],
        cwd="/",
        env=rust_env,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        timeout=10,
    ).stdout.strip()
    rust_sysroot = Path(sysroot_output).resolve(strict=True)
    bwrap = "/usr/bin/bwrap"
    if not Path(bwrap).is_file() or Path(bwrap).is_symlink():
        raise RuntimeError("Bubblewrap is required; candidate code was not executed")
    if not Path("/usr/bin/prlimit").is_file():
        raise RuntimeError("prlimit is required; candidate code was not executed")
    gate, client = build_local_binaries(
        repo,
        state,
        evidence_dir,
        bwrap=bwrap,
        rust_sysroot=rust_sysroot,
        include_webhook_receiver=observe_pr_webhook,
    )

    server_log = (state / "log/server.log").open("wb")
    server_environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(state / "home"),
        "TMPDIR": str(state / "tmp"),
    }
    forgejo = subprocess.Popen(
        [
            str(binary),
            "--work-path",
            str(state),
            "--config",
            str(state / "custom/conf/app.ini"),
            "web",
        ],
        cwd=state,
        env=server_environment,
        stdout=server_log,
        stderr=subprocess.STDOUT,
    )
    stdio_mcp_thread: threading.Thread | None = None
    stdio_mcp_barrier_ready = threading.Event()
    stdio_mcp_gate_release = threading.Event()
    stdio_mcp_context: dict[str, Any] = {}
    webhook_e2e = None
    webhook_receiver: subprocess.Popen[bytes] | None = None
    webhook_receiver_log = None
    webhook_receiver_port: int | None = None
    webhook_journal = state / "data/webhook-inbox.jsonl"
    webhook_secret: str | None = None
    webhook_secret_path: Path | None = None
    webhook_hook_id: int | None = None
    webhook_target: str | None = None
    webhook_open_event: dict[str, Any] | None = None
    webhook_closed_event: dict[str, Any] | None = None
    webhook_duplicate_replay: dict[str, Any] | None = None
    webhook_receiver_restart: dict[str, Any] | None = None
    webhook_reconciliation: dict[str, Any] | None = None
    webhook_hook_readback: dict[str, Any] | None = None
    webhook_hook_disabled = False
    webhook_reconcile_token: str | None = None
    webhook_attestation_seed_path: Path | None = None
    webhook_attestation_public_key: str | None = None
    api: Callable[[str, str], tuple[int, dict[str, Any], dict[str, str]]] | None = None
    try:
        wait_for_health(forgejo)
        assert_forgejo_listener(forgejo)
        call_admin(
            binary,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                OWNER,
                "--random-password",
                "--email",
                f"{OWNER}@local.invalid",
                "--admin",
                "--must-change-password=false",
            ],
        )
        bootstrap_token = call_admin(
            binary,
            state,
            [
                "admin",
                "user",
                "generate-access-token",
                "--username",
                OWNER,
                "--token-name",
                "mission-e2e-repo-provisioning",
                "--raw",
                "--scopes",
                # Forgejo v17's /user/repos requires write:user; its own
                # api_repo_test.go proves write:repository alone is refused.
                "write:user,write:repository",
            ],
        ).decode().strip()
        if not bootstrap_token or any(character.isspace() for character in bootstrap_token):
            raise RuntimeError("Forgejo returned an invalid temporary repository-provisioning token")
        bootstrap_api = make_api(bootstrap_token)
        bootstrap_api("POST", "/user/repos", {
            "name": REPOSITORY,
            "description": "Isolated AI-forge acceptance test",
            "private": False,
            "auto_init": True,
            "default_branch": "main",
        })
        bootstrap_api = None
        bootstrap_token = None
        admin_token = call_admin(
            binary,
            state,
            [
                "admin",
                "user",
                "generate-access-token",
                "--username",
                OWNER,
                "--token-name",
                "mission-e2e-host",
                "--raw",
                "--scopes",
                "read:user,write:repository,write:issue",
            ],
        ).decode().strip()
        if not admin_token or any(character.isspace() for character in admin_token):
            raise RuntimeError("Forgejo returned an invalid temporary host token")
        api = make_api(admin_token)
        if observe_pr_webhook:
            import run_forgejo_live_webhook_reconcile_emitted as webhook_module

            webhook_e2e = webhook_module
            webhook_secret = secrets.token_hex(32)
            webhook_secret_path = state / "webhook.secret"
            write_private(webhook_secret_path, webhook_secret.encode("ascii"))
            webhook_attestation_seed_path = state / "webhook-attestation.seed"
            signing_seed = bytearray(secrets.token_bytes(32))
            try:
                write_private(webhook_attestation_seed_path, bytes(signing_seed))
            finally:
                signing_seed[:] = b"\x00" * len(signing_seed)
            webhook_receiver_port = webhook_e2e.helpers.select_receiver_port()
            receiver_address = f"127.0.0.1:{webhook_receiver_port}"
            webhook_target = f"http://{receiver_address}/forgejo/webhook"
            webhook_receiver_log = (state / "log/webhook-receiver.log").open("wb")
            receiver_binary = state / "rust-artifacts/forgejo-webhook-local"
            public_key_result = subprocess.run(
                [
                    str(receiver_binary),
                    "--print-attestation-public-key",
                    str(webhook_attestation_seed_path),
                ],
                cwd=state,
                env=webhook_e2e.helpers.private_command_environment(state),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=5,
                check=False,
            )
            webhook_attestation_public_key = public_key_result.stdout.strip()
            if public_key_result.returncode != 0 or not re.fullmatch(
                r"[0-9a-f]{64}", webhook_attestation_public_key
            ):
                raise RuntimeError("receiver did not derive a canonical attestation public key")
            write_json(
                evidence_dir / "webhook-attestation-policy.json",
                {
                    "schema_version": 1,
                    "algorithm": "Ed25519",
                    "trusted_public_key_sha256": hashlib.sha256(
                        bytes.fromhex(webhook_attestation_public_key)
                    ).hexdigest(),
                    "private_seed_file_mode": "0600",
                    "private_seed_storage": "host-only state directory beneath /tmp",
                    "agent_mount_boundary": "Bubblewrap replaces /tmp with a fresh tmpfs",
                },
            )
            webhook_receiver = subprocess.Popen(
                [
                    str(receiver_binary),
                    receiver_address,
                    str(webhook_secret_path),
                    str(webhook_journal),
                    str(webhook_attestation_seed_path),
                ],
                cwd=state,
                env=webhook_e2e.helpers.private_command_environment(state),
                stdin=subprocess.DEVNULL,
                stdout=webhook_receiver_log,
                stderr=subprocess.STDOUT,
                close_fds=True,
            )
            webhook_e2e.helpers.wait_for_listener(webhook_receiver, webhook_receiver_port)
            _, created_hook, _ = api(
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/hooks",
                {
                    "type": "gitea",
                    "config": {
                        "url": webhook_target,
                        "content_type": "json",
                        "secret": webhook_secret,
                    },
                    "events": ["pull_request"],
                    "active": True,
                },
            )
            webhook_hook_id = created_hook.get("id")
            if (
                not isinstance(webhook_hook_id, int)
                or isinstance(webhook_hook_id, bool)
                or webhook_hook_id < 1
                or created_hook.get("active") is not True
            ):
                raise RuntimeError("Forgejo did not create the expected active PR webhook")
            _, hook_readback, _ = api(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{webhook_hook_id}"
            )
            webhook_hook_readback = webhook_e2e.hook_readback_record(
                webhook_hook_id,
                hook_readback,
                webhook_target,
                expected_event_selectors=("pull_request",),
            )
            write_json(evidence_dir / "webhook-hook-active-readback.json", webhook_hook_readback)
            if webhook_hook_readback.get("configuration_matches_expected") is not True:
                raise RuntimeError("Forgejo PR webhook configuration differs from the loopback receiver")
            if webhook_hook_readback.get("active_after_run") is not True:
                raise RuntimeError("Forgejo PR webhook is not active after configuration read-back")
            webhook_reconcile_token = call_admin(
                binary,
                state,
                [
                    "admin",
                    "user",
                    "generate-access-token",
                    "--username",
                    OWNER,
                    "--token-name",
                    "mission-e2e-webhook-read-only",
                    "--raw",
                    "--scopes",
                    "read:user,read:repository,read:issue",
                ],
            ).decode().strip()
            if not webhook_reconcile_token or any(character.isspace() for character in webhook_reconcile_token):
                raise RuntimeError("Forgejo returned an invalid read-only webhook reconciliation token")
        call_admin(
            binary,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                REVIEWER,
                "--random-password",
                "--email",
                f"{REVIEWER}@local.invalid",
                "--must-change-password=false",
            ],
        )
        reviewer_token = call_admin(
            binary,
            state,
            [
                "admin",
                "user",
                "generate-access-token",
                "--username",
                REVIEWER,
                "--token-name",
                "mission-e2e-reviewer",
                "--raw",
                "--scopes",
                "read:user,write:repository",
            ],
        ).decode().strip()
        if not reviewer_token or any(character.isspace() for character in reviewer_token):
            raise RuntimeError("Forgejo returned an invalid temporary reviewer token")
        reviewer_api = make_api(reviewer_token)
        _, reviewer_account, _ = reviewer_api("GET", "/user")
        reviewer_id = reviewer_account.get("id")
        if not isinstance(reviewer_id, int) or reviewer_id <= 0:
            raise RuntimeError("Forgejo did not return the synthetic reviewer's numeric identity")
        merge_login = MERGER if args.non_admin_merger else OWNER
        if args.non_admin_merger:
            call_admin(
                binary,
                state,
                [
                    "admin",
                    "user",
                    "create",
                    "--username",
                    MERGER,
                    "--random-password",
                    "--email",
                    f"{MERGER}@local.invalid",
                    "--must-change-password=false",
                ],
            )
            merge_token = call_admin(
                binary,
                state,
                [
                    "admin",
                    "user",
                    "generate-access-token",
                    "--username",
                    MERGER,
                    "--token-name",
                    "mission-e2e-non-admin-merger",
                    "--raw",
                    "--scopes",
                    "read:user,read:issue,write:repository",
                ],
            ).decode().strip()
            if not merge_token or any(character.isspace() for character in merge_token):
                raise RuntimeError("Forgejo returned an invalid temporary non-admin merge token")
            merge_api = make_api(merge_token)
            _, merge_account, _ = merge_api("GET", "/user")
            if (
                merge_account.get("login") != MERGER
                or not isinstance(merge_account.get("id"), int)
                or merge_account.get("id", 0) <= 0
                or merge_account.get("is_admin") is not False
            ):
                raise RuntimeError("Forgejo did not confirm the dedicated merge account is non-admin")
            call_admin(
                binary,
                state,
                [
                    "admin",
                    "user",
                    "create",
                    "--username",
                    POLICY_READER,
                    "--random-password",
                    "--email",
                    f"{POLICY_READER}@local.invalid",
                    "--must-change-password=false",
                ],
            )
            policy_token = call_admin(
                binary,
                state,
                [
                    "admin",
                    "user",
                    "generate-access-token",
                    "--username",
                    POLICY_READER,
                    "--token-name",
                    "mission-e2e-policy-reader",
                    "--raw",
                    "--scopes",
                    "read:user,read:repository",
                ],
            ).decode().strip()
            if not policy_token or any(character.isspace() for character in policy_token):
                raise RuntimeError("Forgejo returned an invalid temporary policy-reader token")
            policy_api = make_api(policy_token)
            _, policy_reader_account, _ = policy_api("GET", "/user")
            if (
                policy_reader_account.get("login") != POLICY_READER
                or not isinstance(policy_reader_account.get("id"), int)
                or policy_reader_account.get("id", 0) <= 0
                or policy_reader_account.get("is_admin") is not False
            ):
                raise RuntimeError("Forgejo did not confirm the policy reader is not a site admin")
            api(
                "PUT",
                f"/repos/{OWNER}/{REPOSITORY}/collaborators/{POLICY_READER}",
                {"permission": "admin"},
            )
            _, policy_repo, _ = policy_api("GET", f"/repos/{OWNER}/{REPOSITORY}")
            permissions = policy_repo.get("permissions")
            if not isinstance(permissions, dict) or permissions.get("admin") is not True:
                raise RuntimeError("Forgejo did not confirm repository-admin read access for policy reader")
            policy_reader_repo_admin = True
        else:
            policy_token = None
            policy_api = None
            policy_reader_account = None
            policy_reader_repo_admin = False
            merge_token = admin_token
            merge_api = api
            merge_account = {"login": OWNER, "is_admin": True}
        policy_bytes = build_policy_bytes(reviewer_id)
        _, issue, _ = api(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {"title": "Implement inclusive integer clamp", "body": REQUIREMENT},
        )
        write_json(evidence_dir / "issue-created.json", issue)
        _, main, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
        base_sha = main["commit"]["id"]
        api(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/branches",
            {"new_branch_name": "agent/clamp", "old_branch_name": "main"},
        )
        stdio_mcp_fixture = (
            build_stdio_mcp_candidate_fixture()
            if getattr(args, "stdio_mcp_egress_during_merge", False)
            else None
        )
        initial_source = (
            stdio_mcp_fixture[0]
            if stdio_mcp_fixture is not None
            else (
                "pub fn clamp_inclusive(value: i32, min: i32, max: i32) -> i32 {\n"
                "    value // BUG: values above the range should clamp to maximum.\n"
                "}\n"
            ).encode()
        )
        api(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs",
            {
                "message": "Add clamp helper (first attempt)",
                "content": base64.b64encode(initial_source).decode(),
                "branch": "agent/clamp",
            },
        )
        _, committed, _ = api(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=agent/clamp",
        )
        actual_initial = base64.b64decode(committed["content"])
        if actual_initial != initial_source:
            raise RuntimeError("Forgejo source read-back differs from the initial candidate")

        stdio_mcp_enabled = getattr(args, "stdio_mcp_egress_during_merge", False)
        stdio_mcp_source: bytes | None = None
        if stdio_mcp_enabled:
            import run_mission_012_stdio_mcp_egress_gate_e2e as stdio_mcp_e2e

            if stdio_mcp_fixture is None:
                raise RuntimeError("stdio MCP candidate fixture was not prepared")
            _, expected_mcp_source, acceptance_source_bytes, actionable_failure_markers = (
                stdio_mcp_fixture
            )
            if expected_mcp_source == actual_initial:
                raise RuntimeError("Forgejo task source has no unique MCP-authorized correction marker")
            acceptance_source = acceptance_source_bytes.decode("utf-8")

            def wait_for_final_gate_state() -> tuple[str, str]:
                stdio_mcp_barrier_ready.set()
                if not stdio_mcp_gate_release.wait(timeout=300):
                    raise TimeoutError("the trusted merge gate did not release the stdio-MCP barrier")
                if stdio_mcp_context.get("cancelled"):
                    raise RuntimeError("the Forgejo PR run was cancelled before its merge gate")
                head = stdio_mcp_context.get("head_sha")
                base = stdio_mcp_context.get("base_sha")
                if not isinstance(head, str) or not isinstance(base, str):
                    raise RuntimeError("the final PR head/base were not published to the gate")
                return head, base

            def deferred_host_route_check() -> tuple[int, dict[str, Any], dict[str, str]]:
                callback = stdio_mcp_context.get("host_route_check")
                if not callable(callback):
                    raise RuntimeError("the Forgejo PR read callback is not ready")
                return callback()

            def deferred_gate_call() -> tuple[int, dict[str, Any]]:
                callback = stdio_mcp_context.get("gate_call")
                if not callable(callback):
                    raise RuntimeError("the trusted merge authority callback is not ready")
                return callback()

            def run_held_stdio_mcp_task() -> None:
                try:
                    stdio_mcp_context["result"] = run_gate_with_active_stdio_mcp_task(
                        state=state,
                        evidence_path=evidence_dir / "stdio-mcp-egress-during-merge.json",
                        owner=OWNER,
                        repository=REPOSITORY,
                        pull_number=int(issue["number"]) + 1,
                        branch="main",
                        branch_before=base_sha,
                        expected_head_sha="deferred-until-pr-proof-is-ready",
                        expected_base_sha=base_sha,
                        gate_token=merge_token,
                        gate_token_scope=(
                            "read:user,read:issue,write:repository"
                            if args.non_admin_merger
                            else "read:user,write:repository,write:issue"
                        ),
                        host_route_check=deferred_host_route_check,
                        gate_call=deferred_gate_call,
                        gate_state_provider=wait_for_final_gate_state,
                        initial_source=actual_initial.decode("utf-8"),
                        acceptance_source=acceptance_source,
                        expected_fixed_source=expected_mcp_source.decode("utf-8"),
                        actionable_failure_markers=actionable_failure_markers,
                        webhook_attestation_seed_path=webhook_attestation_seed_path,
                    )
                except BaseException as error:
                    stdio_mcp_context["error"] = error
                    stdio_mcp_barrier_ready.set()

            stdio_mcp_thread = threading.Thread(
                target=run_held_stdio_mcp_task,
                name="stdio-mcp-forgejo-pr-candidate",
            )
            stdio_mcp_thread.start()
            if not stdio_mcp_barrier_ready.wait(timeout=180):
                raise TimeoutError("the stdio-MCP task did not reach its protected-acceptance barrier")
            if "error" in stdio_mcp_context:
                raise RuntimeError("the stdio-MCP task failed before publishing its candidate source") from stdio_mcp_context["error"]
            task_source_path = (
                state / "stdio-mcp-merge-task" / "mission-workspace" / "src" / "lib.rs"
            )
            if task_source_path.is_symlink() or not task_source_path.is_file():
                raise RuntimeError("the held MCP task did not leave a regular candidate source file")
            stdio_mcp_source = task_source_path.read_bytes()
            if stdio_mcp_source != expected_mcp_source:
                raise RuntimeError("the source written by MCP differs from the requirement-derived correction")
            stdio_mcp_context["source_sha256"] = hashlib.sha256(stdio_mcp_source).hexdigest()

        red_dir = state / "verify-red"
        red_status, red_output, _ = run_candidate_sandbox(
            actual_initial, red_dir, bwrap=bwrap, rust_sysroot=rust_sysroot
        )
        write_private(evidence_dir / "red-verifier.log", red_output)
        validate_initial_candidate_verifier_result(
            red_status, red_output, stdio_mcp=stdio_mcp_enabled
        )

        fake_dir = state / "verify-fake-success"
        fake_status, fake_output, _ = run_candidate_sandbox(
            FAKE_SUCCESS_RS, fake_dir, bwrap=bwrap, rust_sysroot=rust_sysroot
        )
        write_private(evidence_dir / "fake-success-verifier.log", fake_output)
        if (
            fake_status == 0
            or b"FAILED" not in fake_output
            or b"0 passed; 9 failed" not in fake_output
        ):
            raise RuntimeError("the host verifier accepted an executor that exits zero without a result")

        # The egress mode reuses the bytes written by the held MCP process;
        # the default mode remains the deterministic local test adapter.
        corrected_source = (
            stdio_mcp_source
            if stdio_mcp_enabled
            else actual_initial.replace(
                b"value // BUG: values above the range should clamp to maximum.",
                b"value.clamp(min, max)",
                1,
            )
        )
        assert corrected_source is not None
        if corrected_source == actual_initial:
            raise RuntimeError("test adapter could not apply the verifier-directed correction")
        api(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs",
            {
                "message": "Fix clamp after verifier feedback",
                "content": base64.b64encode(corrected_source).decode(),
                "branch": "agent/clamp",
                "sha": committed["sha"],
            },
        )
        _, corrected, _ = api(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=agent/clamp",
        )
        source = base64.b64decode(corrected["content"])
        if source != corrected_source:
            raise RuntimeError("corrected candidate source differs from Forgejo read-back")
        green_dir = state / "verify-green"
        green_status, green_output, green_worker_sha = run_candidate_sandbox(
            source, green_dir, bwrap=bwrap, rust_sysroot=rust_sysroot
        )
        write_private(evidence_dir / "green-verifier.log", green_output)
        if green_status != 0 or b"9 passed; 0 failed" not in green_output:
            raise RuntimeError("corrected Forgejo candidate did not pass all protected acceptance tests")

        _, fixed_branch, _ = api(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/agent%2Fclamp"
        )
        fixed_head_sha = fixed_branch["commit"]["id"]
        key_path = evidence_dir / "verifier.key"
        write_private(key_path, secrets.token_bytes(32))
        requirement_sha = hashlib.sha256(REQUIREMENT.encode()).hexdigest()
        policy_sha = hashlib.sha256(policy_bytes).hexdigest()
        write_private(evidence_dir / "requirements.txt", REQUIREMENT.encode())
        write_private(evidence_dir / "policy.json", policy_bytes)
        proof0, expected0 = make_proof(
            state=state,
            gate=gate,
            key_path=key_path,
            requirement_sha=requirement_sha,
            policy_sha=policy_sha,
            base_sha=base_sha,
            head_sha=fixed_head_sha,
            source=source,
            changed_paths=["src/lib.rs"],
            worker_sha=green_worker_sha,
            verification_status=green_status,
            verification_output=green_output,
            bwrap=bwrap,
            name="proof-before-freshness-probe",
        )

        # Deliberately invalidate proof0, open the PR against that changed
        # source, then restore the exact bytes that the MCP process authored.
        _, content_for_probe, _ = api(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=agent/clamp",
        )
        probe_source = source + b"\n// stale proof probe\n"
        api(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs",
            {
                "message": "Change candidate to invalidate its previous proof",
                "content": base64.b64encode(probe_source).decode(),
                "branch": "agent/clamp",
                "sha": content_for_probe["sha"],
            },
        )
        _, pr, _ = api(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/pulls",
            {
                "title": "Implement inclusive integer clamp",
                "body": f"Closes #{issue['number']}. Implements the issue criteria.",
                "head": "agent/clamp",
                "base": "main",
            },
        )
        write_json(evidence_dir / "pull-request-created.json", pr)
        pr_number = int(pr["number"])
        pr_path = f"/repos/{OWNER}/{REPOSITORY}/pulls/{pr_number}"
        changed_files_at_creation = read_changed_files(api, pr_path)
        changed_paths = sorted(item["filename"] for item in changed_files_at_creation)
        if changed_paths != ["src/lib.rs"]:
            raise RuntimeError(
                f"this slice verifies only src/lib.rs; Forgejo reports changed paths {changed_paths!r}"
            )
        write_json(
            evidence_dir / "changed-files-at-creation.json",
            {"total_count": len(changed_paths), "filenames": changed_paths},
        )

        # Wait for Forgejo's mergeability computation instead of treating an
        # initial cached false/unknown value as final.
        for _ in range(60):
            _, current_pr, _ = api("GET", pr_path)
            if current_pr.get("mergeable") is True:
                break
            time.sleep(0.25)
        if current_pr.get("mergeable") is not True:
            raise RuntimeError(f"Forgejo never reported the PR mergeable: {current_pr.get('mergeable')!r}")
        head_sha = current_pr["head"]["sha"]
        _, live_branch, _ = api(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/agent%2Fclamp"
        )
        if head_sha != live_branch["commit"]["id"]:
            raise RuntimeError("Forgejo PR head differs from the source branch at creation")
        verified_head_sha = head_sha
        verified_base_sha = current_pr["base"]["sha"]
        code, stale = run_preflight(
            client,
            number=pr_number,
            proof=proof0,
            expected=expected0,
            key=key_path,
        )
        write_json(evidence_dir / "preflight-stale-candidate.json", stale)
        if code == 0 or stale.get("error", {}).get("code") != "stale_candidate":
            raise RuntimeError("a changed Forgejo candidate did not invalidate the old proof")

        _, content_after_probe, _ = api(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=agent/clamp",
        )
        probe_source_readback = base64.b64decode(content_after_probe["content"])
        if probe_source_readback != probe_source:
            raise RuntimeError("the stale-proof probe source differs from Forgejo branch read-back")
        api(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs",
            {
                "message": "Restore MCP-authored source after stale-proof probe",
                "content": base64.b64encode(corrected_source).decode(),
                "branch": "agent/clamp",
                "sha": content_after_probe["sha"],
            },
        )
        _, restored_branch, _ = api(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/agent%2Fclamp"
        )
        restored_head_sha = restored_branch["commit"]["id"]
        _, restored_contents, _ = api(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=agent/clamp",
        )
        source = base64.b64decode(restored_contents["content"])
        if source != corrected_source:
            raise RuntimeError("Forgejo did not restore the exact MCP-authored source bytes")
        current_pr, mergeability_observations = wait_for_updated_pr_mergeability(
            api,
            pr_path,
            expected_head_sha=restored_head_sha,
            expected_base_sha=verified_base_sha,
        )
        write_json(
            evidence_dir / "restored-pr-mergeability-poll.json",
            {
                "forgejo_async_check_transition_observed": any(
                    item.get("mergeable") is False
                    for item in mergeability_observations
                ),
                "stable_final_mergeable": current_pr.get("mergeable") is True,
                "observations": mergeability_observations,
            },
        )
        verified_head_sha = restored_head_sha
        verified_base_sha = current_pr["base"]["sha"]
        green_final_dir = state / "verify-green-final"
        final_status, final_output, final_worker_sha = run_candidate_sandbox(
            source, green_final_dir, bwrap=bwrap, rust_sysroot=rust_sysroot
        )
        write_private(evidence_dir / "final-verifier.log", final_output)
        if final_status != 0 or b"9 passed; 0 failed" not in final_output:
            raise RuntimeError("the final changed candidate failed independent acceptance")
        _, current_pr, _ = api("GET", pr_path)
        if (
            current_pr["head"]["sha"] != verified_head_sha
            or current_pr["base"]["sha"] != verified_base_sha
        ):
            raise RuntimeError("PR head or base changed after verification; refusing to bind proof to untested state")
        head_sha = verified_head_sha
        proof, expected = make_proof(
            state=state,
            gate=gate,
            key_path=key_path,
            requirement_sha=requirement_sha,
            policy_sha=policy_sha,
            base_sha=verified_base_sha,
            head_sha=head_sha,
            source=source,
            changed_paths=changed_paths,
            worker_sha=final_worker_sha,
            verification_status=final_status,
            verification_output=final_output,
            bwrap=bwrap,
            name="final-proof",
        )
        target_proof, target_path = make_target_bound_proof(
            state=state,
            gate=gate,
            key_path=key_path,
            v1_proof_path=proof,
            expected_path=expected,
            name="final-proof",
        )
        code, final_preflight = run_preflight(
            client, number=pr_number, proof=proof, expected=expected, key=key_path
        )
        if code != 0:
            _, failed_preflight_pr, _ = api("GET", pr_path)
            _, failed_preflight_branch, _ = api(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/agent%2Fclamp"
            )
            _, failed_preflight_base, _ = api(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main"
            )
            write_json(
                evidence_dir / "final-preflight-state-on-refusal.json",
                {
                    "client_error": final_preflight.get("error"),
                    "pull_request": {
                        "number": failed_preflight_pr.get("number"),
                        "state": failed_preflight_pr.get("state"),
                        "merged": failed_preflight_pr.get("merged"),
                        "mergeable": failed_preflight_pr.get("mergeable"),
                        "mergeable_state": failed_preflight_pr.get("mergeable_state"),
                        "head_sha": failed_preflight_pr.get("head", {}).get("sha"),
                        "base_sha": failed_preflight_pr.get("base", {}).get("sha"),
                        "updated_at": failed_preflight_pr.get("updated_at"),
                    },
                    "branch_head_sha": failed_preflight_branch.get("commit", {}).get("id"),
                    "base_branch_sha": failed_preflight_base.get("commit", {}).get("id"),
                    "expected_head_sha": head_sha,
                    "expected_base_sha": verified_base_sha,
                },
            )
        write_json(evidence_dir / "final-preflight-before-review.json", final_preflight)
        if (
            code
            or final_preflight.get("verdict") != "proof_current"
            or final_preflight.get("merge_authorized") is not False
            or final_preflight.get("merge_attempted") is not False
        ):
            raise RuntimeError("final live proof preflight did not pass read-only")

        api(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{REVIEWER}",
            {"permission": "write"},
        )
        if args.non_admin_merger:
            api(
                "PUT",
                f"/repos/{OWNER}/{REPOSITORY}/collaborators/{MERGER}",
                {"permission": "write"},
            )
            _, merge_repo, _ = merge_api("GET", f"/repos/{OWNER}/{REPOSITORY}")
            merge_permissions = merge_repo.get("permissions")
            if (
                not isinstance(merge_permissions, dict)
                or merge_permissions.get("admin") is not False
                or merge_permissions.get("push") is not True
            ):
                raise RuntimeError(
                    "Forgejo did not confirm the dedicated merge account has write without admin"
                )
            write_json(
                evidence_dir / "merge-account-role-readback.json",
                {
                    "login": merge_account.get("login"),
                    "is_admin": merge_account.get("is_admin"),
                    "repo_permission_admin": merge_permissions.get("admin"),
                    "repo_permission_push": merge_permissions.get("push"),
                    "token_scopes": ["read:user", "read:issue", "write:repository"],
                },
            )
        protection_body = {
            "rule_name": "main",
            "branch_name": "main",
            "apply_to_admins": True,
            "required_approvals": 1,
            "enable_approvals_whitelist": True,
            "approvals_whitelist_username": [REVIEWER],
            "dismiss_stale_approvals": True,
            "block_on_outdated_branch": True,
            "enable_merge_whitelist": True,
            "merge_whitelist_usernames": [merge_login],
            "enable_status_check": False,
            "enable_push": False,
            "unprotected_file_patterns": "",
        }
        api(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/branch_protections",
            protection_body,
        )
        _, protections, _ = api(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branch_protections"
        )
        matching = [item for item in protections if item.get("rule_name") == "main"]
        if len(matching) != 1:
            raise RuntimeError("Forgejo branch protection read-back did not identify exact rule main")
        protection = matching[0]
        write_json(evidence_dir / "branch-protection-readback.json", protection)
        if (
            protection.get("branch_name") != "main"
            or protection.get("required_approvals") != 1
            or protection.get("enable_approvals_whitelist") is not True
            or protection.get("approvals_whitelist_username") != [REVIEWER]
            or protection.get("enable_merge_whitelist") is not True
            or protection.get("merge_whitelist_usernames") != [merge_login]
            or protection.get("apply_to_admins") is not True
            or protection.get("dismiss_stale_approvals") is not True
            or protection.get("block_on_outdated_branch") is not True
            or protection.get("unprotected_file_patterns") != ""
        ):
            raise RuntimeError("Forgejo protection read-back differs from exact main-branch policy")

        policy_reader_scope_refusal_status: int | None = None
        if args.non_admin_merger:
            if policy_api is None or policy_reader_account is None or policy_token is None:
                raise RuntimeError("policy reader setup was incomplete")
            _, policy_readback, _ = policy_api(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/branch_protections"
            )
            policy_matching = [
                item for item in policy_readback if item.get("rule_name") == "main"
            ]
            if len(policy_matching) != 1 or policy_matching[0] != protection:
                raise RuntimeError("read-only policy reader did not read the exact protection")
            try:
                policy_api(
                    "DELETE",
                    f"/repos/{OWNER}/{REPOSITORY}/branch_protections/policy-reader-probe",
                )
            except ApiFailure as error:
                policy_reader_scope_refusal_status = error.status
            else:
                raise RuntimeError("read-only policy token unexpectedly completed a write request")
            if policy_reader_scope_refusal_status != 403:
                raise RuntimeError("policy reader write request was not refused by read-only scope")
            write_json(
                evidence_dir / "policy-reader-role-readback.json",
                {
                    "login": policy_reader_account.get("login"),
                    "is_admin": policy_reader_account.get("is_admin"),
                    "repo_permission_admin": policy_reader_repo_admin,
                    "token_scopes": ["read:user", "read:repository"],
                    "branch_protection_read": True,
                    "write_refusal_http_status": policy_reader_scope_refusal_status,
                },
            )

        no_review_binding = evidence_dir / "no-review-base-binding.json"
        write_json(
            no_review_binding,
            {
                "schema_version": 1,
                "review_id": 1,
                "reviewer_id": reviewer_id,
                "reviewer_login": REVIEWER,
                "pull_request_number": pr_number,
                "base_branch": "main",
                "base_commit": base_sha,
                "candidate_commit": head_sha,
                "source": "synthetic host fixture; no review is expected yet",
            },
        )
        no_review_gate_status, no_review_gate = run_local_test_merge_authority(
            client,
            number=pr_number,
            forgejo_pid=forgejo.pid,
            target_profile="merge-014-agent-forge-core",
            proof=target_proof,
            expected=expected,
            key=key_path,
            policy=evidence_dir / "policy.json",
            review_base_binding=no_review_binding,
            execution_identity="local-test-executor",
            host_token=merge_token,
            policy_token=policy_token,
        )
        write_json(evidence_dir / "no-review-rust-gate.json", no_review_gate)
        if (
            no_review_gate_status == 0
            or no_review_gate.get("verdict") != "rejected"
            or no_review_gate.get("merge_attempted") is not False
            or no_review_gate.get("error", {}).get("code") != "review_approval_missing"
        ):
            raise RuntimeError("Rust merge authority did not refuse the PR without an approved review")

        # Red gate: Forgejo must refuse an owner merge while the protected
        # reviewer approval is still absent.
        no_review_status: int | None = None
        no_review_reason: str | None = None
        try:
            merge_api(
                "POST",
                pr_path + "/merge",
                {"Do": "merge", "head_commit_id": head_sha},
            )
        except ApiFailure as error:
            no_review_reason = api_error_message(error)
            if error.status != 405 or "approval" not in no_review_reason.lower():
                raise
            no_review_status = error.status
        if no_review_status != 405:
            raise RuntimeError("Forgejo did not reject merge without the required review")
        _, still_open, _ = api("GET", pr_path)
        write_json(
            evidence_dir / "no-review-merge-refusal.json",
            {
                "http_status": no_review_status,
                "forgejo_reason": no_review_reason,
                "pull_request_state_after_refusal": still_open.get("state"),
                "merged_after_refusal": still_open.get("merged"),
            },
        )
        if still_open.get("merged") is True or still_open.get("state") != "open":
            raise RuntimeError("the refused merge changed the PR state")

        _, review_basis, _ = api("GET", pr_path)
        if (
            review_basis["base"]["sha"] != verified_base_sha
            or review_basis["head"]["sha"] != verified_head_sha
        ):
            raise RuntimeError("PR base/head changed before separate review; re-verification is required")
        base_sha = verified_base_sha
        _, review, _ = reviewer_api(
            "POST",
            pr_path + "/reviews",
            {
                "body": f"Reviewed this candidate and its acceptance result against main at {base_sha}.",
                "event": "APPROVED",
                "commit_id": head_sha,
            },
        )
        write_json(evidence_dir / "review-submission.json", review)
        if (
            review.get("state") != "APPROVED"
            or review.get("commit_id") != head_sha
            or review.get("pull_request_url")
            != f"{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/{pr_number}"
        ):
            raise RuntimeError("Forgejo review response does not bind the approved PR/head")
        non_whitelisted_merge_status: int | None = None
        non_whitelisted_merge_reason: str | None = None
        try:
            reviewer_api(
                "POST",
                pr_path + "/merge",
                {"Do": "merge", "head_commit_id": verified_head_sha},
            )
        except ApiFailure as error:
            non_whitelisted_merge_reason = api_error_message(error)
            if (
                error.status != 405
                or "not allowed to merge" not in non_whitelisted_merge_reason.lower()
            ):
                raise
            non_whitelisted_merge_status = error.status
        if non_whitelisted_merge_status != 405:
            raise RuntimeError("Forgejo did not reject merge by an approved but non-whitelisted reviewer")
        _, after_non_whitelisted_attempt, _ = api("GET", pr_path)
        write_json(
            evidence_dir / "non-whitelisted-merge-refusal.json",
            {
                "http_status": non_whitelisted_merge_status,
                "forgejo_reason": non_whitelisted_merge_reason,
                "pull_request_state_after_refusal": after_non_whitelisted_attempt.get("state"),
                "merged_after_refusal": after_non_whitelisted_attempt.get("merged"),
            },
        )
        if (
            after_non_whitelisted_attempt.get("state") != "open"
            or after_non_whitelisted_attempt.get("merged") is not False
        ):
            raise RuntimeError("non-whitelisted merge refusal changed the PR state")
        non_whitelisted_admin_merge_status: int | None = None
        non_whitelisted_admin_merge_reason: str | None = None
        if args.non_admin_merger:
            try:
                api(
                    "POST",
                    pr_path + "/merge",
                    {"Do": "merge", "head_commit_id": verified_head_sha},
                )
            except ApiFailure as error:
                non_whitelisted_admin_merge_reason = api_error_message(error)
                if (
                    error.status != 405
                    or "not allowed to merge"
                    not in non_whitelisted_admin_merge_reason.lower()
                ):
                    raise
                non_whitelisted_admin_merge_status = error.status
            if non_whitelisted_admin_merge_status != 405:
                raise RuntimeError("Forgejo allowed the non-whitelisted site admin to merge")
            _, after_admin_attempt, _ = api("GET", pr_path)
            write_json(
                evidence_dir / "non-whitelisted-admin-merge-refusal.json",
                {
                    "http_status": non_whitelisted_admin_merge_status,
                    "forgejo_reason": non_whitelisted_admin_merge_reason,
                    "pull_request_state_after_refusal": after_admin_attempt.get("state"),
                    "merged_after_refusal": after_admin_attempt.get("merged"),
                    "merge_whitelist_usernames": [merge_login],
                    "merge_actor_login": OWNER,
                    "merge_actor_is_admin": True,
                },
            )
            if (
                after_admin_attempt.get("state") != "open"
                or after_admin_attempt.get("merged") is not False
            ):
                raise RuntimeError("non-whitelisted admin merge refusal changed the PR state")
        review_id = review["id"]
        reviewer_id = review["user"]["id"]
        write_json(
            evidence_dir / "review-base-binding.json",
            {
                "schema_version": 1,
                "review_id": review_id,
                "reviewer_id": reviewer_id,
                "reviewer_login": REVIEWER,
                "pull_request_number": pr_number,
                "base_branch": "main",
                "base_commit": base_sha,
                "candidate_commit": head_sha,
                "source": "local test driver read the current PR immediately before submitting the synthetic separate reviewer approval",
            },
        )

        non_whitelisted_gate_status, non_whitelisted_gate = run_local_test_merge_authority(
            client,
            number=pr_number,
            forgejo_pid=forgejo.pid,
            target_profile="merge-014-agent-forge-core",
            proof=target_proof,
            expected=expected,
            key=key_path,
            policy=evidence_dir / "policy.json",
            review_base_binding=evidence_dir / "review-base-binding.json",
            execution_identity="local-test-executor",
            host_token=reviewer_token,
            policy_token=policy_token,
        )
        write_json(evidence_dir / "non-whitelisted-rust-gate.json", non_whitelisted_gate)
        if (
            non_whitelisted_gate_status == 0
            or non_whitelisted_gate.get("verdict") not in {"rejected", "inconclusive"}
            or non_whitelisted_gate.get("merge_attempted") is not False
            or non_whitelisted_gate.get("error", {}).get("code")
            not in {"upstream_forbidden", "branch_protection_does_not_match_local_gate_policy"}
        ):
            raise RuntimeError("Rust merge authority did not fail closed for the approved non-whitelisted caller")

        # Final reads: a complete review page, exact branch-protection scope,
        # and the current PR binding after verification and review.
        _, reviews, review_headers = api(
            "GET", pr_path + "/reviews?limit=50&page=1"
        )
        total_count = int(review_headers.get("x-total-count", "-1"))
        write_json(
            evidence_dir / "reviews-final.json",
            {"x_total_count": total_count, "reviews": reviews},
        )
        if total_count != len(reviews) or total_count != 1:
            raise RuntimeError("review pagination was incomplete or changed")
        approval = reviews[0]
        if (
            approval.get("state") != "APPROVED"
            or approval.get("commit_id") != head_sha
            or approval.get("stale") is not False
            or approval.get("dismissed") is not False
            or approval.get("official") is not True
            or approval.get("user", {}).get("id") != reviewer_id
            or approval.get("pull_request_url")
            != f"{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/{pr_number}"
        ):
            raise RuntimeError("current Forgejo review does not satisfy the configured reviewer policy")
        _, final_pr, _ = api("GET", pr_path)
        changed_files_before_merge = read_changed_files(api, pr_path)
        changed_paths_before_merge = sorted(
            item["filename"] for item in changed_files_before_merge
        )
        write_json(
            evidence_dir / "changed-files-before-merge.json",
            {
                "total_count": len(changed_paths_before_merge),
                "filenames": changed_paths_before_merge,
            },
        )
        _, final_protections, _ = api(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branch_protections"
        )
        write_json(
            evidence_dir / "premerge-forgejo-readback.json",
            {
                "pull_request": final_pr,
                "branch_protections": final_protections,
                "review_count": total_count,
            },
        )
        if (
            final_pr.get("state") != "open"
            or final_pr.get("merged") is not False
            or final_pr.get("mergeable") is not True
            or final_pr.get("base", {}).get("ref") != "main"
            or final_pr.get("base", {}).get("sha") != base_sha
            or final_pr.get("head", {}).get("sha") != head_sha
            or changed_paths_before_merge != changed_paths
            or final_protections != protections
        ):
            raise RuntimeError("final PR or protection read-back changed after validation")
        code, final_preflight = run_preflight(
            client, number=pr_number, proof=proof, expected=expected, key=key_path
        )
        write_json(evidence_dir / "final-premerge-preflight.json", final_preflight)
        if code or final_preflight.get("verdict") != "proof_current":
            raise RuntimeError("proof became stale before the final merge request")

        # The trusted host-only Rust command owns the POST. Forgejo validates
        # head_commit_id, but its API supplies no expected-base CAS; this
        # disposable instance is single-writer only for this test run.
        merge_request = {"Do": "merge", "head_commit_id": head_sha}
        write_json(evidence_dir / "merge-request.json", merge_request)
        def invoke_merge_authority() -> tuple[int, dict[str, Any]]:
            return run_local_test_merge_authority(
                client,
                number=pr_number,
                forgejo_pid=forgejo.pid,
                target_profile="merge-014-agent-forge-core",
                proof=target_proof,
                expected=expected,
                key=key_path,
                policy=evidence_dir / "policy.json",
                review_base_binding=evidence_dir / "review-base-binding.json",
                execution_identity="local-test-executor",
                host_token=merge_token,
                policy_token=policy_token,
            )

        stdio_mcp_task_result: dict[str, Any] | None = None
        if getattr(args, "stdio_mcp_egress_during_merge", False):
            if stdio_mcp_thread is None or not stdio_mcp_thread.is_alive():
                raise RuntimeError("the same stdio-MCP process that wrote the candidate is no longer held at its gate")
            stdio_mcp_context.update(
                {
                    "head_sha": head_sha,
                    "base_sha": base_sha,
                    "host_route_check": lambda: merge_api("GET", pr_path),
                    "gate_call": invoke_merge_authority,
                }
            )
            stdio_mcp_gate_release.set()
            stdio_mcp_thread.join(timeout=150)
            if stdio_mcp_thread.is_alive():
                stdio_mcp_context["cancelled"] = True
                raise TimeoutError("the held stdio-MCP task did not finish after the trusted merge gate")
            if "error" in stdio_mcp_context:
                raise RuntimeError("the held stdio-MCP task or merge gate failed") from stdio_mcp_context["error"]
            try:
                merge_gate_status, merge_gate_result, stdio_mcp_task_result = stdio_mcp_context[
                    "result"
                ]
            except (KeyError, TypeError, ValueError):
                raise RuntimeError("the held stdio-MCP task produced no gate result") from None
        elif getattr(args, "sandbox_egress_during_merge", False):
            merge_gate_status, merge_gate_result = run_gate_with_active_sandbox_probe(
                state=state,
                owner=OWNER,
                repository=REPOSITORY,
                pull_number=pr_number,
                host_route_check=lambda: api("GET", pr_path),
                gate_call=invoke_merge_authority,
                evidence_path=evidence_dir / "agent-egress-during-merge.json",
            )
        else:
            merge_gate_status, merge_gate_result = invoke_merge_authority()
        write_json(evidence_dir / "rust-merge-authority-result.json", merge_gate_result)
        if (
            merge_gate_status != 0
            or merge_gate_result.get("verdict") != "merged"
            or merge_gate_result.get("merge_authorized") is not True
            or merge_gate_result.get("merge_attempted") is not True
            or merge_gate_result.get("submitted_head_sha") != head_sha
            or merge_gate_result.get("validated_base_sha") != base_sha
            or merge_gate_result.get("main_head_sha")
            != merge_gate_result.get("merge_commit_sha")
            or merge_gate_result.get("postmerge_base_sha")
            != merge_gate_result.get("main_head_sha")
            or merge_gate_result.get("issue_state") != "closed"
        ):
            raise RuntimeError("Rust local merge authority did not confirm the protected merge")
        merge_status = merge_gate_result.get("http_status")
        _, merged_pr, _ = api("GET", pr_path)
        _, merged_main, _ = api(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main"
        )
        _, merged_issue, _ = api(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue['number']}"
        )
        _, merged_source, _ = api(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref=main",
        )
        merged_source_bytes = base64.b64decode(merged_source["content"])
        postmerge_status, postmerge_output, postmerge_worker_sha = run_candidate_sandbox(
            merged_source_bytes,
            state / "verify-postmerge",
            bwrap=bwrap,
            rust_sysroot=rust_sysroot,
        )
        write_private(evidence_dir / "postmerge-verifier.log", postmerge_output)
        if postmerge_status != 0 or b"9 passed; 0 failed" not in postmerge_output:
            raise RuntimeError("the source read back from main failed post-merge black-box acceptance")
        postmerge_behavior = {
            "tested_source_sha256": hashlib.sha256(merged_source_bytes).hexdigest(),
            "worker_sha256": postmerge_worker_sha,
            "acceptance_exit_code": postmerge_status,
            "passed_tests": 9,
            "failed_tests": 0,
        }
        write_json(
            evidence_dir / "postmerge-forgejo-readback.json",
            {
                "pull_request": merged_pr,
                "main_head": merged_main.get("commit", {}).get("id"),
                "issue": merged_issue,
                "source_sha256": hashlib.sha256(merged_source_bytes).hexdigest(),
                "tested_source_sha256": hashlib.sha256(source).hexdigest(),
                "postmerge_behavior": postmerge_behavior,
            },
        )
        if (
            merged_pr.get("state") != "closed"
            or merged_pr.get("merged") is not True
            or merged_pr.get("head", {}).get("sha") != head_sha
            or merged_pr.get("base", {}).get("ref") != "main"
            or merged_pr.get("base", {}).get("sha") != merged_main.get("commit", {}).get("id")
            or merged_main.get("commit", {}).get("id")
            != merged_pr.get("merge_commit_sha")
            or merged_issue.get("state") != "closed"
            or merged_source_bytes != source
        ):
            raise RuntimeError("post-merge API read-back did not confirm PR, issue, and tested source")

        stdio_mcp_source_binding: dict[str, Any] | None = None
        if stdio_mcp_task_result is not None:
            barrier = stdio_mcp_task_result.get("mcp_barrier", {})
            process_observation = barrier.get("sandbox_agent_process_observation")
            before_observation = (
                process_observation.get("before_gate")
                if isinstance(process_observation, dict)
                else None
            )
            after_observation = (
                process_observation.get("after_gate")
                if isinstance(process_observation, dict)
                else None
            )
            observed_identity_fields = (
                "agent_host_pid",
                "agent_start_time_ticks",
                "agent_network_namespace",
                "agent_pid_namespace",
            )
            source_sha256 = hashlib.sha256(merged_source_bytes).hexdigest()
            if (
                stdio_mcp_context.get("source_sha256") != source_sha256
                or stdio_mcp_task_result.get("source_sha256") != source_sha256
                or barrier.get("child_process_alive_during_gate_read") is not True
                or not isinstance(process_observation, dict)
                or process_observation.get("observed_by")
                != "trusted parent via /proc descendant tree"
                or not isinstance(before_observation, dict)
                or not isinstance(after_observation, dict)
                or before_observation.get("agent_alive") is not True
                or after_observation.get("agent_alive") is not True
                or before_observation.get("observed_by_parent_procfs") is not True
                or after_observation.get("observed_by_parent_procfs") is not True
                or before_observation.get("agent_entrypoint")
                != "mission_012_test_agent_adapter.py"
                or after_observation.get("agent_entrypoint")
                != "mission_012_test_agent_adapter.py"
                or any(
                    before_observation.get(field) != after_observation.get(field)
                    for field in observed_identity_fields
                )
                or after_observation.get("agent_network_namespace")
                == after_observation.get("host_network_namespace")
                or after_observation.get("agent_pid_namespace")
                == after_observation.get("host_pid_namespace")
                or barrier.get("released_after_gate_operation") is not True
                or stdio_mcp_task_result.get("source_changed_through_mcp") is not True
            ):
                raise RuntimeError(
                    "the PR or merged source is not byte-identical to the source written by the held MCP process"
                )
            stdio_mcp_source_binding = {
                "producer": "same stdio-MCP process using the explicitly named local-test-adapter",
                "llm_invoked": False,
                "mcp_source_sha256": stdio_mcp_context["source_sha256"],
                "forgejo_candidate_source_sha256": hashlib.sha256(source).hexdigest(),
                "forgejo_candidate_commit": head_sha,
                "pull_request_head_after_restore": merged_pr.get("head", {}).get("sha"),
                "merge_commit_sha": merge_gate_result.get("merge_commit_sha"),
                "merged_source_sha256": source_sha256,
                "same_process_alive_during_gate": barrier[
                    "child_process_alive_during_gate_read"
                ],
                "host_observed_sandbox_agent": process_observation,
                "same_process_released_after_merge_operation": barrier[
                    "released_after_gate_operation"
                ],
            }
            write_json(
                evidence_dir / "stdio-mcp-source-binding.json",
                stdio_mcp_source_binding,
            )

        webhook_summary: dict[str, Any] | None = None
        if observe_pr_webhook:
            if (
                webhook_e2e is None
                or webhook_hook_id is None
                or webhook_reconcile_token is None
                or webhook_attestation_public_key is None
                or webhook_target is None
                or webhook_receiver is None
                or webhook_receiver.poll() is not None
            ):
                raise RuntimeError("the local PR webhook receiver or trusted reconciliation inputs are unavailable")
            webhook_closed_event = wait_for_pull_request_webhook(
                webhook_journal,
                pull_request_number=pr_number,
                action="closed",
                expected_merged=True,
                expected_head_sha=head_sha,
                expected_merge_commit_sha=merge_gate_result.get("merge_commit_sha"),
            )
            webhook_task = wait_for_webhook_task_receipt(
                state,
                webhook_hook_id,
                webhook_closed_event["delivery_id"],
            )
            if replay_pr_webhook:
                if webhook_secret is None:
                    raise RuntimeError("the local webhook secret is unavailable for the duplicate replay")
                if restart_pr_webhook_before_replay:
                    if (
                        webhook_receiver is None
                        or webhook_receiver_log is None
                        or receiver_binary is None
                        or webhook_secret_path is None
                        or webhook_attestation_seed_path is None
                        or webhook_receiver_port is None
                    ):
                        raise RuntimeError("the local receiver cannot be restarted with its original bindings")
                    webhook_receiver, webhook_receiver_log, webhook_receiver_restart = (
                        restart_webhook_receiver_for_replay(
                            webhook_receiver,
                            webhook_receiver_log,
                            binary=receiver_binary,
                            state=state,
                            address=f"127.0.0.1:{webhook_receiver_port}",
                            secret_path=webhook_secret_path,
                            attestation_seed_path=webhook_attestation_seed_path,
                            journal=webhook_journal,
                            delivery_id=webhook_closed_event["delivery_id"],
                        )
                    )
                    write_json(
                        evidence_dir / "webhook-receiver-restart-summary.json",
                        webhook_receiver_restart,
                    )
                webhook_duplicate_replay = replay_accepted_pr_webhook_duplicate(
                    webhook_journal,
                    webhook_target,
                    webhook_closed_event["delivery_id"],
                    webhook_secret,
                    pull_request_number=pr_number,
                    expected_head_sha=head_sha,
                    expected_merge_commit_sha=merge_gate_result["merge_commit_sha"],
                )
                webhook_duplicate_replay["after_receiver_restart"] = (
                    webhook_receiver_restart is not None
                )
                write_json(
                    evidence_dir / "webhook-duplicate-replay-summary.json",
                    webhook_duplicate_replay,
                )
            webhook_status, webhook_reconciliation = run_single_webhook_reconciliation(
                client,
                state,
                webhook_journal,
                webhook_closed_event["delivery_id"],
                webhook_reconcile_token,
                webhook_attestation_public_key,
            )
            canonical = webhook_reconciliation.get("canonical_state")
            resource = webhook_reconciliation.get("resource")
            reconciliation_error = webhook_reconciliation.get("error")
            reconciliation_error_details = (
                reconciliation_error.get("details")
                if isinstance(reconciliation_error, dict)
                else None
            )
            expected_merge_sha = merge_gate_result.get("merge_commit_sha")
            resource_matches = isinstance(resource, dict)
            canonical_matches = isinstance(canonical, dict)
            reconciliation_checks = {
                "process_exit_zero": webhook_status == 0,
                "verdict_current_state_read": webhook_reconciliation.get("verdict")
                == "current_state_read",
                "delivery_id_matches": webhook_reconciliation.get("delivery_id")
                == webhook_closed_event["delivery_id"],
                "one_upstream_read": webhook_reconciliation.get("upstream_read_attempts") == 1,
                "resource_is_pull_request": resource_matches
                and resource.get("kind") == "pull_request",
                "resource_number_matches": resource_matches and resource.get("number") == pr_number,
                "resource_owner_matches": resource_matches and resource.get("owner") == OWNER,
                "resource_repository_matches": resource_matches
                and resource.get("repository") == REPOSITORY,
                "canonical_state_closed": canonical_matches and canonical.get("state") == "closed",
                "canonical_pr_merged": canonical_matches and canonical.get("merged") is True,
                "canonical_head_matches": canonical_matches and canonical.get("head_sha") == head_sha,
                "canonical_base_matches_merge": canonical_matches
                and canonical.get("base_sha") == expected_merge_sha,
                "canonical_merge_commit_matches": canonical_matches
                and canonical.get("merge_commit_sha") == expected_merge_sha,
            }
            diagnostic = {
                "schema_version": 1,
                "checks": reconciliation_checks,
                "expected": {
                    "delivery_id": webhook_closed_event["delivery_id"],
                    "pull_request_number": pr_number,
                    "owner": OWNER,
                    "repository": REPOSITORY,
                    "head_sha": head_sha,
                    "merge_commit_sha": expected_merge_sha,
                },
                "observed": {
                    "process_exit_code": webhook_status,
                    "verdict": webhook_reconciliation.get("verdict"),
                    "delivery_id": webhook_reconciliation.get("delivery_id"),
                    "upstream_read_attempts": webhook_reconciliation.get("upstream_read_attempts"),
                    "error_code": reconciliation_error.get("code")
                    if isinstance(reconciliation_error, dict)
                    else None,
                    "error_field_path": reconciliation_error_details.get("field_path")
                    if isinstance(reconciliation_error_details, dict)
                    else None,
                    "resource": {
                        key: resource.get(key)
                        for key in ("kind", "number", "owner", "repository")
                    }
                    if resource_matches
                    else None,
                    "canonical_state": {
                        key: canonical.get(key)
                        for key in ("state", "merged", "head_sha", "base_sha", "merge_commit_sha")
                    }
                    if canonical_matches
                    else None,
                },
            }
            write_json(evidence_dir / "webhook-reconciliation-diagnostic.json", diagnostic)
            failed_reconciliation_checks = [
                name for name, passed in reconciliation_checks.items() if not passed
            ]
            if failed_reconciliation_checks:
                raise RuntimeError(
                    "webhook reconciliation checks failed: " + ",".join(failed_reconciliation_checks)
                )
            write_json(evidence_dir / "webhook-merged-delivery-summary.json", webhook_closed_event)
            write_json(evidence_dir / "webhook-merged-task-receipt.json", webhook_task)
            write_json(evidence_dir / "webhook-merged-reconciliation.json", webhook_reconciliation)

            api(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/hooks/{webhook_hook_id}",
                webhook_e2e.hook_deactivation_patch(("pull_request",)),
            )
            _, disabled_readback, _ = api(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{webhook_hook_id}"
            )
            webhook_hook_readback = webhook_e2e.hook_readback_record(
                webhook_hook_id,
                disabled_readback,
                webhook_target,
                expected_event_selectors=("pull_request",),
            )
            webhook_hook_disabled = webhook_hook_readback.get("deactivation_readback_confirmed") is True
            if (
                not webhook_hook_disabled
                or webhook_hook_readback.get("configuration_matches_expected") is not True
            ):
                raise RuntimeError("Forgejo webhook deactivation or configuration read-back failed")
            write_json(evidence_dir / "webhook-hook-disabled-readback.json", webhook_hook_readback)
            webhook_summary = {
                "enabled": True,
                "delivery_id": webhook_closed_event["delivery_id"],
                "delivery_sequence": webhook_closed_event["sequence"],
                "action": webhook_closed_event["action"],
                "head_sha": webhook_closed_event["head_sha"],
                "merge_commit_sha": webhook_closed_event["merge_commit_sha"],
                "forgejo_task_http_202": webhook_task["response_status"] == 202,
                "reconciled_verdict": webhook_reconciliation.get("verdict"),
                "reconciled_merge_commit_sha": canonical.get("merge_commit_sha"),
                "hook_deactivated": webhook_hook_disabled,
                "secret_recorded": False,
                "duplicate_replay": webhook_duplicate_replay,
                "receiver_restart": webhook_receiver_restart,
            }

        semantic_summary = None
        if semantic_clean_merge_canary:
            semantic_summary = run_semantic_clean_merge_canary(
                state=state,
                api=api,
                reviewer_api=reviewer_api,
                client=client,
                gate=gate,
                forgejo_pid=forgejo.pid,
                key_path=key_path,
                policy_bytes=policy_bytes,
                merge_token=merge_token,
                policy_token=policy_token,
                reviewer_id=reviewer_id,
                bwrap=bwrap,
                rust_sysroot=rust_sysroot,
            )

        result = {
            "schema_version": 1,
            "forgejo_version": "17.0.0+gitea-1.22.0",
            "origin": ORIGIN,
            "issue_number": issue["number"],
            "pull_request_number": pr_number,
            "requirement_sha256": requirement_sha,
            "policy_sha256": policy_sha,
            "base_commit": base_sha,
            "candidate_commit": head_sha,
            "candidate_sha256": hashlib.sha256(source).hexdigest(),
            "changed_paths": changed_paths_before_merge,
            "worker_sha256": final_worker_sha,
            "forgejo_binary_sha256": process_e2e.sha256_file(binary),
            "forgejo_listener_pid": forgejo.pid,
            "red_acceptance": {
                "exit_code": red_status,
                "failed_tests": [
                    "below_min_returns_min",
                    "above_max_returns_max",
                    "singleton_interval_clamps",
                    "i32_min_clamps",
                    "i32_max_clamps",
                ],
                "passing_tests": [
                    "in_range_is_preserved",
                    "equal_lower_bound_is_preserved",
                    "equal_upper_bound_is_preserved",
                    "negative_interval_clamps",
                ],
            },
            "green_acceptance": "9 passed; 0 failed",
            "stale_candidate_verdict": stale.get("error", {}).get("code"),
            "fresh_premerge_verdict": final_preflight.get("verdict"),
            "branch_protection": {
                key: protection.get(key)
                for key in (
                    "rule_name",
                    "branch_name",
                    "required_approvals",
                    "enable_approvals_whitelist",
                    "approvals_whitelist_username",
                    "apply_to_admins",
                    "dismiss_stale_approvals",
                    "block_on_outdated_branch",
                    "unprotected_file_patterns",
                    "enable_merge_whitelist",
                    "merge_whitelist_usernames",
                )
            },
            "no_review_merge_refusal_http_status": no_review_status,
            "no_review_merge_refusal_reason": no_review_reason,
            "no_review_rust_gate_refused": no_review_gate.get("verdict") == "rejected",
            "no_review_rust_gate_merge_attempted": no_review_gate.get("merge_attempted"),
            "false_executor_status_refused": fake_status != 0,
            "non_whitelisted_reviewer_rust_gate_failed_closed": (
                non_whitelisted_gate_status != 0
                and non_whitelisted_gate.get("merge_attempted") is False
            ),
            "non_whitelisted_reviewer_rust_gate_verdict": non_whitelisted_gate.get(
                "verdict"
            ),
            "non_whitelisted_reviewer_rust_gate_merge_attempted": (
                non_whitelisted_gate.get("merge_attempted")
            ),
            "non_whitelisted_reviewer_rust_gate_error_code": non_whitelisted_gate.get(
                "error", {}
            ).get("code"),
            "non_whitelisted_merge_refusal_http_status": non_whitelisted_merge_status,
            "non_whitelisted_merge_refusal_reason": non_whitelisted_merge_reason,
            "review_id": review_id,
            "reviewer_id": reviewer_id,
            "reviewer_login": REVIEWER,
            "merge_actor_login": merge_account.get("login"),
            "merge_actor_is_admin": merge_account.get("is_admin"),
            "non_admin_merge_account_tested": args.non_admin_merger,
            "policy_reader_login": (
                policy_reader_account.get("login") if policy_reader_account else None
            ),
            "policy_reader_is_admin": (
                policy_reader_account.get("is_admin") if policy_reader_account else None
            ),
            "policy_reader_repo_admin": policy_reader_repo_admin,
            "policy_reader_readonly_scope_refusal_http_status": (
                policy_reader_scope_refusal_status
            ),
            "non_whitelisted_admin_merge_refusal_http_status": (
                non_whitelisted_admin_merge_status
            ),
            "non_whitelisted_admin_merge_refusal_reason": non_whitelisted_admin_merge_reason,
            "merge_http_status": merge_status,
            "merge_authority": "host-invoked Rust local-test command; no LLM",
            "merged": merged_pr["merged"],
            "merge_commit_sha": merged_pr.get("merge_commit_sha"),
            "main_head_after_merge": merged_main.get("commit", {}).get("id"),
            "issue_state_after_merge": merged_issue["state"],
            "postmerge_behavior": postmerge_behavior,
            "agent": "deterministic local test adapter; no LLM configured or invoked",
            "authority": "Rust CLI carries the merge request; local test profile only",
            "external_writers_excluded": False,
            "sandbox_egress_during_merge": getattr(
                args, "sandbox_egress_during_merge", False
            ),
            "stdio_mcp_egress_during_merge": getattr(
                args, "stdio_mcp_egress_during_merge", False
            ),
            "stdio_mcp_task_acceptance": (
                stdio_mcp_task_result.get("task_acceptance")
                if stdio_mcp_task_result is not None
                else None
            ),
            "stdio_mcp_gate_barrier": (
                stdio_mcp_task_result.get("mcp_barrier")
                if stdio_mcp_task_result is not None
                else None
            ),
            "stdio_mcp_source_binding": stdio_mcp_source_binding,
            "same_run_webhook_reconciliation": webhook_summary,
            "semantic_clean_merge_canary": semantic_summary,
        }
        write_json(evidence_dir / "result.json", result)
        return result
    finally:
        if stdio_mcp_thread is not None and stdio_mcp_thread.is_alive():
            stdio_mcp_context["cancelled"] = True
            stdio_mcp_gate_release.set()
            stdio_mcp_thread.join(timeout=120)
            if stdio_mcp_thread.is_alive():
                write_json(
                    evidence_dir / "stdio-mcp-cleanup-inconclusive.json",
                    {"status": "inconclusive", "reason": "held task did not exit after cancellation"},
                )
        if (
            webhook_hook_id is not None
            and not webhook_hook_disabled
            and api is not None
            and webhook_e2e is not None
        ):
            try:
                api(
                    "PATCH",
                    f"/repos/{OWNER}/{REPOSITORY}/hooks/{webhook_hook_id}",
                    webhook_e2e.hook_deactivation_patch(("pull_request",)),
                )
                _, cleanup_readback, _ = api(
                    "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{webhook_hook_id}"
                )
                cleanup_summary = webhook_e2e.hook_readback_record(
                    webhook_hook_id,
                    cleanup_readback,
                    webhook_target,
                    expected_event_selectors=("pull_request",),
                )
                webhook_hook_disabled = (
                    cleanup_summary.get("deactivation_readback_confirmed") is True
                )
                write_json(evidence_dir / "webhook-hook-cleanup-readback.json", cleanup_summary)
            except Exception:
                write_json(
                    evidence_dir / "webhook-hook-cleanup-inconclusive.json",
                    {"status": "inconclusive", "reason": "test hook deactivation read-back failed"},
                )
        if webhook_receiver is not None:
            receiver_stop = {"started": True, "stop_action": "already_exited"}
            if webhook_receiver.poll() is None:
                receiver_stop["stop_action"] = "terminate"
                try:
                    webhook_receiver.terminate()
                except ProcessLookupError:
                    receiver_stop["stop_action"] = "exited_during_stop"
                try:
                    webhook_receiver.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    receiver_stop["stop_action"] = "kill_after_timeout"
                    try:
                        webhook_receiver.kill()
                    except ProcessLookupError:
                        receiver_stop["stop_action"] = "exited_during_stop"
                    webhook_receiver.wait(timeout=5)
            receiver_stop["stopped"] = webhook_receiver.poll() is not None
            receiver_stop["returncode"] = webhook_receiver.returncode
            write_json(evidence_dir / "webhook-receiver-stop.json", receiver_stop)
        if webhook_receiver_log is not None:
            webhook_receiver_log.close()
        if forgejo.poll() is None:
            forgejo.terminate()
            try:
                forgejo.wait(timeout=10)
            except subprocess.TimeoutExpired:
                forgejo.kill()
                forgejo.wait(timeout=5)
        server_log.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(Path(__file__).resolve().parents[1]))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument(
        "--non-admin-merger",
        action="store_true",
        help="create a write collaborator as the sole merge actor and verify it is not an admin",
    )
    parser.add_argument(
        "--sandbox-egress-during-merge",
        action="store_true",
        help="hold the no-network test probe across the trusted merge gate and repeat live route probes afterward",
    )
    parser.add_argument(
        "--stdio-mcp-egress-during-merge",
        action="store_true",
        help="hold the real stdio-MCP task after green acceptance while the trusted gate merges and reads back the PR",
    )
    parser.add_argument(
        "--observe-pr-webhook",
        action="store_true",
        help="receive Forgejo's native merged pull_request event and reconcile it to the same merge commit",
    )
    parser.add_argument(
        "--replay-pr-webhook",
        action="store_true",
        help="re-sign the accepted native PR payload to test receiver deduplication before canonical reconciliation",
    )
    parser.add_argument(
        "--restart-pr-webhook-before-replay",
        action="store_true",
        help="gracefully restart the local receiver over its journal before replaying the accepted native PR event",
    )
    parser.add_argument(
        "--semantic-clean-merge-canary",
        action="store_true",
        help="after the base clamp walkthrough, run a second local Forgejo PR scenario where cleanly merged independent changes fail protected semantic acceptance before correction",
    )
    args = parser.parse_args()
    try:
        result = run(args)
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    print(f"Private Forgejo database/logs/evidence retained at {args.state_dir}")
    print("All raw tokens remained in process memory; this test driver is not a production merge authority.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
