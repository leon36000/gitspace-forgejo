#!/usr/bin/env python3
"""Reconcile a real branch-creation push emitted by disposable local Forgejo."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import ProxyHandler, Request, build_opener
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_e2e as helpers  # noqa: E402
import run_forgejo_live_webhook_reconcile_emitted as emitted  # noqa: E402

OWNER = helpers.OWNER
REPOSITORY = helpers.REPOSITORY
FOREIGN_REPOSITORY = "native-push-out-of-scope-proof"
ORIGIN = helpers.ORIGIN
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
BRANCH = "agent/native-push-proof"
FOREIGN_BRANCH = "agent/out-of-scope-proof"


def anonymous_repository_status(owner: str, repository: str) -> int:
    path = f"/api/v1/repos/{quote(owner, safe='')}/{quote(repository, safe='')}"
    request = Request(ORIGIN + path, headers={"Accept": "application/json"}, method="GET")
    opener = build_opener(ProxyHandler({}), helpers.NoRedirect)
    try:
        with opener.open(request, timeout=5) as response:
            return int(response.status)
    except HTTPError as error:
        error.close()
        return int(error.code)
    except URLError:
        raise RuntimeError("anonymous Forgejo repository read failed at transport") from None


def validate_private_repository_anonymous_status(
    *, is_private: object, anonymous_status: object
) -> None:
    if is_private is not True:
        raise ValueError("Forgejo did not confirm the private repository")
    if type(anonymous_status) is not int or anonymous_status != 404:
        raise ValueError("anonymous API read did not hide the private repository")


def run_git(args: list[str], *, cwd: Path | None, env: dict[str, str], stage: str) -> str:
    process = subprocess.run(
        ["git", *args],
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=60,
        check=False,
    )
    if process.returncode:
        raise RuntimeError(f"local Git {stage} failed ({process.returncode})")
    return process.stdout.decode("utf-8", errors="strict").strip()


def run_reconcile_batch(
    binary: Path,
    state: Path,
    journal: Path,
    delivery_ids: list[str],
    credential: str,
) -> tuple[int, dict[str, Any], bytes]:
    if len(delivery_ids) < 2 or len(delivery_ids) > 100 or len(set(delivery_ids)) != len(delivery_ids):
        raise ValueError("batch requires 2-100 unique delivery IDs")
    credential_bytes = credential.encode("utf-8")
    read_fd, write_fd = os.pipe()
    try:
        view = memoryview(credential_bytes)
        while view:
            written = os.write(write_fd, view)
            if written <= 0:
                raise OSError("short write to private reconciliation credential pipe")
            view = view[written:]
    finally:
        os.close(write_fd)
    try:
        completed = subprocess.run(
            [str(binary), "--host-credential-fd", str(read_fd), str(journal), *delivery_ids],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=helpers.private_command_environment(state),
            pass_fds=(read_fd,),
            timeout=20,
            check=False,
        )
        remaining_credential = bytearray()
        while True:
            chunk = os.read(read_fd, len(credential_bytes) + 1)
            if not chunk:
                break
            remaining_credential.extend(chunk)
    finally:
        os.close(read_fd)
    if credential_bytes in completed.stdout or credential_bytes in completed.stderr:
        raise RuntimeError("batch reconciliation CLI exposed its private credential")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError:
        raise RuntimeError("batch reconciliation CLI did not return structured JSON") from None
    if not isinstance(value, dict):
        raise RuntimeError("batch reconciliation CLI returned a non-object result")
    return completed.returncode, value, bytes(remaining_credential)


def validate_push_reconcile_batch(
    exit_status: int,
    requested_events: list[dict[str, Any]],
    batch: dict[str, Any],
    *,
    owner: str,
    repository: str,
    branch: str,
    current_head_sha: str,
) -> list[dict[str, Any]]:
    if type(exit_status) is not int or exit_status != 0:
        raise ValueError("push reconciliation batch did not exit successfully")
    if not isinstance(batch, dict):
        raise ValueError("push reconciliation batch is not an object")
    if (
        batch.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or batch.get("api_version") != "forgejo-rest-v1"
        or batch.get("verdict") != "current_state_read"
    ):
        raise ValueError("push reconciliation batch envelope is invalid")
    attempts = batch.get("upstream_read_attempts")
    if type(attempts) is not int or attempts != 1:
        raise ValueError("same-branch push batch must use one upstream read")
    results = batch.get("results")
    if (
        not isinstance(requested_events, list)
        or len(requested_events) < 2
        or not isinstance(results, list)
        or len(results) != len(requested_events)
    ):
        raise ValueError("push reconciliation batch result count is invalid")
    expected_html_url = (
        f"{ORIGIN}/{owner}/{repository}/src/branch/{quote(branch, safe='')}"
    )
    checked: list[dict[str, Any]] = []
    seen: set[str] = set()
    for event, result in zip(requested_events, results, strict=True):
        delivery_id = event.get("delivery_id") if isinstance(event, dict) else None
        if not isinstance(delivery_id, str) or delivery_id in seen:
            raise ValueError("requested push deliveries are duplicate or invalid")
        seen.add(delivery_id)
        emitted.validate_reconciliation_delivery_binding(event, result)
        resource = result.get("resource") if isinstance(result, dict) else None
        canonical = result.get("canonical_state") if isinstance(result, dict) else None
        if (
            not isinstance(result, dict)
            or result.get("verdict") != "current_state_read"
            or not isinstance(resource, dict)
            or resource.get("kind") != "branch"
            or resource.get("owner") != owner
            or resource.get("repository") != repository
            or resource.get("branch") != branch
            or not isinstance(canonical, dict)
            or canonical.get("state") != "present"
            or canonical.get("head_sha") != current_head_sha
            or canonical.get("html_url") != expected_html_url
        ):
            raise ValueError("push batch item is not the current canonical branch state")
        checked.append(result)
    return checked


def validate_partial_push_batch_refusal(
    exit_status: int,
    requested_delivery_ids: list[str],
    result: dict[str, Any],
    *,
    missing_delivery_id: str,
    credential_preserved: bool,
) -> None:
    if type(exit_status) is not int or exit_status != 1:
        raise ValueError("partial push batch refusal did not exit with a refusal status")
    if (
        not isinstance(requested_delivery_ids, list)
        or not 2 <= len(requested_delivery_ids) <= 100
        or any(not isinstance(item, str) or not item for item in requested_delivery_ids)
        or len(set(requested_delivery_ids)) != len(requested_delivery_ids)
        or not isinstance(missing_delivery_id, str)
        or missing_delivery_id not in requested_delivery_ids
    ):
        raise ValueError("partial push batch missing delivery selection is invalid")
    if credential_preserved is not True:
        raise ValueError("partial push batch consumed the credential before refusing")
    if not isinstance(result, dict) or (
        result.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("verdict") != "inconclusive"
        or type(result.get("requested_delivery_count")) is not int
        or result.get("requested_delivery_count") != len(requested_delivery_ids)
    ):
        raise ValueError("partial push batch refusal envelope is invalid")
    attempts = result.get("upstream_read_attempts")
    if type(attempts) is not int or attempts != 0:
        raise ValueError("partial push batch refusal attempted an upstream read")
    error = result.get("error")
    details = error.get("details") if isinstance(error, dict) else None
    if (
        not isinstance(error, dict)
        or error.get("code") != "delivery_not_found"
        or error.get("retryable") is not False
        or not isinstance(error.get("action"), str)
        or not error["action"].strip()
        or not isinstance(details, dict)
        or details.get("delivery_id") != missing_delivery_id
        or details.get("source") != "verified_inbox"
    ):
        raise ValueError("partial push batch refusal does not identify the missing delivery")


def validate_out_of_scope_push_batch_refusal(
    exit_status: int,
    requested_events: list[dict[str, Any]],
    result: dict[str, Any],
    *,
    configured_repository: str,
    event_repository: str,
    credential_preserved: bool,
) -> None:
    if type(exit_status) is not int or exit_status != 1:
        raise ValueError("out-of-scope push batch did not exit with a refusal status")
    if credential_preserved is not True:
        raise ValueError("out-of-scope push batch consumed the host credential")
    if (
        not isinstance(requested_events, list)
        or len(requested_events) < 2
        or any(not isinstance(event, dict) for event in requested_events)
        or any(
            not isinstance(event.get("delivery_id"), str) or not event["delivery_id"]
            for event in requested_events
        )
        or len({event.get("delivery_id") for event in requested_events}) != len(requested_events)
    ):
        raise ValueError("out-of-scope push delivery selection is invalid")
    if not isinstance(result, dict) or (
        result.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("verdict") != "inconclusive"
    ):
        raise ValueError("out-of-scope push batch envelope is invalid")
    if type(result.get("upstream_read_attempts")) is not int or result["upstream_read_attempts"] != 0:
        raise ValueError("out-of-scope push batch attempted an upstream read")
    rows = result.get("results")
    if not isinstance(rows, list) or len(rows) != len(requested_events):
        raise ValueError("out-of-scope push batch result count is invalid")
    for event, row in zip(requested_events, rows, strict=True):
        error = row.get("error") if isinstance(row, dict) else None
        details = error.get("details") if isinstance(error, dict) else None
        if (
            not isinstance(row, dict)
            or row.get("verdict") != "inconclusive"
            or row.get("delivery_id") != event.get("delivery_id")
            or row.get("canonical_state") is not None
            or not isinstance(error, dict)
            or error.get("code") != "resource_out_of_scope"
            or error.get("retryable") is not False
            or not isinstance(error.get("action"), str)
            or "matching configured repository" not in error["action"]
            or not isinstance(details, dict)
            or details.get("configured_repository") != configured_repository
            or details.get("event_repository") != event_repository
        ):
            raise ValueError("out-of-scope push refusal is not actionable and delivery-bound")


def validate_network_isolation_observation(
    *,
    host_network_namespace_inode: int,
    sandbox_network_namespace_inode: int,
    host_listener_reachable: bool,
    sandbox_loopback_probe_connected: bool,
    parent_listener_connections: int,
    journal_unchanged: bool,
) -> None:
    if (
        type(host_network_namespace_inode) is not int
        or host_network_namespace_inode < 0
        or type(sandbox_network_namespace_inode) is not int
        or sandbox_network_namespace_inode < 0
        or host_network_namespace_inode == sandbox_network_namespace_inode
    ):
        raise ValueError("network namespaces are not independently isolated")
    if host_listener_reachable is not True:
        raise ValueError("host loopback listener was not reachable as a positive control")
    if sandbox_loopback_probe_connected is not False:
        raise ValueError("sandbox loopback probe connected to the host listener")
    if type(parent_listener_connections) is not int or parent_listener_connections != 0:
        raise ValueError("parent listener received a connection from the sandbox")
    if journal_unchanged is not True:
        raise ValueError("network-isolated refusal changed the inbox journal")


def run_reconcile_batch_network_isolated(
    binary: Path,
    state: Path,
    journal: Path,
    delivery_ids: list[str],
) -> tuple[int, dict[str, Any], bool, dict[str, Any]]:
    if len(delivery_ids) < 2 or len(delivery_ids) > 100 or len(set(delivery_ids)) != len(delivery_ids):
        raise ValueError("batch requires 2-100 unique delivery IDs")
    if state.is_symlink() or binary.is_symlink() or journal.is_symlink():
        raise ValueError("network-isolated reconcile inputs must not be symlinks")
    if (state / "data").is_symlink():
        raise ValueError("network-isolated reconcile data directory must not be a symlink")
    state = state.resolve(strict=True)
    binary = binary.resolve(strict=True)
    journal = journal.resolve(strict=True)
    if state.parent != Path("/tmp") or not (state / "data").is_dir():
        raise ValueError("network-isolated reconcile state must be a direct child of /tmp")
    if not binary.is_file() or binary.is_symlink():
        raise ValueError("network-isolated reconcile binary must be a regular file")
    if journal.parent != state / "data" or journal.is_symlink() or not journal.is_file():
        raise ValueError("network-isolated reconcile journal must be a regular file in the private data dir")
    bwrap = shutil.which("bwrap")
    if bwrap is None:
        raise RuntimeError("Bubblewrap is required for the network-isolated refusal check")

    host_namespace_inode = os.stat("/proc/self/ns/net").st_ino
    journal_sha256_before = helpers.sha256_file(journal)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 3130))
        listener.listen(8)
        listener.setblocking(False)
        try:
            host_probe = socket.create_connection(("127.0.0.1", 3130), timeout=2)
            positive_control = True
            host_connection, _ = listener.accept()
        except (OSError, TimeoutError):
            raise RuntimeError("host loopback listener positive control failed") from None
        finally:
            if "host_probe" in locals():
                host_probe.close()
        host_connection.close()

        sandbox_root = Path("/tmp/forgejo-network-guard")
        sandbox_data = sandbox_root / "data"
        sandbox_binary = sandbox_root / "forgejo-reconcile-local"
        wrapper = r'''import json, os, socket, subprocess, sys
binary, journal, host_namespace, *delivery_ids = sys.argv[1:]
credential = sys.stdin.buffer.read(256)
if not credential or len(credential) >= 256:
    raise SystemExit(90)
read_fd, write_fd = os.pipe()
try:
    remaining = memoryview(credential)
    while remaining:
        written = os.write(write_fd, remaining)
        if written <= 0:
            raise SystemExit(91)
        remaining = remaining[written:]
finally:
    os.close(write_fd)
try:
    completed = subprocess.run(
        [binary, "--host-credential-fd", str(read_fd), journal, *delivery_ids],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={"PATH": "/usr/bin:/bin", "HOME": "/tmp", "TMPDIR": "/tmp"},
        pass_fds=(read_fd,),
        timeout=20,
        check=False,
    )
    pending = bytearray()
    while True:
        chunk = os.read(read_fd, len(credential) + 1)
        if not chunk:
            break
        pending.extend(chunk)
finally:
    os.close(read_fd)
if credential in completed.stdout or credential in completed.stderr:
    raise SystemExit(92)
try:
    result = json.loads(completed.stdout)
except json.JSONDecodeError:
    raise SystemExit(93) from None
probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
probe.settimeout(1)
loopback_connected = probe.connect_ex(("127.0.0.1", 3130)) == 0
probe.close()
print(json.dumps({
    "cli_exit_status": completed.returncode,
    "result": result,
    "credential_preserved": bytes(pending) == credential,
    "host_network_namespace_inode": int(host_namespace),
    "sandbox_network_namespace_inode": os.stat("/proc/self/ns/net").st_ino,
    "sandbox_loopback_probe_connected": loopback_connected,
}, sort_keys=True))
'''
        command = [
            bwrap,
            "--die-with-parent",
            "--new-session",
            "--unshare-net",
            "--ro-bind", "/", "/",
            "--tmpfs", "/home",
            "--tmpfs", "/root",
            "--tmpfs", "/run",
            "--tmpfs", "/tmp",
            "--dir", str(sandbox_root),
            "--dir", str(sandbox_data),
            "--bind", str(state / "data"), str(sandbox_data),
            "--ro-bind", str(binary), str(sandbox_binary),
            "--proc", "/proc",
            "--dev", "/dev",
            "--clearenv",
            "--setenv", "PATH", "/usr/bin:/bin",
            "--setenv", "HOME", "/tmp",
            "--setenv", "TMPDIR", "/tmp",
            "--",
            sys.executable,
            "-c",
            wrapper,
            str(sandbox_binary),
            str(sandbox_data / journal.name),
            str(host_namespace_inode),
            *delivery_ids,
        ]
        synthetic_credential = b"NETWORK-ISOLATED-CREDENTIAL-CANARY"
        completed = subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={"PATH": "/usr/bin:/bin", "HOME": "/tmp"},
            input=synthetic_credential,
            timeout=35,
            check=False,
            close_fds=True,
        )
        unexpected_connections = 0
        while True:
            try:
                connection, _ = listener.accept()
            except BlockingIOError:
                break
            unexpected_connections += 1
            connection.close()

    journal_unchanged = helpers.sha256_file(journal) == journal_sha256_before
    if completed.returncode != 0 or synthetic_credential in completed.stdout or synthetic_credential in completed.stderr:
        raise RuntimeError("network-isolated reconcile supervisor failed safely")
    try:
        observation = json.loads(completed.stdout)
    except json.JSONDecodeError:
        raise RuntimeError("network-isolated reconcile supervisor returned invalid JSON") from None
    if not isinstance(observation, dict) or not isinstance(observation.get("result"), dict):
        raise RuntimeError("network-isolated reconcile supervisor returned an invalid envelope")
    network_evidence = {
        "host_network_namespace_inode": observation.get("host_network_namespace_inode"),
        "sandbox_network_namespace_inode": observation.get("sandbox_network_namespace_inode"),
        "host_listener_reachable": positive_control,
        "sandbox_loopback_probe_connected": observation.get("sandbox_loopback_probe_connected"),
        "parent_listener_connections": unexpected_connections,
        "journal_unchanged": journal_unchanged,
    }
    validate_network_isolation_observation(**network_evidence)
    return (
        observation.get("cli_exit_status"),
        observation["result"],
        observation.get("credential_preserved") is True,
        network_evidence,
    )


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
    if not (repo / "Cargo.toml").is_file() or not forgejo.is_file():
        raise RuntimeError("a Cargo repository and regular Forgejo binary are required")
    if shutil.which("git") is None or shutil.which("rustc") is None or shutil.which("bwrap") is None:
        raise RuntimeError("git, rustc and Bubblewrap are required")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3130)) == 0:
            raise RuntimeError("127.0.0.1:3130 is already in use")

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
    helpers.forgejo_helpers.create_app_ini(state)
    emitted.private_config_with_test_allowlist(state)
    evidence_dir.mkdir(mode=0o700, parents=True)

    tokens: list[str] = []
    server_log = (state / "log/forgejo-server.log").open("wb")
    receiver_log = (state / "log/webhook-receiver.log").open("wb")
    server: subprocess.Popen[bytes] | None = None
    receiver: subprocess.Popen[bytes] | None = None
    receiver_port: int | None = None
    receiver_bin: Path | None = None
    reconcile_bin: Path | None = None
    build_provenance: dict[str, Any] | None = None
    owner_api: helpers.LocalForgejoApi | None = None
    hook_id: int | None = None
    hook_disabled = False
    foreign_hook_id: int | None = None
    foreign_hook_disabled = False
    hook_readback: dict[str, Any] | None = None
    foreign_hook_readback: dict[str, Any] | None = None
    expected_hook_url: str | None = None
    webhook_secret: str | None = None
    owner_token: str | None = None
    reader_token: str | None = None
    branch_readback: dict[str, Any] | None = None
    push_events: list[dict[str, Any]] = []
    hook_tasks: list[dict[str, Any]] = []
    foreign_push_events: list[dict[str, Any]] = []
    foreign_hook_tasks: list[dict[str, Any]] = []
    reconciliations: list[dict[str, Any]] = []
    journal_path = state / "data/webhook-inbox.jsonl"
    stage = "isolated_build"
    failure: str | None = None
    cleanup_failure: str | None = None
    server_stop: dict[str, Any] | None = None
    receiver_stop: dict[str, Any] | None = None
    safe_result: dict[str, Any] | None = None

    try:
        receiver_bin, reconcile_bin, build_provenance = helpers.build_local_binaries(
            repo, state, evidence_dir
        )
        stage = "start_forgejo"
        server = subprocess.Popen(
            [
                str(forgejo), "--work-path", str(state), "--config",
                str(state / "custom/conf/app.ini"), "web",
            ],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=server_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        helpers.forgejo_helpers.wait_for_health(server)
        helpers.forgejo_helpers.assert_forgejo_listener(server)

        stage = "provision_private_repository"
        helpers.admin_command(
            forgejo,
            state,
            [
                "admin", "user", "create", "--username", OWNER,
                "--random-password", "--email", f"{OWNER}@local.invalid",
                "--admin", "--must-change-password=false",
            ],
        )
        bootstrap_token = helpers.create_access_token(
            forgejo, state, OWNER, "native-push-bootstrap", "write:user,write:repository"
        )
        tokens.append(bootstrap_token)
        bootstrap_api = helpers.LocalForgejoApi(bootstrap_token)
        created_repo, _ = bootstrap_api.request(
            "POST",
            "/user/repos",
            {
                "name": REPOSITORY,
                "description": "Private local native push reconciliation E2E",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if created_repo.get("private") is not True or created_repo.get("full_name") != f"{OWNER}/{REPOSITORY}":
            raise RuntimeError("Forgejo did not create the expected private repository")
        stage = "provision_foreign_private_repository"
        foreign_repo_readback, _ = bootstrap_api.request(
            "POST",
            "/user/repos",
            {
                "name": FOREIGN_REPOSITORY,
                "description": "Private local out-of-scope push refusal E2E",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if (
            foreign_repo_readback.get("private") is not True
            or foreign_repo_readback.get("full_name") != f"{OWNER}/{FOREIGN_REPOSITORY}"
        ):
            raise RuntimeError("Forgejo did not create the expected private foreign repository")
        tokens.remove(bootstrap_token)
        bootstrap_api = None

        owner_token = helpers.create_access_token(
            forgejo,
            state,
            OWNER,
            "native-push-owner",
            "read:user,read:repository,read:issue,write:repository,write:issue",
        )
        tokens.append(owner_token)
        owner_api = helpers.LocalForgejoApi(owner_token)
        helpers.admin_command(
            forgejo,
            state,
            [
                "admin", "user", "create", "--username", helpers.READER,
                "--random-password", "--email", f"{helpers.READER}@local.invalid",
                "--must-change-password=false",
            ],
        )
        reader_token = helpers.create_access_token(
            forgejo,
            state,
            helpers.READER,
            "native-push-read-only",
            "read:user,read:repository",
        )
        tokens.append(reader_token)
        reader_api = helpers.LocalForgejoApi(reader_token)
        reader_account, _ = reader_api.request("GET", "/user")
        owner_api.request(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{helpers.READER}",
            {"permission": "read"},
        )
        repo_readback, _ = reader_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}")
        permissions = repo_readback.get("permissions")
        if (
            reader_account.get("login") != helpers.READER
            or reader_account.get("is_admin") is not False
            or repo_readback.get("private") is not True
            or not isinstance(permissions, dict)
            or permissions.get("push") is not False
            or permissions.get("pull") is not True
        ):
            raise RuntimeError("Forgejo did not confirm a private repo and read-only reader")
        stage = "verify_private_repository_anonymous_read_denial"
        anonymous_repository_status_code = anonymous_repository_status(OWNER, REPOSITORY)
        validate_private_repository_anonymous_status(
            is_private=repo_readback.get("private"),
            anonymous_status=anonymous_repository_status_code,
        )

        stage = "start_receiver_and_create_push_hook"
        webhook_secret = secrets.token_hex(32)
        secret_path = state / "webhook.secret"
        helpers.forgejo_helpers.write_private(secret_path, webhook_secret.encode("ascii"))
        receiver_port = helpers.select_receiver_port()
        receiver_address = f"127.0.0.1:{receiver_port}"
        expected_hook_url = f"http://{receiver_address}/forgejo/webhook"
        receiver = subprocess.Popen(
            [str(receiver_bin), receiver_address, str(secret_path), str(journal_path)],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=receiver_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        helpers.wait_for_listener(receiver, receiver_port)
        hook_response, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/hooks",
            {
                "type": "gitea",
                "config": {
                    "url": expected_hook_url,
                    "content_type": "json",
                    "secret": webhook_secret,
                },
                "events": ["push"],
                "active": True,
            },
        )
        hook_id = hook_response.get("id")
        if not isinstance(hook_id, int) or hook_id < 1 or hook_response.get("active") is not True:
            raise RuntimeError("Forgejo did not create the expected active push webhook")
        foreign_hook_response, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{FOREIGN_REPOSITORY}/hooks",
            {
                "type": "gitea",
                "config": {
                    "url": expected_hook_url,
                    "content_type": "json",
                    "secret": webhook_secret,
                },
                "events": ["push"],
                "active": True,
            },
        )
        foreign_hook_id = foreign_hook_response.get("id")
        if (
            not isinstance(foreign_hook_id, int)
            or foreign_hook_id < 1
            or foreign_hook_response.get("active") is not True
        ):
            raise RuntimeError("Forgejo did not create the expected foreign push webhook")

        stage = "push_new_branch_through_git_http"
        checkout = state / "git-checkout"
        askpass = state / "git-askpass"
        helpers.forgejo_helpers.write_private(
            askpass,
            b'#!/bin/sh\ncase "$1" in\n  *Username*) printf "%s\\n" "$FORGEJO_TEST_USER" ;;\n'
            b'  *Password*) printf "%s\\n" "$FORGEJO_TEST_TOKEN" ;;\n  *) exit 1 ;;\nesac\n',
        )
        os.chmod(askpass, 0o700)
        git_env = helpers.private_command_environment(state)
        git_env.update(
            {
                "GIT_ASKPASS": str(askpass),
                "GIT_TERMINAL_PROMPT": "0",
                "GIT_CONFIG_GLOBAL": "/dev/null",
                "GIT_CONFIG_NOSYSTEM": "1",
                "FORGEJO_TEST_USER": OWNER,
                "FORGEJO_TEST_TOKEN": owner_token,
            }
        )
        repository_url = f"{ORIGIN}/{OWNER}/{REPOSITORY}.git"
        run_git(["clone", "--quiet", "--single-branch", "--branch", "main", repository_url, str(checkout)], cwd=None, env=git_env, stage="clone")
        run_git(["config", "user.name", "Forgejo Native Push E2E"], cwd=checkout, env=git_env, stage="configure author")
        run_git(["config", "user.email", "forgejo-native-push@local.invalid"], cwd=checkout, env=git_env, stage="configure email")
        main_sha = run_git(["rev-parse", "HEAD"], cwd=checkout, env=git_env, stage="read main SHA")
        run_git(["checkout", "--quiet", "-b", BRANCH], cwd=checkout, env=git_env, stage="create branch")
        (checkout / "native-push-proof.txt").write_text(
            "A new branch committed through local Git HTTP.\n", encoding="utf-8"
        )
        run_git(["add", "native-push-proof.txt"], cwd=checkout, env=git_env, stage="stage change")
        run_git(["commit", "--quiet", "-m", "Add native push proof"], cwd=checkout, env=git_env, stage="commit change")
        first_push_sha = run_git(["rev-parse", "HEAD"], cwd=checkout, env=git_env, stage="read first push SHA")
        run_git(["push", "--quiet", "--set-upstream", "origin", BRANCH], cwd=checkout, env=git_env, stage="push new branch")

        stage = "read_emitted_push_and_forgejo_task"
        records = emitted.wait_for_journal_count(journal_path, 1)
        if len(records) != 1:
            raise RuntimeError("Forgejo emitted an unexpected number of events after branch creation")
        first_push_event = emitted.parse_live_push_delivery_record(
            records[0], owner=OWNER, repository=REPOSITORY
        )
        if (
            first_push_event["before"] != "0" * len(first_push_event["before"])
            or first_push_event["after"] != first_push_sha
            or first_push_event["branch"] != BRANCH
        ):
            raise RuntimeError("Forgejo's first push did not describe the new branch commit")
        first_tasks = emitted.wait_for_hook_tasks(state / "data/forgejo.db", hook_id, 1)
        if len(first_tasks) != 1:
            raise RuntimeError("Forgejo recorded an unexpected number of tasks after branch creation")
        emitted.validate_hook_task_bindings(first_tasks, [first_push_event])
        branch_path = quote(BRANCH, safe="")
        first_branch_readback, _ = reader_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{branch_path}"
        )
        first_branch_sha = first_branch_readback.get("commit", {}).get("id")
        if (
            first_branch_readback.get("name") != BRANCH
            or first_branch_sha != first_push_sha
            or first_push_sha == main_sha
        ):
            raise RuntimeError("Forgejo API did not confirm the newly created branch commit")

        stage = "push_update_to_existing_branch"
        (checkout / "native-push-proof.txt").write_text(
            "A later Git push advanced the branch after its creation event.\n", encoding="utf-8"
        )
        run_git(["add", "native-push-proof.txt"], cwd=checkout, env=git_env, stage="stage update")
        run_git(["commit", "--quiet", "-m", "Advance native push proof"], cwd=checkout, env=git_env, stage="commit update")
        second_push_sha = run_git(["rev-parse", "HEAD"], cwd=checkout, env=git_env, stage="read updated SHA")
        run_git(["push", "--quiet", "origin", BRANCH], cwd=checkout, env=git_env, stage="push branch update")

        stage = "read_emitted_update_and_bind_both_delivery_tasks"
        records = emitted.wait_for_journal_count(journal_path, 2)
        if len(records) != 2:
            raise RuntimeError("Forgejo emitted an unexpected number of events after branch update")
        push_events = [
            emitted.parse_live_push_delivery_record(record, owner=OWNER, repository=REPOSITORY)
            for record in records
        ]
        first_push_event, second_push_event = push_events
        if (
            first_push_event["delivery_id"] == second_push_event["delivery_id"]
            or first_push_event["sequence"] >= second_push_event["sequence"]
            or second_push_event["branch"] != BRANCH
            or second_push_event["before"] != first_push_sha
            or second_push_event["after"] != second_push_sha
        ):
            raise RuntimeError("Forgejo's update event did not follow the existing branch head")
        hook_tasks = emitted.wait_for_hook_tasks(state / "data/forgejo.db", hook_id, 2)
        if len(hook_tasks) != 2:
            raise RuntimeError("Forgejo recorded an unexpected number of push webhook tasks")
        emitted.validate_hook_task_bindings(hook_tasks, push_events)
        branch_readback, _ = reader_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{branch_path}"
        )
        branch_sha = branch_readback.get("commit", {}).get("id")
        if branch_readback.get("name") != BRANCH or branch_sha != second_push_sha:
            raise RuntimeError("Forgejo API did not confirm the updated branch head")

        stage = "batch_reconcile_old_and_current_push_deliveries_with_one_read"
        journal_before = journal_path.read_bytes()
        batch_status, batch_reconciliation, batch_remaining_credential = run_reconcile_batch(
            reconcile_bin,
            state,
            journal_path,
            [event["delivery_id"] for event in push_events],
            reader_token,
        )
        if batch_remaining_credential:
            raise RuntimeError("successful push reconciliation did not consume its credential")
        checked_results = validate_push_reconcile_batch(
            batch_status,
            push_events,
            batch_reconciliation,
            owner=OWNER,
            repository=REPOSITORY,
            branch=BRANCH,
            current_head_sha=branch_sha,
        )
        if (
            push_events[0]["after"] == branch_sha
            or push_events[1]["after"] != branch_sha
            or push_events[0]["after"] == push_events[1]["after"]
        ):
            raise RuntimeError("push batch does not contain one stale and one current notification")
        reconciliations = [
            {
                "notification_delivery_id": event["delivery_id"],
                "notification_after_sha": event["after"],
                "canonical_head_sha": result["canonical_state"]["head_sha"],
            }
            for event, result in zip(push_events, checked_results, strict=True)
        ]
        journal_after_batch = journal_path.read_bytes()
        if journal_before != journal_after_batch:
            raise RuntimeError("reconciliation modified the append-only webhook journal")

        stage = "reject_partial_push_batch_before_credential_or_upstream_read"
        missing_delivery_id = "forgejo-push-delivery-absent-004"
        partial_delivery_ids = [push_events[0]["delivery_id"], missing_delivery_id]
        refusal_status, refusal_result, refusal_remaining_credential = run_reconcile_batch(
            reconcile_bin,
            state,
            journal_path,
            partial_delivery_ids,
            reader_token,
        )
        credential_preserved = refusal_remaining_credential == reader_token.encode("utf-8")
        validate_partial_push_batch_refusal(
            refusal_status,
            partial_delivery_ids,
            refusal_result,
            missing_delivery_id=missing_delivery_id,
            credential_preserved=credential_preserved,
        )
        journal_after_refusal = journal_path.read_bytes()
        if journal_before != journal_after_refusal:
            raise RuntimeError("partial batch refusal modified the append-only webhook journal")

        stage = "emit_pushes_from_a_second_private_repository"
        foreign_checkout = state / "foreign-git-checkout"
        foreign_repository_url = f"{ORIGIN}/{OWNER}/{FOREIGN_REPOSITORY}.git"
        run_git(
            ["clone", "--quiet", "--single-branch", "--branch", "main", foreign_repository_url, str(foreign_checkout)],
            cwd=None,
            env=git_env,
            stage="clone foreign repo",
        )
        run_git(["config", "user.name", "Forgejo Out Of Scope E2E"], cwd=foreign_checkout, env=git_env, stage="configure foreign author")
        run_git(["config", "user.email", "forgejo-out-of-scope@local.invalid"], cwd=foreign_checkout, env=git_env, stage="configure foreign email")
        run_git(["checkout", "--quiet", "-b", FOREIGN_BRANCH], cwd=foreign_checkout, env=git_env, stage="create foreign branch")
        foreign_proof = foreign_checkout / "out-of-scope-proof.txt"
        foreign_proof.write_text("First foreign repository push.\n", encoding="utf-8")
        run_git(["add", "out-of-scope-proof.txt"], cwd=foreign_checkout, env=git_env, stage="stage foreign creation")
        run_git(["commit", "--quiet", "-m", "Create foreign proof branch"], cwd=foreign_checkout, env=git_env, stage="commit foreign creation")
        foreign_first_sha = run_git(["rev-parse", "HEAD"], cwd=foreign_checkout, env=git_env, stage="read foreign creation SHA")
        run_git(["push", "--quiet", "--set-upstream", "origin", FOREIGN_BRANCH], cwd=foreign_checkout, env=git_env, stage="push foreign branch creation")
        foreign_proof.write_text("Second foreign repository push.\n", encoding="utf-8")
        run_git(["add", "out-of-scope-proof.txt"], cwd=foreign_checkout, env=git_env, stage="stage foreign update")
        run_git(["commit", "--quiet", "-m", "Advance foreign proof branch"], cwd=foreign_checkout, env=git_env, stage="commit foreign update")
        foreign_second_sha = run_git(["rev-parse", "HEAD"], cwd=foreign_checkout, env=git_env, stage="read foreign update SHA")
        run_git(["push", "--quiet", "origin", FOREIGN_BRANCH], cwd=foreign_checkout, env=git_env, stage="push foreign branch update")

        stage = "bind_foreign_push_deliveries_to_the_foreign_hook"
        records = emitted.wait_for_journal_count(journal_path, 4)
        if len(records) != 4:
            raise RuntimeError("Forgejo emitted an unexpected number of out-of-scope push events")
        foreign_push_events = [
            emitted.parse_live_push_delivery_record(
                record, owner=OWNER, repository=FOREIGN_REPOSITORY
            )
            for record in records[2:]
        ]
        foreign_first_event, foreign_second_event = foreign_push_events
        if (
            foreign_first_event["delivery_id"] == foreign_second_event["delivery_id"]
            or foreign_first_event["sequence"] >= foreign_second_event["sequence"]
            or foreign_first_event["before"] != "0" * len(foreign_first_event["before"])
            or foreign_first_event["after"] != foreign_first_sha
            or foreign_second_event["before"] != foreign_first_sha
            or foreign_second_event["after"] != foreign_second_sha
            or foreign_second_event["branch"] != FOREIGN_BRANCH
        ):
            raise RuntimeError("foreign push events do not describe the expected branch updates")
        foreign_hook_tasks = emitted.wait_for_hook_tasks(
            state / "data/forgejo.db", foreign_hook_id, 2
        )
        if len(foreign_hook_tasks) != 2:
            raise RuntimeError("Forgejo did not record both foreign push webhook tasks")
        emitted.validate_hook_task_bindings(foreign_hook_tasks, foreign_push_events)
        foreign_branch_path = quote(FOREIGN_BRANCH, safe="")
        foreign_branch_readback, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{FOREIGN_REPOSITORY}/branches/{foreign_branch_path}"
        )
        if (
            foreign_branch_readback.get("name") != FOREIGN_BRANCH
            or foreign_branch_readback.get("commit", {}).get("id") != foreign_second_sha
        ):
            raise RuntimeError("Forgejo API did not confirm the foreign branch's current head")

        stage = "refuse_foreign_push_batch_before_credential_or_upstream_read"
        journal_before_out_of_scope = journal_path.read_bytes()
        out_of_scope_status, out_of_scope_result, out_of_scope_remaining_credential = run_reconcile_batch(
            reconcile_bin,
            state,
            journal_path,
            [event["delivery_id"] for event in foreign_push_events],
            reader_token,
        )
        out_of_scope_credential_preserved = (
            out_of_scope_remaining_credential == reader_token.encode("utf-8")
        )
        validate_out_of_scope_push_batch_refusal(
            out_of_scope_status,
            foreign_push_events,
            out_of_scope_result,
            configured_repository=f"{OWNER}/{REPOSITORY}",
            event_repository=f"{OWNER}/{FOREIGN_REPOSITORY}",
            credential_preserved=out_of_scope_credential_preserved,
        )
        journal_after_out_of_scope = journal_path.read_bytes()
        if journal_before_out_of_scope != journal_after_out_of_scope:
            raise RuntimeError("out-of-scope push refusal modified the append-only webhook journal")

        stage = "deactivate_hook_and_read_back"
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
            emitted.hook_deactivation_patch(("push",)),
        )
        hook_api_readback, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
        )
        hook_readback = emitted.hook_readback_record(
            hook_id, hook_api_readback, expected_hook_url, ("push",)
        )
        hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
        if not hook_disabled or not hook_readback["configuration_matches_expected"]:
            raise RuntimeError("Forgejo hook deactivation/configuration readback failed")
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{FOREIGN_REPOSITORY}/hooks/{foreign_hook_id}",
            emitted.hook_deactivation_patch(("push",)),
        )
        foreign_hook_api_readback, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{FOREIGN_REPOSITORY}/hooks/{foreign_hook_id}"
        )
        foreign_hook_readback = emitted.hook_readback_record(
            foreign_hook_id, foreign_hook_api_readback, expected_hook_url, ("push",)
        )
        foreign_hook_disabled = (
            foreign_hook_readback["deactivation_readback_confirmed"] is True
            and foreign_hook_readback["configuration_matches_expected"] is True
        )
        if not foreign_hook_disabled:
            raise RuntimeError("Forgejo foreign hook deactivation/configuration readback failed")

        safe_result = {
            "schema_version": 1,
            "verdict": "passed",
            "forgejo": {
                "version": version.stdout.strip(),
                "binary_sha256": helpers.sha256_file(forgejo),
                "listener": "127.0.0.1:3130",
                "webhook_allowlist": "127.0.0.1",
            },
            "repository": {
                "full_name": f"{OWNER}/{REPOSITORY}",
                "private": True,
                "reader_is_non_admin": reader_account.get("is_admin") is False,
                "reader_permission": permissions,
            },
            "private_repository_read_boundary": {
                "anonymous_api_status": anonymous_repository_status_code,
                "authenticated_reconciliation_verdict": batch_reconciliation.get("verdict"),
                "authenticated_reconciliation_results": len(checked_results),
                "credential_source": "inherited_private_pipe_fd",
                "credential_recorded": False,
            },
            "foreign_repository": {
                "full_name": f"{OWNER}/{FOREIGN_REPOSITORY}",
                "private": foreign_repo_readback.get("private") is True,
                "branch": FOREIGN_BRANCH,
                "latest_commit_sha": foreign_second_sha,
                "branch_readback_sha": foreign_branch_readback.get("commit", {}).get("id"),
                "push_events": foreign_push_events,
                "forgejo_delivery_tasks": foreign_hook_tasks,
            },
            "git_push": {
                "branch": BRANCH,
                "base_sha": main_sha,
                "first_commit_sha": first_push_sha,
                "latest_commit_sha": second_push_sha,
                "events": push_events,
                "forgejo_delivery_tasks": hook_tasks,
                "initial_api_branch_readback": {
                    "name": first_branch_readback.get("name"),
                    "head_sha": first_branch_sha,
                },
                "api_branch_readback": {
                    "name": branch_readback.get("name"),
                    "head_sha": branch_sha,
                },
            },
            "hook": {**hook_readback, "secret_recorded": False},
            "foreign_hook": {**foreign_hook_readback, "secret_recorded": False},
            "reconciliation": {
                "batch_exit_status": batch_status,
                "batch_result": batch_reconciliation,
                "credential_consumed": True,
                "deliveries": reconciliations,
                "journal_unchanged": journal_before == journal_after_batch,
            },
            "partial_selection_refusal": {
                "exit_status": refusal_status,
                "requested_delivery_ids": partial_delivery_ids,
                "missing_delivery_id": missing_delivery_id,
                "result": refusal_result,
                "credential_preserved": credential_preserved,
                "journal_unchanged": journal_before == journal_after_refusal,
            },
            "out_of_scope_push_batch_refusal": {
                "exit_status": out_of_scope_status,
                "requested_delivery_ids": [event["delivery_id"] for event in foreign_push_events],
                "configured_repository": f"{OWNER}/{REPOSITORY}",
                "event_repository": f"{OWNER}/{FOREIGN_REPOSITORY}",
                "result": out_of_scope_result,
                "credential_preserved": out_of_scope_credential_preserved,
                "journal_unchanged": journal_before_out_of_scope == journal_after_out_of_scope,
            },
            "source_head_commit": subprocess.run(
                ["git", "-C", str(repo), "rev-parse", "HEAD"],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                timeout=10,
                check=True,
            ).stdout.strip(),
            "limitations": [
                "Forgejo v17.0.0, receiver, Git push and API all ran locally; only the outbound webhook connected to a loopback receiver.",
                "The pushed file is a test change; this slice does not exercise an LLM, AgentOS, PR review, or merge authority.",
                "No comparative productivity, human-intervention, token, or cost result is measured.",
            ],
        }
        stage = "complete"
    except Exception as error:
        failure = type(error).__name__
    finally:
        if hook_id is not None and owner_api is not None and not hook_disabled:
            try:
                owner_api.request(
                    "PATCH",
                    f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                    emitted.hook_deactivation_patch(("push",)),
                )
                hook_api_readback, _ = owner_api.request(
                    "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
                )
                hook_readback = emitted.hook_readback_record(
                    hook_id, hook_api_readback, expected_hook_url, ("push",)
                )
                hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
                if not hook_disabled:
                    cleanup_failure = "hook_deactivation_not_confirmed"
            except Exception as error:
                cleanup_failure = type(error).__name__
        if foreign_hook_id is not None and owner_api is not None and not foreign_hook_disabled:
            try:
                owner_api.request(
                    "PATCH",
                    f"/repos/{OWNER}/{FOREIGN_REPOSITORY}/hooks/{foreign_hook_id}",
                    emitted.hook_deactivation_patch(("push",)),
                )
                foreign_hook_api_readback, _ = owner_api.request(
                    "GET", f"/repos/{OWNER}/{FOREIGN_REPOSITORY}/hooks/{foreign_hook_id}"
                )
                foreign_hook_readback = emitted.hook_readback_record(
                    foreign_hook_id, foreign_hook_api_readback, expected_hook_url, ("push",)
                )
                foreign_hook_disabled = (
                    foreign_hook_readback["deactivation_readback_confirmed"] is True
                )
                if not foreign_hook_disabled:
                    cleanup_failure = cleanup_failure or "foreign_hook_deactivation_not_confirmed"
            except Exception as error:
                cleanup_failure = cleanup_failure or type(error).__name__
        receiver_stop = helpers.stop_process(receiver)
        server_stop = helpers.stop_process(server)
        receiver_log.close()
        server_log.close()

    cleanup = {
        "forgejo": {
            "stop": server_stop,
            "listener_addresses_after_stop": helpers.listener_addresses_for_port(3130),
        },
        "receiver": {
            "stop": receiver_stop,
            "listener_addresses_after_stop": (
                helpers.listener_addresses_for_port(receiver_port) if receiver_port else []
            ),
        },
        "hook_disabled": hook_disabled,
        "foreign_hook_disabled": foreign_hook_disabled,
        "foreign_hook_readback": foreign_hook_readback,
        "cleanup_failure": cleanup_failure,
        "private_state_dir": str(state),
    }
    if safe_result is not None:
        safe_result["cleanup"] = cleanup
        source_files = sorted(
            {
                repo / "Cargo.toml",
                repo / "Cargo.lock",
                Path(__file__).resolve(),
                (SCRIPT_DIR / "run_forgejo_live_webhook_reconcile_e2e.py").resolve(),
                (SCRIPT_DIR / "run_forgejo_live_webhook_reconcile_emitted.py").resolve(),
                (repo / "tests/test_forgejo_emitted_webhook_e2e.py").resolve(),
                *(repo / "src").rglob("*.rs"),
            }
        )
        safe_result["provenance"] = {
            "source_sha256": {
                str(path.relative_to(repo)): helpers.sha256_file(path)
                for path in source_files
            },
            "forgejo_binary_sha256": helpers.sha256_file(forgejo),
            "receiver_sha256": helpers.sha256_file(receiver_bin),
            "reconcile_cli_sha256": helpers.sha256_file(reconcile_bin),
            "cargo_build": build_provenance,
            "webhook_secret_recorded": False,
            "credentials_recorded": False,
        }
        helpers.write_safe_json(evidence_dir / "push-events.json", push_events, tokens)
        helpers.write_safe_json(evidence_dir / "forgejo-hook-tasks.json", hook_tasks, tokens)
        helpers.write_safe_json(evidence_dir / "foreign-push-events.json", foreign_push_events, tokens)
        helpers.write_safe_json(evidence_dir / "foreign-hook-tasks.json", foreign_hook_tasks, tokens)
        helpers.write_safe_json(evidence_dir / "branch-readback.json", branch_readback, tokens)
        helpers.write_safe_json(
            evidence_dir / "reconciliation.json",
            safe_result.get("reconciliation"),
            tokens,
        )
        helpers.write_safe_json(
            evidence_dir / "out-of-scope-refusal.json",
            safe_result.get("out_of_scope_push_batch_refusal"),
            tokens,
        )
    else:
        safe_result = {
            "schema_version": 1,
            "verdict": "failed",
            "stage": stage,
            "failure_type": failure,
            "cleanup": cleanup,
        }
    helpers.write_safe_json(evidence_dir / "result.json", safe_result, tokens)
    helpers.write_checksums(evidence_dir)
    if failure is not None:
        raise RuntimeError(f"native push E2E failed at {stage} ({failure})")
    return safe_result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(SCRIPT_DIR.parent))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    result = run(parser.parse_args())
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
