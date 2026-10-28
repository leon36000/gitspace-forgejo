#!/usr/bin/env python3
"""Exercise the real local Forgejo webhook receiver and reconciliation CLI.

Forgejo itself is a disposable local v17 instance. The webhook body is
synthetic and intentionally stale; this harness does not claim Forgejo sent it.
Raw credentials stay in memory or private pipes under a fresh /tmp state dir.
"""

from __future__ import annotations

import argparse
from contextlib import redirect_stdout
import hashlib
import hmac
import io
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import time
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_pr_merge_e2e as forgejo_helpers  # noqa: E402


OWNER = forgejo_helpers.OWNER
REPOSITORY = forgejo_helpers.REPOSITORY
ORIGIN = forgejo_helpers.ORIGIN
READER = "forge-reader"
DELIVERY_ID = "live-stale-issue-41"
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class LocalForgejoApi:
    def __init__(self, token: str | None):
        self._token = token
        self._opener = build_opener(ProxyHandler({}), NoRedirect)

    def request(self, method: str, path: str, body: Any = None) -> tuple[Any, dict[str, str]]:
        if not path.startswith("/") or path.startswith("//"):
            raise RuntimeError("Forgejo API path escaped its fixed local origin")
        headers = {"Accept": "application/json"}
        if self._token is not None:
            headers["Authorization"] = f"token {self._token}"
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(body).encode()
        request = Request(
            ORIGIN + "/api/v1" + path,
            data=data,
            headers=headers,
            method=method,
        )
        try:
            with self._opener.open(request, timeout=5) as response:
                raw = response.read(1024 * 1024 + 1)
                if len(raw) > 1024 * 1024:
                    raise RuntimeError("Forgejo API response exceeded the local E2E limit")
                value = json.loads(raw) if raw else None
                return value, {key.lower(): val for key, val in response.headers.items()}
        except HTTPError as error:
            error.close()
            raise RuntimeError(f"Forgejo API {method} {path} returned HTTP {error.code}") from None
        except URLError as error:
            raise RuntimeError(f"Forgejo API {method} {path} transport failed") from None


def private_command_environment(state: Path) -> dict[str, str]:
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": str(state / "home"),
        "TMPDIR": str(state / "tmp"),
    }


def admin_command(binary: Path, state: Path, args: list[str]) -> bytes:
    command = [
        str(binary),
        "--work-path",
        str(state),
        "--config",
        str(state / "custom/conf/app.ini"),
        *args,
    ]
    completed = subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=private_command_environment(state),
        timeout=30,
    )
    if completed.returncode:
        # Forgejo may print token material in CLI errors; keep it out of logs.
        raise RuntimeError(f"local Forgejo admin command failed ({completed.returncode})")
    return completed.stdout


def create_access_token(binary: Path, state: Path, username: str, name: str, scopes: str) -> str:
    raw = admin_command(
        binary,
        state,
        [
            "admin",
            "user",
            "generate-access-token",
            "--username",
            username,
            "--token-name",
            name,
            "--raw",
            "--scopes",
            scopes,
        ],
    ).decode().strip()
    if not raw or any(character.isspace() for character in raw):
        raise RuntimeError(f"Forgejo generated an invalid token for {username}")
    return raw


def select_receiver_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


def wait_for_listener(process: subprocess.Popen, port: int, timeout: float = 10.0) -> None:
    ss = shutil.which("ss")
    if ss is None:
        raise RuntimeError("ss is required to verify the local webhook receiver listener")
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"local webhook receiver exited with code {process.returncode}")
        listing = subprocess.run(
            [ss, "-H", "-ltnp"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=2,
            check=False,
        ).stdout
        for line in listing.splitlines():
            columns = line.split()
            if len(columns) >= 6 and columns[3] == f"127.0.0.1:{port}":
                if f"pid={process.pid}," in line:
                    return
                raise RuntimeError("receiver port is owned by a process outside this E2E run")
        time.sleep(0.1)
    raise RuntimeError("local webhook receiver did not bind its expected loopback port")


def send_signed_webhook(address: str, secret: bytes, delivery: str, payload: bytes) -> dict[str, Any]:
    signature = hmac.new(secret, payload, hashlib.sha256).hexdigest()
    request = Request(
        f"http://{address}/forgejo/webhook",
        data=payload,
        headers={
            "Content-Type": "application/json",
            "X-Forgejo-Delivery": delivery,
            "X-Forgejo-Event": "issues",
            "X-Forgejo-Signature": signature,
        },
        method="POST",
    )
    opener = build_opener(ProxyHandler({}), NoRedirect)
    try:
        with opener.open(request, timeout=5) as response:
            data = response.read(64 * 1024 + 1)
            if len(data) > 64 * 1024:
                raise RuntimeError("webhook receiver response exceeded the E2E limit")
            return {"http_status": response.status, "body": json.loads(data)}
    except HTTPError as error:
        error.close()
        raise RuntimeError(f"local webhook receiver returned HTTP {error.code}") from None
    except URLError as error:
        raise RuntimeError("local webhook receiver transport failed") from None


def run_reconcile(
    binary: Path,
    state: Path,
    journal: Path,
    delivery: str,
    credential: str,
    target_profile: str | None = None,
) -> tuple[int, dict[str, Any]]:
    command = [str(binary)]
    if target_profile is not None:
        if target_profile not in {"merge-014-agent-forge-core", "mission-012-clamp"}:
            raise RuntimeError("reconciliation target profile is not in the fixed local allowlist")
        command.extend(["--target-profile", target_profile])
    command.extend(["--host-credential-fd"])
    read_fd, write_fd = os.pipe()
    try:
        os.write(write_fd, credential.encode())
    finally:
        os.close(write_fd)
    try:
        completed = subprocess.run(
            [*command, str(read_fd), str(journal), delivery],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=private_command_environment(state),
            pass_fds=(read_fd,),
            timeout=20,
            check=False,
        )
    finally:
        os.close(read_fd)
    if credential.encode() in completed.stdout or credential.encode() in completed.stderr:
        raise RuntimeError("reconciliation CLI exposed its private credential")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError:
        raise RuntimeError("reconciliation CLI did not return structured JSON") from None
    if not isinstance(value, dict):
        raise RuntimeError("reconciliation CLI returned a non-object result")
    return completed.returncode, value


def stop_process(process: subprocess.Popen | None) -> dict[str, Any]:
    if process is None:
        return {"started": False, "stopped": True, "returncode": None}
    action = "already_exited"
    if process.poll() is None:
        action = "terminate"
        try:
            process.terminate()
        except ProcessLookupError:
            action = "exited_during_stop"
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            action = "kill_after_timeout"
            try:
                process.kill()
            except ProcessLookupError:
                action = "exited_during_stop"
            process.wait(timeout=5)
    return {
        "started": True,
        "stopped": process.poll() is not None,
        "returncode": process.returncode,
        "stop_action": action,
    }


def listener_addresses_for_port(port: int) -> list[str]:
    ss = shutil.which("ss")
    if ss is None:
        raise RuntimeError("ss is required to verify process cleanup")
    listing = subprocess.run(
        [ss, "-H", "-ltnp"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        timeout=5,
        check=True,
    ).stdout
    addresses = []
    for line in listing.splitlines():
        columns = line.split()
        if len(columns) >= 4 and columns[3].rsplit(":", 1)[-1] == str(port):
            addresses.append(columns[3])
    return sorted(set(addresses))


def build_local_binaries(
    repo: Path,
    state: Path,
    evidence_dir: Path,
) -> tuple[Path, Path, dict[str, Any]]:
    """Build trusted local CLIs with host paths hidden and networking disabled."""
    process_e2e = forgejo_helpers.process_e2e
    if repo != process_e2e.REPO.resolve(strict=True):
        raise RuntimeError("isolated build helper must target the runner's own repository")
    bwrap = Path(shutil.which("bwrap") or "/usr/bin/bwrap").resolve(strict=True)
    prlimit = Path("/usr/bin/prlimit")
    if not prlimit.is_file():
        raise RuntimeError("prlimit is required for bounded local compilation")
    rustc_program = shutil.which("rustc")
    if rustc_program is None:
        raise RuntimeError("rustc is required for the isolated local source build")
    rust_env = {
        "PATH": f"{Path(rustc_program).resolve().parent}:/usr/bin:/bin",
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
        stderr=subprocess.PIPE,
        text=True,
        timeout=10,
    ).stdout.strip()
    rust_sysroot = Path(sysroot_output).resolve(strict=True)
    if not (rust_sysroot / "bin/cargo").is_file() or not (rust_sysroot / "bin/rustc").is_file():
        raise RuntimeError("resolved Rust sysroot does not contain direct cargo and rustc binaries")

    source_stage = state / "cargo-source"
    registry_stage = state / "cargo-registry"
    artifacts_dir = state / "rust-artifacts"
    artifacts_dir.mkdir(mode=0o700)
    process_e2e.copy_source_tree(source_stage)
    cargo_home = Path(
        os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
    ).resolve(strict=True)
    process_e2e.copy_locked_registry(cargo_home / "registry", registry_stage)

    names = ["forgejo-webhook-local", "forgejo-reconcile-local"]
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
        bwrap,
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
            str(prlimit),
            "--cpu=240",
            "--as=4294967296",
            "--nproc=384",
            "--nofile=128",
            "--fsize=67108864",
            "--",
            "/usr/bin/python3",
            "-c",
            helper,
            json.dumps(names),
        ]
    )
    artifacts = {name: artifacts_dir / name for name in names}
    build_log = io.StringIO()
    try:
        with redirect_stdout(build_log):
            process_e2e.run_artifact_build(command, cwd=repo, artifacts=artifacts)
    except Exception as error:
        forgejo_helpers.write_private(evidence_dir / "cargo-build.log", build_log.getvalue().encode())
        raise RuntimeError("offline Bubblewrap build of the webhook CLIs failed") from error
    forgejo_helpers.write_private(evidence_dir / "cargo-build.log", build_log.getvalue().encode())

    tool_env = {"PATH": "/usr/bin:/bin", "HOME": str(state / "home"), "TMPDIR": str(state / "tmp")}
    versions = {}
    for name, executable in (
        ("rustc", rust_sysroot / "bin/rustc"),
        ("cargo", rust_sysroot / "bin/cargo"),
    ):
        version = subprocess.run(
            [str(executable), "--version"],
            cwd="/",
            env=tool_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=10,
            check=True,
        )
        versions[name] = version.stdout.strip()
    build_provenance = {
        "policy": "Bubblewrap allowlist; --unshare-net and --unshare-pid; clear environment; read-only staged source, registry and Rust sysroot; --offline --locked",
        "bubblewrap_version": subprocess.run(
            [str(bwrap), "--version"],
            cwd="/",
            env=tool_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=10,
            check=True,
        ).stdout.strip(),
        "rustc_sha256": sha256_file(rust_sysroot / "bin/rustc"),
        "cargo_sha256": sha256_file(rust_sysroot / "bin/cargo"),
        "bubblewrap_sha256": sha256_file(bwrap),
        "staged_source_tree_sha256": process_e2e.sha256_tree(source_stage),
        "staged_registry_tree_sha256": process_e2e.sha256_tree(registry_stage),
        "built_executable_sha256": {
            name: sha256_file(path) for name, path in artifacts.items()
        },
        "toolchain_versions": versions,
    }
    return artifacts[names[0]], artifacts[names[1]], build_provenance


def write_safe_json(path: Path, value: Any, tokens: list[str]) -> None:
    encoded = json.dumps(value, sort_keys=True, indent=2).encode() + b"\n"
    if any(token.encode() in encoded for token in tokens):
        raise RuntimeError("refusing to persist evidence containing a private token")
    forgejo_helpers.write_private(path, encoded)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_checksums(evidence_dir: Path) -> None:
    lines = []
    for path in sorted(item for item in evidence_dir.iterdir() if item.is_file()):
        if path.name == "SHA256SUMS":
            continue
        lines.append(f"{sha256_file(path)}  {path.name}")
    forgejo_helpers.write_private(evidence_dir / "SHA256SUMS", ("\n".join(lines) + "\n").encode())


def run(args: argparse.Namespace) -> dict[str, Any]:
    binary_arg = Path(args.forgejo_binary)
    if binary_arg.is_symlink():
        raise RuntimeError("Forgejo binary must not be a symlink")
    forgejo = binary_arg.resolve(strict=True)
    repo = Path(args.repo_root).resolve(strict=True)
    state = Path(args.state_dir)
    evidence_dir = Path(args.evidence_dir).resolve()
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("state dir must be a new direct child of /tmp")
    if evidence_dir.exists():
        raise RuntimeError("refusing to overwrite existing evidence directory")
    if not (repo / "Cargo.toml").is_file():
        raise RuntimeError("repo root must contain Cargo.toml")
    if not forgejo.is_file():
        raise RuntimeError("Forgejo binary must resolve to a regular file")
    if shutil.which("rustc") is None or shutil.which("bwrap") is None:
        raise RuntimeError("rustc and Bubblewrap are required for the isolated local source build")
    for port in (3130,):
        with socket.socket() as probe:
            if probe.connect_ex(("127.0.0.1", port)) == 0:
                raise RuntimeError(f"127.0.0.1:{port} is already in use")

    version = subprocess.run(
        [str(forgejo), "--version"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env={"PATH": "/usr/bin:/bin", "HOME": "/tmp"},
        timeout=10,
        check=False,
    )
    if version.returncode or FORGEJO_VERSION not in version.stdout:
        raise RuntimeError("this E2E requires the locally verified Forgejo 17.0.0 binary")

    state.mkdir(mode=0o700)
    (state / "home").mkdir(mode=0o700)
    (state / "tmp").mkdir(mode=0o700)
    (state / "log").mkdir(mode=0o700)
    forgejo_helpers.create_app_ini(state)
    evidence_dir.mkdir(mode=0o700, parents=True)
    tokens: list[str] = []
    server_log = (state / "log/forgejo-server.log").open("wb")
    receiver_log = (state / "log/webhook-receiver.log").open("wb")
    server: subprocess.Popen | None = None
    receiver: subprocess.Popen | None = None
    receiver_port: int | None = None
    safe_result: dict[str, Any] | None = None
    build_provenance: dict[str, Any] | None = None
    process_cleanup: dict[str, Any] | None = None
    try:
        receiver_bin, reconcile_bin, build_provenance = build_local_binaries(
            repo,
            state,
            evidence_dir,
        )

        server = subprocess.Popen(
            [
                str(forgejo),
                "--work-path",
                str(state),
                "--config",
                str(state / "custom/conf/app.ini"),
                "web",
            ],
            cwd=state,
            env=private_command_environment(state),
            stdout=server_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        forgejo_helpers.wait_for_health(server)
        forgejo_helpers.assert_forgejo_listener(server)

        admin_command(
            forgejo,
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
        bootstrap_token = create_access_token(
            forgejo, state, OWNER, "live-webhook-repository-provisioning", "write:user,write:repository"
        )
        tokens.append(bootstrap_token)
        bootstrap_api = LocalForgejoApi(bootstrap_token)
        created_repo, _ = bootstrap_api.request(
            "POST",
            "/user/repos",
            {
                "name": REPOSITORY,
                "description": "Private local webhook reconciliation E2E",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if created_repo.get("private") is not True or created_repo.get("full_name") != f"{OWNER}/{REPOSITORY}":
            raise RuntimeError("Forgejo did not create the exact private allowlisted repository")
        bootstrap_api = None
        tokens.remove(bootstrap_token)
        bootstrap_token = None

        owner_token = create_access_token(
            forgejo,
            state,
            OWNER,
            "live-webhook-owner-provisioning",
            "read:user,read:repository,read:issue,write:repository,write:issue",
        )
        tokens.append(owner_token)
        owner_api = LocalForgejoApi(owner_token)
        admin_command(
            forgejo,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                READER,
                "--random-password",
                "--email",
                f"{READER}@local.invalid",
                "--must-change-password=false",
            ],
        )
        reader_scopes = "read:user,read:repository,read:issue"
        reader_token = create_access_token(
            forgejo, state, READER, "live-webhook-read-only", reader_scopes
        )
        tokens.append(reader_token)
        reader_api = LocalForgejoApi(reader_token)
        reader_account, _ = reader_api.request("GET", "/user")
        if reader_account.get("login") != READER or reader_account.get("is_admin") is not False:
            raise RuntimeError("Forgejo did not confirm the API reader is a non-admin")
        owner_api.request(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{quote(READER, safe='')}",
            {"permission": "read"},
        )
        repo_readback, _ = reader_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}")
        permissions = repo_readback.get("permissions")
        if (
            repo_readback.get("private") is not True
            or not isinstance(permissions, dict)
            or permissions.get("admin") is not False
            or permissions.get("push") is not False
            or permissions.get("pull") is not True
        ):
            raise RuntimeError("Forgejo did not confirm private read-only collaborator permissions")

        issue, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {"title": "Live local reconciliation", "body": "State is checked against the Forgejo API."},
        )
        issue_number = issue.get("number")
        if not isinstance(issue_number, int) or issue_number < 1 or issue.get("state") != "open":
            raise RuntimeError("Forgejo did not create the expected open private issue")
        current_issue, _ = reader_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        )
        if current_issue.get("number") != issue_number or current_issue.get("state") != "open":
            raise RuntimeError("read-only account could not read the current private issue")

        webhook_secret = secrets.token_bytes(32)
        secret_path = state / "webhook.secret"
        forgejo_helpers.write_private(secret_path, webhook_secret)
        journal_path = state / "data/webhook-inbox.jsonl"
        receiver_port = select_receiver_port()
        receiver_address = f"127.0.0.1:{receiver_port}"
        receiver = subprocess.Popen(
            [str(receiver_bin), receiver_address, str(secret_path), str(journal_path)],
            cwd=state,
            env=private_command_environment(state),
            stdout=receiver_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        wait_for_listener(receiver, receiver_port)

        stale_body = json.dumps(
            {
                "action": "closed",
                "repository": {"full_name": f"{OWNER}/{REPOSITORY}"},
                "issue": {
                    "number": issue_number,
                    "state": "closed",
                    "html_url": f"{ORIGIN}/{OWNER}/{REPOSITORY}/issues/{issue_number}",
                },
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        receiver_first = send_signed_webhook(receiver_address, webhook_secret, DELIVERY_ID, stale_body)
        receiver_duplicate = send_signed_webhook(receiver_address, webhook_secret, DELIVERY_ID, stale_body)
        if (
            receiver_first["http_status"] != 202
            or receiver_first["body"].get("accepted") is not True
            or receiver_first["body"].get("duplicate") is not False
            or receiver_first["body"].get("reconcile_required") is not True
            or receiver_duplicate["http_status"] != 202
            or receiver_duplicate["body"].get("duplicate") is not True
            or receiver_duplicate["body"].get("sequence") != receiver_first["body"].get("sequence")
        ):
            raise RuntimeError("receiver did not accept then idempotently deduplicate the signed delivery")

        reconcile_status, reconciled = run_reconcile(
            reconcile_bin, state, journal_path, DELIVERY_ID, reader_token
        )
        if (
            reconcile_status != 0
            or reconciled.get("verdict") != "current_state_read"
            or reconciled.get("upstream_read_attempts") != 1
            or reconciled.get("resource", {}).get("number") != issue_number
            or reconciled.get("resource", {}).get("owner") != OWNER
            or reconciled.get("resource", {}).get("repository") != REPOSITORY
            or reconciled.get("canonical_state", {}).get("state") != "open"
            or reconciled.get("canonical_state", {}).get("html_url")
            != f"{ORIGIN}/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        ):
            raise RuntimeError("CLI failed to prefer current Forgejo state over the stale webhook payload")
        if "canonical_state" in reconciled.get("error", {}):
            raise RuntimeError("unexpected error result included canonical state")

        invalid_token = "invalid-" + secrets.token_urlsafe(24)
        tokens.append(invalid_token)
        invalid_status, invalid_result = run_reconcile(
            reconcile_bin, state, journal_path, DELIVERY_ID, invalid_token
        )
        if (
            invalid_status == 0
            or invalid_result.get("verdict") != "inconclusive"
            or invalid_result.get("error", {}).get("code") != "upstream_unauthorized"
            or invalid_result.get("error", {}).get("retryable") is not False
            or invalid_result.get("upstream_read_attempts") != 1
            or "canonical_state" in invalid_result
        ):
            raise RuntimeError("invalid Forgejo credential did not remain inconclusive without canonical state")

        post_issue, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        )
        if post_issue.get("state") != "open":
            raise RuntimeError("the live issue changed during a read-only reconciliation run")

        commit = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "HEAD"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=10,
            check=True,
        ).stdout.strip()
        safe_result = {
            "schema_version": 1,
            "verdict": "passed",
            "forgejo": {
                "version": version.stdout.strip(),
                "binary_sha256": sha256_file(forgejo),
                "listener": "127.0.0.1:3130",
            },
            "repository": {
                "full_name": f"{OWNER}/{REPOSITORY}",
                "private": True,
                "api_reader": {
                    "login": READER,
                    "is_admin": reader_account.get("is_admin"),
                    "repo_permission_admin": permissions.get("admin"),
                    "repo_permission_push": permissions.get("push"),
                    "repo_permission_pull": permissions.get("pull"),
                    "requested_token_scopes": reader_scopes.split(","),
                },
            },
            "issue_number": issue_number,
            "webhook": {
                "delivery_id": DELIVERY_ID,
                "payload_state": "closed",
                "payload_origin": "synthetic local signed webhook; Forgejo sender was not configured",
                "first_delivery": receiver_first,
                "duplicate_delivery": receiver_duplicate,
            },
            "reconciliation": reconciled,
            "invalid_credential": {
                "exit_status": invalid_status,
                "result": invalid_result,
                "credential_material_recorded": False,
            },
            "post_run_issue_state": post_issue.get("state"),
            "source_head_commit": commit,
            "limitations": [
                "No LLM or AgentOS execution was used.",
                "The webhook delivery was synthesized locally; Forgejo-to-receiver delivery was not tested.",
                "The build ran in a Bubblewrap no-network allowlist; runtime services ran as the current user, so OS privilege isolation and malicious candidate execution were not tested.",
                "This validates one isolated local Forgejo v17 issue-read path, not production deployment or general value.",
            ],
        }
    finally:
        receiver_stop = stop_process(receiver)
        server_stop = stop_process(server)
        process_cleanup = {
            "forgejo": {
                "expected_address": "127.0.0.1:3130",
                "stop": server_stop,
                "listener_addresses_after_stop": (
                    listener_addresses_for_port(3130) if server is not None else []
                ),
                "listener_checked": server is not None,
            },
            "receiver": {
                "expected_address": f"127.0.0.1:{receiver_port}" if receiver_port is not None else None,
                "stop": receiver_stop,
                "listener_addresses_after_stop": (
                    listener_addresses_for_port(receiver_port)
                    if receiver is not None and receiver_port is not None
                    else []
                ),
                "listener_checked": receiver is not None and receiver_port is not None,
            },
        }
        process_cleanup["tracked_processes_exited"] = bool(
            server_stop["stopped"] and receiver_stop["stopped"]
        )
        process_cleanup["expected_tcp_listeners_absent"] = not any(
            process_cleanup[service]["listener_addresses_after_stop"]
            for service in ("forgejo", "receiver")
        )
        write_safe_json(evidence_dir / "port-cleanup-readback.json", process_cleanup, tokens)
        receiver_log.close()
        server_log.close()

    assert safe_result is not None
    assert process_cleanup is not None
    assert build_provenance is not None
    safe_result["process_cleanup"] = process_cleanup
    if (
        not process_cleanup["tracked_processes_exited"]
        or not process_cleanup["expected_tcp_listeners_absent"]
    ):
        raise RuntimeError("local Forgejo or webhook receiver did not stop cleanly")
    write_safe_json(evidence_dir / "result.json", safe_result, tokens)
    write_safe_json(evidence_dir / "receiver-first-delivery.json", receiver_first, tokens)
    write_safe_json(evidence_dir / "receiver-duplicate-delivery.json", receiver_duplicate, tokens)
    write_safe_json(evidence_dir / "current-state-read.json", reconciled, tokens)
    write_safe_json(
        evidence_dir / "invalid-credential-result.json",
        {"exit_status": invalid_status, "result": invalid_result},
        tokens,
    )
    write_safe_json(
        evidence_dir / "reader-role-readback.json",
        safe_result["repository"]["api_reader"],
        tokens,
    )
    provenance = {
        "head_commit": safe_result["source_head_commit"],
        "source_worktree_status": subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "status",
                "--porcelain",
                "--",
                "Cargo.toml",
                "Cargo.lock",
                "src",
                "scripts/run_forgejo_live_webhook_reconcile_e2e.py",
                "scripts/run_forgejo_live_pr_merge_e2e.py",
                "scripts/run_forgejo_mission_delivery_process_e2e.py",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=10,
            check=True,
        ).stdout.splitlines(),
        "forgejo_binary_sha256": safe_result["forgejo"]["binary_sha256"],
        "source_sha256": {
            str(path.relative_to(repo)): sha256_file(path)
            for path in sorted(
                {
                    repo / "Cargo.toml",
                    repo / "Cargo.lock",
                    repo / "scripts/run_forgejo_live_webhook_reconcile_e2e.py",
                    repo / "scripts/run_forgejo_live_pr_merge_e2e.py",
                    repo / "scripts/run_forgejo_mission_delivery_process_e2e.py",
                    *((repo / "src").rglob("*.rs")),
                }
            )
        },
        "build_tools": {"python": sys.version.split()[0]},
        "cargo_build": build_provenance,
        "built_executable_sha256": {
            "forgejo-webhook-local": sha256_file(receiver_bin),
            "forgejo-reconcile-local": sha256_file(reconcile_bin),
        },
        "port_cleanup_readback_sha256": sha256_file(
            evidence_dir / "port-cleanup-readback.json"
        ),
        "private_state_dir": str(state),
        "private_state_dir_is_not_in_evidence": True,
    }
    write_safe_json(evidence_dir / "provenance.json", provenance, tokens)
    write_checksums(evidence_dir)
    return safe_result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(SCRIPT_DIR.parent))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    args = parser.parse_args()
    result = run(args)
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
