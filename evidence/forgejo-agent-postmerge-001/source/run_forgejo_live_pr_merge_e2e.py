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
from typing import Any, Callable
from urllib.error import HTTPError, URLError
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

import run_forgejo_mission_delivery_process_e2e as process_e2e
import mission_012_sandboxed_test_adapter as sandbox_probe
import mission_012_test_agent_adapter as test_agent_adapter


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
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        raise RuntimeError(f"command exceeded {timeout} seconds: {command[0]}")
    finally:
        reader.join(timeout=5)
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


def create_app_ini(state: Path) -> None:
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
        "ENABLED = false",
        "",
        "[session]",
        "PROVIDER = memory",
        "",
        "[cache]",
        "ADAPTER = memory",
        "",
    ]
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


def build_local_binaries(
    repo: Path,
    state: Path,
    evidence_dir: Path,
    *,
    bwrap: str,
    rust_sysroot: Path,
) -> tuple[Path, Path]:
    """Compile only the two trusted local executables in a no-network sandbox."""
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

    helper = r'''import json, os, shutil, subprocess, sys
names = json.loads(sys.argv[1])
os.makedirs("/tmp/cargo-home/registry", exist_ok=True)
shutil.copytree("/bootstrap/registry/index", "/tmp/cargo-home/registry/index")
shutil.copytree("/bootstrap/registry/cache", "/tmp/cargo-home/registry/cache")
process = subprocess.Popen([
    "/toolchain/bin/cargo", "build", "--offline", "--locked",
    "--bin", names[0], "--bin", names[1]
], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
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
            json.dumps(["gate", "forgejo-reconcile-local"]),
        ]
    )
    artifacts = {
        "gate": artifacts_dir / "gate",
        "forgejo-reconcile-local": artifacts_dir / "forgejo-reconcile-local",
    }
    build_log = io.StringIO()
    try:
        with redirect_stdout(build_log):
            process_e2e.run_artifact_build(command, cwd=repo, artifacts=artifacts)
    except Exception as error:
        write_private(evidence_dir / "cargo-build.log", build_log.getvalue().encode())
        raise RuntimeError("offline isolated build of the proof gate and Forgejo client failed") from error
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


def run_local_test_merge_authority(
    client: Path,
    *,
    number: int,
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
    """Invoke the gate with separate actor and optional policy pipes."""
    tokens = [("host", host_token)]
    if policy_token is not None:
        tokens.append(("policy", policy_token))
    descriptors: list[tuple[str, int, int, bytes]] = []
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
        if policy_token is not None:
            command.extend(["--policy-credential-fd", str(descriptors[1][1])])
            pass_fds.append(descriptors[1][1])
        if any(token in command for _, token in tokens):
            raise RuntimeError("private credential was placed in CLI arguments")
        status, output = bounded_process(
            command,
            timeout=60,
            pass_fds=tuple(pass_fds),
        )
    finally:
        for _, read_fd, write_fd, _ in descriptors:
            os.close(read_fd)
            if write_fd >= 0:
                os.close(write_fd)
    if any(token_bytes in output for _, _, _, token_bytes in descriptors):
        raise RuntimeError("Rust merge authority exposed a private credential in output")
    try:
        result = json.loads(output)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"local merge authority returned invalid JSON: {error}") from None
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
        if (
            event.get("type") != "sandbox_egress_probe_ready"
            or event.get("sandbox_checks")
            != {
                "credential_visible": False,
                "network_connected": False,
                "direct_forgejo_routes_blocked": ["api", "web", "git_receive_pack"],
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
    if not binary.is_file():
        raise RuntimeError("--forgejo-binary must resolve to a regular local file")
    if not (repo / "Cargo.toml").is_file():
        raise RuntimeError("--repo-root is not the Rust project")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3130)) == 0:
            raise RuntimeError("127.0.0.1:3130 is already in use; refusing to start")

    state.mkdir(mode=0o700)
    create_app_ini(state)
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
        initial_source = (
            "pub fn clamp_inclusive(value: i32, min: i32, max: i32) -> i32 {\n"
            "    value\n"
            "}\n"
        ).encode()
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

        red_dir = state / "verify-red"
        red_status, red_output, _ = run_candidate_sandbox(
            actual_initial, red_dir, bwrap=bwrap, rust_sysroot=rust_sysroot
        )
        write_private(evidence_dir / "red-verifier.log", red_output)
        if (
            red_status == 0
            or b"below_min_returns_min ... FAILED" not in red_output
            or b"above_max_returns_max ... FAILED" not in red_output
            or b"singleton_interval_clamps ... FAILED" not in red_output
            or b"i32_min_clamps ... FAILED" not in red_output
            or b"i32_max_clamps ... FAILED" not in red_output
            or b"in_range_is_preserved ... ok" not in red_output
            or b"equal_lower_bound_is_preserved ... ok" not in red_output
            or b"equal_upper_bound_is_preserved ... ok" not in red_output
            or b"negative_interval_clamps ... ok" not in red_output
            or b"4 passed; 5 failed" not in red_output
        ):
            raise RuntimeError("the initial Forgejo candidate did not produce the expected actionable failure")

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

        # Explicitly identified deterministic test adapter: use the red feedback
        # to apply the one-line correction, then write it as a new Forgejo commit.
        corrected_source = actual_initial.replace(
            b"    value\n", b"    value.clamp(min, max)\n", 1
        )
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

        # Change the candidate before opening the PR, so Forgejo's internal
        # refs/pull/<n>/head is created from the final head. The stale proof
        # remains bound to the earlier commit and must fail at live preflight.
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
            f"/repos/{OWNER}/{REPOSITORY}/contents/src/lib.rs?ref={verified_head_sha}",
        )
        source = base64.b64decode(content_after_probe["content"])
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

        if getattr(args, "sandbox_egress_during_merge", False):
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
        }
        write_json(evidence_dir / "result.json", result)
        return result
    finally:
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
