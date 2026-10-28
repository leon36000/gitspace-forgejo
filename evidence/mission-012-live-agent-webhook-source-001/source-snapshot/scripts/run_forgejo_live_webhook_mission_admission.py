#!/usr/bin/env python3
"""Admit a mission from an issue event emitted by a disposable Forgejo instance."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
from urllib.parse import urljoin, urlsplit
import secrets
import shutil
import socket
import subprocess
import sys
import time
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_emitted as emitted  # noqa: E402
import run_forgejo_live_webhook_reconcile_e2e as webhook_helpers  # noqa: E402
import run_mission_012_live_admission_e2e as admission_helpers  # noqa: E402
import run_forgejo_live_webhook_native_replay_e2e as native_replay  # noqa: E402

OWNER = admission_helpers.OWNER
REPOSITORY = admission_helpers.REPOSITORY
REVIEWER = "verifier-bot"
ORIGIN = admission_helpers.ORIGIN
REQUIREMENT_ID = "MISSION-012-CLAMP-001"
FORGEJO_SHA256 = admission_helpers.FORGEJO_SHA256
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
READ_SCOPES = "read:repository,read:issue"


def safe_failure_detail(
    error: BaseException, tokens: list[str], webhook_secret: bytes | None
) -> str:
    """Keep actionable exception text while removing credentials and hook-key forms."""
    detail = str(error)
    private_values = list(tokens)
    if webhook_secret:
        private_values.extend(
            [
                webhook_secret.hex(),
                base64.b64encode(webhook_secret).decode("ascii"),
                webhook_secret.decode("utf-8", "ignore"),
            ]
        )
    for private_value in private_values:
        if private_value:
            detail = detail.replace(private_value, "[REDACTED]")
    return detail[:500]


def receiver_target(address: tuple[str, int]) -> tuple[int, str]:
    """Validate the receiver bind address and return its port and Forgejo hook URL."""
    if not isinstance(address, tuple) or len(address) != 2:
        raise ValueError("receiver address must be an address/port tuple")
    host, port = address
    if (
        host != "127.0.0.1"
        or not isinstance(port, int)
        or isinstance(port, bool)
        or not 1 <= port <= 65535
    ):
        raise ValueError("receiver must bind to a valid loopback address")
    return port, f"http://{host}:{port}/forgejo/webhook"


def mission_branch_protection_policy() -> dict[str, Any]:
    return {
        "rule_name": "main",
        "branch_name": "main",
        "enable_push": False,
        "required_approvals": 1,
        "dismiss_stale_approvals": True,
        "block_on_outdated_branch": True,
        "apply_to_admins": True,
        "enable_approvals_whitelist": True,
        "approvals_whitelist_username": [REVIEWER],
        "enable_merge_whitelist": True,
        "merge_whitelist_usernames": [OWNER],
        "merge_whitelist_teams": [],
        "enable_status_check": False,
    }


def branch_protection_matches_policy(protection: object) -> bool:
    if not isinstance(protection, dict):
        return False
    return all(
        protection.get(key) == value
        for key, value in mission_branch_protection_policy().items()
    )


def generate_webhook_secret() -> bytes:
    """Return the exact ASCII secret bytes that Forgejo and the receiver share."""
    return secrets.token_hex(32).encode("ascii")


def forgejo_secret_value(secret: bytes) -> str:
    """Validate receiver bytes before placing the identical UTF-8 text in Forgejo config."""
    if len(secret) != 64 or any(byte not in b"0123456789abcdef" for byte in secret):
        raise ValueError("webhook secret must be 64 lowercase hexadecimal ASCII bytes")
    return secret.decode("ascii")


def validate_single_hook_task_binding(
    tasks: list[dict[str, Any]], event: dict[str, Any]
) -> dict[str, Any]:
    """Require a successful Forgejo HTTP 202 task for the exact journal delivery."""
    if len(tasks) != 1 or not isinstance(event, dict):
        raise ValueError("expected exactly one Forgejo issue task and receiver event")
    task = tasks[0]
    sequence = event.get("sequence")
    delivery_id = event.get("delivery_id")
    if (
        not isinstance(task, dict)
        or task.get("event_type") != "issues"
        or task.get("is_delivered") is not True
        or task.get("is_succeed") is not True
        or task.get("response_status") != 202
        or not isinstance(task.get("uuid"), str)
        or task.get("uuid") != delivery_id
        or not isinstance(sequence, int)
        or isinstance(sequence, bool)
        or sequence < 1
        or event.get("event") != "issues"
        or event.get("action") != "opened"
        or event.get("payload_state") != "open"
    ):
        raise ValueError("Forgejo task does not acknowledge the expected open issue delivery")
    return {
        "forgejo_delivery_uuid": task["uuid"],
        "receiver_delivery_id": delivery_id,
        "receiver_sequence": sequence,
        "response_status": task["response_status"],
    }


def parse_received_issue_event(
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    issue_number: int,
    expected_action: str,
) -> dict[str, Any]:
    """Parse a journal event and hash the complete received Forgejo payload."""
    event = emitted.parse_live_delivery_record(
        record,
        owner=owner,
        repository=repository,
        issue_number=issue_number,
        expected_action=expected_action,
    )
    envelope = record.get("payload") if isinstance(record, dict) else None
    payload = envelope.get("payload") if isinstance(envelope, dict) else None
    if not isinstance(payload, dict):
        raise ValueError("received Forgejo issue payload is not a structured object")
    event["payload_semantic_sha256"] = native_replay.canonical_json_sha256(payload)
    return event


def validate_replayed_issue_delivery_binding(
    original_task: dict[str, Any],
    replay_task: dict[str, Any],
    original_event: dict[str, Any],
    replay_event: dict[str, Any],
) -> dict[str, Any]:
    """Bind Forgejo's new-UUID native replay to the same acknowledged issue event."""
    if not all(
        isinstance(value, dict)
        for value in (original_task, replay_task, original_event, replay_event)
    ):
        raise ValueError("native issue replay evidence must be structured objects")
    tasks = (original_task, replay_task)
    events = (original_event, replay_event)
    delivery_ids: list[str] = []
    issue_number: int | None = None
    for expected_sequence, (task, event) in enumerate(zip(tasks, events, strict=True), start=1):
        delivery_id = task.get("uuid")
        sequence = event.get("sequence")
        number = event.get("issue_number")
        if (
            not isinstance(delivery_id, str)
            or not delivery_id
            or event.get("delivery_id") != delivery_id
            or task.get("event_type") != "issues"
            or task.get("is_delivered") is not True
            or task.get("is_succeed") is not True
            or type(task.get("response_status")) is not int
            or task.get("response_status") != 202
            or type(sequence) is not int
            or sequence != expected_sequence
            or event.get("event") != "issues"
            or event.get("action") != "opened"
            or event.get("payload_state") != "open"
            or not isinstance(event.get("payload_semantic_sha256"), str)
            or native_replay.SHA256_RE.fullmatch(event["payload_semantic_sha256"]) is None
            or type(number) is not int
            or number < 1
            or (issue_number is not None and number != issue_number)
        ):
            raise ValueError("native issue replay task or receiver event is not bound to the original issue")
        issue_number = number
        delivery_ids.append(delivery_id)

    if delivery_ids[0] == delivery_ids[1]:
        raise ValueError("native Forgejo replay must create a distinct delivery UUID")
    original_raw_hash = original_task.get("payload_sha256")
    replay_raw_hash = replay_task.get("payload_sha256")
    original_semantic_hash = original_task.get("payload_semantic_sha256")
    replay_semantic_hash = replay_task.get("payload_semantic_sha256")
    if (
        not isinstance(original_raw_hash, str)
        or native_replay.SHA256_RE.fullmatch(original_raw_hash) is None
        or replay_raw_hash != original_raw_hash
        or not isinstance(original_semantic_hash, str)
        or native_replay.SHA256_RE.fullmatch(original_semantic_hash) is None
        or replay_semantic_hash != original_semantic_hash
        or original_event.get("payload_semantic_sha256") != original_semantic_hash
        or replay_event.get("payload_semantic_sha256") != replay_semantic_hash
    ):
        raise ValueError("native issue replay payload differs from the original Forgejo task")
    return {
        "original_delivery_id": delivery_ids[0],
        "replay_delivery_id": delivery_ids[1],
        "issue_number": issue_number,
        "receiver_sequences": [1, 2],
        "payload_sha256": original_raw_hash,
        "payload_semantic_sha256": original_semantic_hash,
        "received_payload_semantic_sha256": original_event["payload_semantic_sha256"],
        "payload_unchanged": True,
    }


def validate_admission_result(
    result: dict[str, Any],
    *,
    event: dict[str, Any],
    requirement_sha256: str,
    base_commit: str,
    allocation_id: str,
) -> None:
    """Bind the CLI's versioned host admission to the emitted delivery and base."""
    admission = result.get("host_admission") if isinstance(result, dict) else None
    webhook = result.get("webhook") if isinstance(result, dict) else None
    scope = admission.get("scope") if isinstance(admission, dict) else None
    allocation = admission.get("allocation") if isinstance(admission, dict) else None
    if (
        result.get("schema_version") != 2
        or result.get("verdict") != "claimed"
        or result.get("admission_mode") != "webhook_delivery"
        or result.get("profile_id") != "mission-012-clamp"
        or result.get("agent_runtime_invoked") is not False
        or result.get("sandbox_only") is not True
        or not isinstance(webhook, dict)
        or webhook.get("delivery_id") != event.get("delivery_id")
        or webhook.get("reconciled_state") != "open"
        or webhook.get("requirement_sha256_matches_cli_argument") is not True
        or not isinstance(scope, dict)
        or scope
        != {
            "origin": ORIGIN,
            "owner": OWNER,
            "repository": REPOSITORY,
            "branch": "main",
        }
        or admission.get("issue_number") != event.get("issue_number")
        or admission.get("base_commit") != base_commit
        or admission.get("requirement_path") != f"requirements/{REQUIREMENT_ID}.json"
        or admission.get("requirement_sha256") != requirement_sha256
        or not isinstance(allocation, dict)
        or allocation.get("allocation_id") != allocation_id
        or allocation.get("execution_generation") != 1
    ):
        raise ValueError("mission admission does not match the emitted event and pinned inputs")


def validate_native_replay_admission_result(
    original: dict[str, Any],
    replay: dict[str, Any],
    *,
    expected_delivery_id: str,
    mission_journal_before: bytes,
    mission_journal_after: bytes,
    admission_file_before: bytes,
    admission_file_after: bytes,
) -> dict[str, Any]:
    """Require a new webhook UUID to reuse, not append, the same mission claim."""
    if not isinstance(original, dict) or not isinstance(replay, dict):
        raise ValueError("native replay admission results must be structured objects")
    original_handoff = original.get("host_admission")
    replay_handoff = replay.get("host_admission")
    original_webhook = original.get("webhook")
    replay_webhook = replay.get("webhook")
    if not all(
        isinstance(value, dict)
        for value in (original_handoff, replay_handoff, original_webhook, replay_webhook)
    ):
        raise ValueError("native replay admission omitted its handoff or webhook binding")
    allocation = original_handoff.get("allocation")
    request = original_handoff.get("request")
    if not isinstance(allocation, dict) or not isinstance(request, dict):
        raise ValueError("native replay admission handoff is incomplete")
    allocation_id = allocation.get("allocation_id")
    generation = allocation.get("execution_generation")
    mission_id = request.get("mission_id")
    idempotency_key = request.get("idempotency_key")
    original_delivery_id = original_webhook.get("delivery_id")
    if (
        original.get("verdict") != "claimed"
        or original.get("allocation_replayed") is not False
        or original.get("admission_mode") != "webhook_delivery"
        or replay.get("verdict") != "already_claimed"
        or replay.get("allocation_replayed") is not True
        or replay.get("admission_mode") != "webhook_delivery"
        or replay_handoff != original_handoff
        or original_webhook.get("reconciled_state") != "open"
        or replay_webhook.get("reconciled_state") != "open"
        or not isinstance(original_delivery_id, str)
        or not original_delivery_id
        or not isinstance(expected_delivery_id, str)
        or not expected_delivery_id
        or original_delivery_id == expected_delivery_id
        or replay_webhook.get("delivery_id") != expected_delivery_id
        or not isinstance(allocation_id, str)
        or not allocation_id
        or type(generation) is not int
        or generation < 1
        or not isinstance(mission_id, str)
        or not mission_id
        or not isinstance(idempotency_key, str)
        or not idempotency_key
        or not isinstance(mission_journal_before, bytes)
        or not mission_journal_before
        or mission_journal_after != mission_journal_before
        or not isinstance(admission_file_before, bytes)
        or not admission_file_before
        or admission_file_after != admission_file_before
    ):
        raise ValueError("native replay changed the mission allocation, claim, or persisted handoff")
    return {
        "delivery_id": expected_delivery_id,
        "mission_id": mission_id,
        "allocation_id": allocation_id,
        "execution_generation": generation,
        "verdict": "already_claimed",
        "mission_journal_unchanged": True,
        "mission_journal_before_sha256": hashlib.sha256(mission_journal_before).hexdigest(),
        "mission_journal_after_sha256": hashlib.sha256(mission_journal_after).hexdigest(),
        "admission_file_unchanged": True,
        "admission_file_before_sha256": hashlib.sha256(admission_file_before).hexdigest(),
        "admission_file_after_sha256": hashlib.sha256(admission_file_after).hexdigest(),
    }


def mission_hook_readback_record(
    hook_id: int | None,
    api_readback: dict[str, Any] | None,
    expected_target: str | None,
) -> dict[str, Any]:
    """Serialize and validate the issue-only hook used to admit a mission."""
    return emitted.hook_readback_record(
        hook_id,
        api_readback,
        expected_target,
        expected_event_selectors=("issues",),
    )


def fixture_files(requirement: bytes) -> dict[str, bytes]:
    """Return the small intentionally broken MISSION-012 project stored in Forgejo."""
    return {
        "Cargo.toml": b'[package]\nname = "mission012-clamp"\nversion = "0.1.0"\nedition = "2024"\n',
        "Cargo.lock": (
            b"# This file is automatically @generated by Cargo.\nversion = 4\n\n"
            b'[[package]]\nname = "mission012-clamp"\nversion = "0.1.0"\n'
        ),
        "src/lib.rs": (
            b"pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n"
            b"    if value < minimum {\n        minimum\n"
            b"    } else if value > maximum {\n"
            b"        minimum // BUG: values above the range should clamp to maximum.\n"
            b"    } else {\n        value\n    }\n}\n"
        ),
        "tests/acceptance.rs": (
            b"use mission012_clamp::clamp_to_range;\n\n"
            b"#[test]\nfn values_outside_the_range_clamp_to_the_nearest_boundary() {\n"
            b"    assert_eq!(clamp_to_range(-5, -2, 10), -2);\n"
            b"    assert_eq!(clamp_to_range(15, -2, 10), 10);\n}\n\n"
            b"#[test]\nfn values_inside_the_range_are_unchanged() {\n"
            b"    assert_eq!(clamp_to_range(4, -2, 10), 4);\n}\n"
        ),
        f"requirements/{REQUIREMENT_ID}.json": requirement,
    }


def _expect_api(
    api: Any, method: str, path: str, expected: set[int], value: object | None = None
) -> Any:
    status, body, _ = api(method, path, value)
    if status not in expected:
        raise RuntimeError(f"Forgejo API {method} {path} returned HTTP {status}")
    return body


def _run_mission_cli(
    binary: Path,
    project: Path,
    state: Path,
    arguments: list[str],
    credential: bytes,
) -> tuple[int, dict[str, Any]]:
    completed = subprocess.run(
        [str(binary), *arguments],
        cwd=project,
        env={"PATH": "/usr/bin:/bin", "HOME": str(state / "home"), "TMPDIR": str(state / "tmp")},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=30,
        check=False,
    )
    if credential in completed.stdout:
        raise RuntimeError("mission CLI exposed its private Forgejo credential")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError:
        raise RuntimeError("mission CLI did not return structured JSON") from None
    if not isinstance(value, dict):
        raise RuntimeError("mission CLI returned a non-object result")
    return completed.returncode, value


def _wait_for_issue_task(database: Path, hook_id: int, timeout: float = 10.0) -> list[dict[str, Any]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        tasks = emitted.hook_task_readback(database, hook_id)
        issue_tasks = [task for task in tasks if task.get("event_type") == "issues"]
        if len(issue_tasks) > 1:
            raise RuntimeError("Forgejo recorded duplicate issue webhook tasks for one issue")
        if len(issue_tasks) == 1:
            task = issue_tasks[0]
            status = task.get("response_status")
            has_http_response = isinstance(status, int) and not isinstance(status, bool)
            acknowledged = (
                task.get("is_delivered") is True
                and task.get("is_succeed") is True
                and status == 202
            )
            rejected = (
                task.get("is_delivered") is True
                and has_http_response
                and status != 202
            )
            if acknowledged or rejected:
                return issue_tasks
        time.sleep(0.1)
    raise RuntimeError("Forgejo did not finish the issue webhook HTTP response")


def _source_provenance(repo: Path) -> dict[str, Any]:
    source_files = {
        repo / "Cargo.toml",
        repo / "Cargo.lock",
        Path(__file__).resolve(),
        Path(emitted.__file__).resolve(),
        Path(webhook_helpers.__file__).resolve(),
        Path(admission_helpers.__file__).resolve(),
        Path(native_replay.__file__).resolve(),
        repo / "tests/test_forgejo_live_webhook_mission_admission.py",
        repo / "tests/test_forgejo_webhook_native_replay_e2e.py",
        repo / f"tests/fixtures/mission-012/requirements/{REQUIREMENT_ID}.json",
        *((repo / "src").rglob("*.rs")),
        *((repo / "tests").rglob("*.rs")),
    }
    files = sorted((path for path in source_files if path.is_file()), key=lambda path: str(path))
    git_paths = [
        "Cargo.toml",
        "Cargo.lock",
        "src",
        "tests",
        "scripts/run_forgejo_live_webhook_mission_admission.py",
        "scripts/run_forgejo_live_webhook_reconcile_emitted.py",
        "scripts/run_forgejo_live_webhook_reconcile_e2e.py",
        "scripts/run_mission_012_live_admission_e2e.py",
        "scripts/run_forgejo_live_webhook_native_replay_e2e.py",
    ]
    status = subprocess.run(
        ["git", "-C", str(repo), "status", "--porcelain", "--", *git_paths],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        timeout=10,
        check=True,
    )
    head = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        timeout=10,
        check=True,
    ).stdout.strip()
    return {
        "head_commit": head,
        "source_worktree_status": status.stdout.splitlines(),
        "source_sha256": {
            str(path.relative_to(repo)): webhook_helpers.sha256_file(path) for path in files
        },
    }


def built_executable_hashes(
    state: Path,
    mission_binary: Path,
    receiver_binary: Path,
    *,
    include_mcp_host: bool,
) -> dict[str, str]:
    executables = {
        "forgejo-mission-local": mission_binary,
        "forgejo-webhook-local": receiver_binary,
    }
    if include_mcp_host:
        executables["mission-012-mcp-host"] = (
            state / "build/host-artifacts/mission-012-mcp-host"
        )
    hashes: dict[str, str] = {}
    for name, path in executables.items():
        if path.is_symlink() or not path.is_file():
            raise RuntimeError(f"isolated build omitted the expected executable: {name}")
        hashes[name] = webhook_helpers.sha256_file(path)
    return hashes


def run(args: argparse.Namespace) -> dict[str, Any]:
    binary_arg = Path(args.forgejo_binary)
    if binary_arg.is_symlink() or not binary_arg.is_file():
        raise RuntimeError("Forgejo binary must be a regular non-symlink file")
    forgejo = binary_arg.resolve(strict=True)
    repo = Path(args.repo_root).resolve(strict=True)
    if repo != SCRIPT_DIR.parent.resolve(strict=True):
        raise RuntimeError("live mission build helper must target this runner's repository")
    state = Path(args.state_dir)
    evidence_dir = Path(args.evidence_dir).resolve()
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("state dir must be a new direct child of /tmp")
    if evidence_dir.exists():
        raise RuntimeError("refusing to overwrite existing evidence directory")
    if not (repo / "Cargo.toml").is_file():
        raise RuntimeError("a Cargo repository is required")
    if shutil.which("bwrap") is None or shutil.which("rustc") is None:
        raise RuntimeError("Bubblewrap and rustc are required for the isolated CLI build")
    forgejo_hash = webhook_helpers.sha256_file(forgejo)
    if forgejo_hash != FORGEJO_SHA256:
        raise RuntimeError("Forgejo binary hash differs from the previously identified local build")
    version = subprocess.run(
        [str(forgejo), "--version"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env={"PATH": "/usr/bin:/bin", "HOME": "/tmp"},
        timeout=10,
        check=False,
    )
    if version.returncode or FORGEJO_VERSION.encode() not in version.stdout:
        raise RuntimeError("this E2E requires the locally identified Forgejo 17.0.0 binary")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3132)) == 0:
            raise RuntimeError("127.0.0.1:3132 is occupied; no server was started")

    state.mkdir(mode=0o700)
    for name in ("data", "repositories", "log", "tmp", "home"):
        (state / name).mkdir(mode=0o700)
    admission_helpers.create_app_ini(state)
    emitted.private_config_with_test_allowlist(state)
    project = state / "project"
    admission_helpers.copy_project(repo, project)
    evidence_dir.mkdir(mode=0o700, parents=True)

    tokens: list[str] = []
    server_log = (state / "log/forgejo-server.log").open("xb")
    server: subprocess.Popen[bytes] | None = None
    receiver: subprocess.Popen[bytes] | None = None
    receiver_log_stream: Any = None
    receiver_port: int | None = None
    receiver_address: tuple[str, int] | None = None
    receiver_binary: Path | None = None
    mission_binary: Path | None = None
    build_log: bytes | None = None
    owner_api = None
    owner_password: str | None = None
    hook_id: int | None = None
    hook_api_readback: dict[str, Any] | None = None
    branch_protection_readback: dict[str, Any] | None = None
    hook_disabled = False
    hook_target: str | None = None
    event: dict[str, Any] | None = None
    task_binding: dict[str, Any] | None = None
    admission_result: dict[str, Any] | None = None
    replay_result: dict[str, Any] | None = None
    native_replay_binding: dict[str, Any] | None = None
    native_replay_admission: dict[str, Any] | None = None
    native_task_readback: list[dict[str, Any]] | None = None
    invalid_credential_result: dict[str, Any] | None = None
    invalid_status: int | None = None
    stage = "isolated_build"
    failure: str | None = None
    failure_detail: str | None = None
    cleanup_failure: str | None = None
    server_stop: dict[str, Any] | None = None
    receiver_stop: dict[str, Any] | None = None
    webhook_secret: bytes | None = None
    journal_path = project / ".runtime/forgejo-webhook-inbox.jsonl"
    requirement_path = f"requirements/{REQUIREMENT_ID}.json"
    requirement = (repo / f"tests/fixtures/mission-012/requirements/{REQUIREMENT_ID}.json").read_bytes()
    base_commit: str | None = None
    issue_number: int | None = None
    read_token: str | None = None
    allocation_id = args.allocation_id
    issue_tasks: list[dict[str, Any]] = []

    try:
        stage = "build_trusted_local_adapters"
        include_mcp_host = bool(getattr(args, "build_mcp_host", False))
        mission_binary, build_log, receiver_binary = admission_helpers.build_cli_isolated(
            project,
            state,
            repo,
            include_webhook_receiver=True,
            include_mcp_host=include_mcp_host,
        )
        if receiver_binary is None:
            raise RuntimeError("isolated build omitted the real webhook receiver")

        stage = "start_forgejo"
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
            env=webhook_helpers.private_command_environment(state),
            stdin=subprocess.DEVNULL,
            stdout=server_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        admission_helpers.wait_for_health(server)
        admission_helpers.ensure_owned_listener(server)

        stage = "provision_private_repository"
        owner_password_output = webhook_helpers.admin_command(
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
        if bool(getattr(args, "native_replay_admission", False)):
            password_matches = native_replay.PASSWORD_RE.findall(owner_password_output)
            if len(password_matches) != 1:
                raise RuntimeError("Forgejo did not return one generated owner password")
            owner_password = password_matches[0].decode("utf-8")
            tokens.append(owner_password)
        provision_token = webhook_helpers.create_access_token(
            forgejo,
            state,
            OWNER,
            "live-webhook-mission-provisioner",
            "write:user,write:repository,write:issue",
        )
        tokens.append(provision_token)
        owner_api = admission_helpers.api(provision_token)
        created_repo = _expect_api(
            owner_api,
            "POST",
            "/user/repos",
            {200, 201},
            {
                "name": REPOSITORY,
                "description": "Private live-webhook mission admission test",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if not isinstance(created_repo, dict) or created_repo.get("private") is not True:
            raise RuntimeError("Forgejo did not create the expected private repository")

        stage = "seed_versioned_mission_fixture"
        for path, contents in fixture_files(requirement).items():
            _expect_api(
                owner_api,
                "POST",
                f"/repos/{OWNER}/{REPOSITORY}/contents/{path}",
                {200, 201},
                {
                    "message": f"Seed isolated MISSION-012 fixture {path}",
                    "content": base64.b64encode(contents).decode("ascii"),
                    "branch": "main",
                },
            )
        stage = "provision_independent_reviewer"
        webhook_helpers.admin_command(
            forgejo,
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
        _expect_api(
            owner_api,
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{REVIEWER}",
            {200, 201, 204},
            {"permission": "write"},
        )

        stage = "protect_main_for_independent_review_and_merge"
        protection = mission_branch_protection_policy()
        _expect_api(
            owner_api,
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/branch_protections",
            {200, 201},
            protection,
        )
        protection_list = _expect_api(
            owner_api, "GET", f"/repos/{OWNER}/{REPOSITORY}/branch_protections", {200}
        )
        matching_protections = [
            item for item in protection_list
            if isinstance(item, dict) and item.get("rule_name") == "main"
        ] if isinstance(protection_list, list) else []
        if len(matching_protections) != 1 or not branch_protection_matches_policy(matching_protections[0]):
            raise RuntimeError("Forgejo branch protection read-back differs from the reviewer and merge policy")
        branch_protection_readback = matching_protections[0]
        branch = _expect_api(
            owner_api, "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main", {200}
        )
        if not isinstance(branch, dict) or branch.get("protected") is not True:
            raise RuntimeError("Forgejo readback did not mark the mission base branch protected")
        base_commit = branch.get("commit", {}).get("id")
        if not isinstance(base_commit, str) or not base_commit:
            raise RuntimeError("Forgejo did not return the protected base commit")

        stage = "provision_read_only_agent_credential"
        webhook_helpers.admin_command(
            forgejo,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                webhook_helpers.READER,
                "--random-password",
                "--email",
                f"{webhook_helpers.READER}@local.invalid",
                "--must-change-password=false",
            ],
        )
        _expect_api(
            owner_api,
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{webhook_helpers.READER}",
            {200, 201, 204},
            {"permission": "read"},
        )
        reader_token = webhook_helpers.create_access_token(
            forgejo,
            state,
            webhook_helpers.READER,
            "live-webhook-mission-read-only",
            READ_SCOPES,
        )
        tokens.append(reader_token)
        reader_api = admission_helpers.api(reader_token)
        repository_readback = _expect_api(
            reader_api, "GET", f"/repos/{OWNER}/{REPOSITORY}", {200}
        )
        permissions = repository_readback.get("permissions") if isinstance(repository_readback, dict) else None
        if (
            not isinstance(permissions, dict)
            or repository_readback.get("private") is not True
            or permissions.get("admin") is not False
            or permissions.get("push") is not False
            or permissions.get("pull") is not True
        ):
            raise RuntimeError("Forgejo did not confirm private read-only repository permissions")

        stage = "start_receiver_and_create_issue_hook"
        webhook_secret = generate_webhook_secret()
        webhook_secret_text = forgejo_secret_value(webhook_secret)
        secret_path = state / "webhook.secret"
        admission_helpers.private_file(secret_path, webhook_secret)
        inbox_path = journal_path
        receiver, receiver_address, receiver_log_stream = admission_helpers.start_webhook_receiver(
            receiver_binary,
            secret_path,
            inbox_path,
            state / "log/receiver-process.log",
        )
        receiver_port, hook_target = receiver_target(receiver_address)
        webhook_helpers.wait_for_listener(receiver, receiver_port)
        hook = _expect_api(
            owner_api,
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/hooks",
            {200, 201},
            {
                "type": "gitea",
                "config": {
                    "url": hook_target,
                    "content_type": "json",
                    "secret": webhook_secret_text,
                },
                "events": ["issues"],
                "active": True,
            },
        )
        hook_id = hook.get("id") if isinstance(hook, dict) else None
        if not isinstance(hook_id, int) or hook.get("active") is not True:
            raise RuntimeError("Forgejo did not create the expected active issue hook")

        stage = "emit_real_opened_issue_event"
        issue = _expect_api(
            owner_api,
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {200, 201},
            {
                "title": "MISSION-012: clamp upper boundary",
                "body": (
                    "Correct clamp_to_range so values above maximum return maximum. "
                    "Preserve lower-bound and inclusive in-range behavior. Edit only "
                    "src/lib.rs and run the protected acceptance tests before and after the fix."
                ),
            },
        )
        issue_number = issue.get("number") if isinstance(issue, dict) else None
        if issue_number != 1 or issue.get("state") != "open":
            raise RuntimeError("Forgejo did not create the expected open mission issue #1")
        inbox_records = emitted.wait_for_journal_count(inbox_path, 1)
        if len(inbox_records) != 1:
            raise RuntimeError("receiver journal contains an unexpected number of issue deliveries")
        event = parse_received_issue_event(
            inbox_records[0],
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
            expected_action="opened",
        )
        issue_tasks = _wait_for_issue_task(state / "data/forgejo.db", hook_id)
        task_binding = validate_single_hook_task_binding(issue_tasks, event)

        stage = "refuse_invalid_host_credential"
        requirement_sha256 = hashlib.sha256(requirement).hexdigest()
        credential_path = project / ".runtime/forgejo-read.token"
        admission_helpers.private_file(credential_path, reader_token.encode("ascii") + b"\n")
        bad_credential_path = project / ".runtime/invalid-forgejo-read.token"
        bad_token = "invalid-" + secrets.token_urlsafe(24)
        tokens.append(bad_token)
        admission_helpers.private_file(bad_credential_path, bad_token.encode("ascii") + b"\n")
        admission_path = project / ".runtime/host-admission.json"
        mission_journal = project / ".runtime/mission-execution.jsonl"
        common_arguments = [
            "--profile",
            "mission-012-clamp",
            "--issue",
            "1",
            "--requirement",
            REQUIREMENT_ID,
            "--revision",
            "1",
            "--allocation-id",
            allocation_id,
            "--workspace-id",
            "mission-012-clamp-issue-1",
            "--delivery-id",
            event["delivery_id"],
            "--requirement-sha256",
            requirement_sha256,
            "--host-admission-file",
            str(admission_path),
        ]
        invalid_status, invalid_credential_result = _run_mission_cli(
            mission_binary,
            project,
            state,
            [*common_arguments, "--host-credential-file", str(bad_credential_path)],
            bad_token.encode("ascii"),
        )
        if (
            invalid_status != 1
            or invalid_credential_result.get("error", {}).get("code")
            != "mission_source_unauthorized"
            or mission_journal.exists()
            or admission_path.exists()
        ):
            raise RuntimeError("invalid host credential did not fail before mission claim state")

        stage = "admit_emitted_issue_delivery"
        authenticated_status, admission_result = _run_mission_cli(
            mission_binary,
            project,
            state,
            [*common_arguments, "--host-credential-file", str(credential_path)],
            reader_token.encode("ascii"),
        )
        if authenticated_status != 0:
            raise RuntimeError("valid Forgejo issue delivery was not admitted")
        validate_admission_result(
            admission_result,
            event=event,
            requirement_sha256=requirement_sha256,
            base_commit=base_commit,
            allocation_id=allocation_id,
        )
        if (
            not admission_path.is_file()
            or admission_path.stat().st_mode & 0o777 != 0o600
            or not mission_journal.is_file()
            or mission_journal.stat().st_mode & 0o777 != 0o600
            or reader_token.encode("ascii") in admission_path.read_bytes()
            or reader_token.encode("ascii") in mission_journal.read_bytes()
            or credential_path.stat().st_mode & 0o777 != 0o600
        ):
            raise RuntimeError("mission claim or host credential did not preserve private file state")
        admission_sha256 = webhook_helpers.sha256_file(admission_path)
        mission_journal_sha256 = webhook_helpers.sha256_file(mission_journal)

        stage = "replay_same_emitted_delivery"
        replay_status, replay_result = _run_mission_cli(
            mission_binary,
            project,
            state,
            [*common_arguments, "--host-credential-file", str(credential_path)],
            reader_token.encode("ascii"),
        )
        if (
            replay_status != 0
            or replay_result.get("verdict") != "already_claimed"
            or replay_result.get("host_admission") != admission_result.get("host_admission")
            or replay_result.get("allocation_replayed") is not True
            or webhook_helpers.sha256_file(admission_path) != admission_sha256
            or webhook_helpers.sha256_file(mission_journal) != mission_journal_sha256
            or len(emitted.read_journal_records(inbox_path)) != 1
        ):
            raise RuntimeError("same webhook delivery replay changed the durable mission claim")

        if bool(getattr(args, "native_replay_admission", False)):
            if owner_password is None:
                raise RuntimeError("native replay admission requires the private owner session")
            stage = "forgejo_native_replay_new_delivery_uuid"
            web_session = native_replay.login_web_session(OWNER, owner_password, origin=ORIGIN)
            hook_details_path = f"/{OWNER}/{REPOSITORY}/settings/hooks/{hook_id}"
            native_replay_path = f"{hook_details_path}/replay/{event['delivery_id']}"
            native_status, native_headers, _ = native_replay.local_web_request(
                web_session,
                "POST",
                native_replay_path,
                b"",
                {
                    "Origin": ORIGIN,
                    "Sec-Fetch-Site": "same-origin",
                    "Referer": ORIGIN + hook_details_path,
                },
                origin=ORIGIN,
            )
            location = native_headers.get("location")
            redirected = urlsplit(urljoin(ORIGIN + native_replay_path, location or ""))
            expected_origin = urlsplit(ORIGIN)
            if (
                native_status not in {302, 303}
                or redirected.scheme != expected_origin.scheme
                or redirected.netloc != expected_origin.netloc
                or redirected.path != hook_details_path
            ):
                raise RuntimeError("Forgejo native replay did not redirect to its local hook page")
            native_tasks = native_replay.wait_for_task_count(
                state / "data/forgejo.db", hook_id, expected_count=2, final_status=202
            )
            native_records = emitted.wait_for_journal_count(inbox_path, 2)
            if len(native_tasks) != 2 or len(native_records) != 2:
                raise RuntimeError("native replay created an unexpected number of tasks or events")
            native_task_readback = native_tasks
            native_events = [
                parse_received_issue_event(
                    record,
                    owner=OWNER,
                    repository=REPOSITORY,
                    issue_number=issue_number,
                    expected_action="opened",
                )
                for record in native_records
            ]
            native_replay_binding = validate_replayed_issue_delivery_binding(
                native_tasks[0], native_tasks[1], native_events[0], native_events[1]
            )
            if (
                native_events[0].get("delivery_id") != event.get("delivery_id")
                or native_events[1].get("delivery_id") != native_replay_binding["replay_delivery_id"]
            ):
                raise RuntimeError("Forgejo native replay event UUIDs differ from their task readback")

            mission_journal_before = mission_journal.read_bytes()
            admission_file_before = admission_path.read_bytes()
            replay_arguments = list(common_arguments)
            replay_delivery_index = replay_arguments.index("--delivery-id") + 1
            replay_arguments[replay_delivery_index] = native_replay_binding["replay_delivery_id"]
            native_cli_status, native_cli_result = _run_mission_cli(
                mission_binary,
                project,
                state,
                [*replay_arguments, "--host-credential-file", str(credential_path)],
                reader_token.encode("ascii"),
            )
            mission_journal_after = mission_journal.read_bytes()
            admission_file_after = admission_path.read_bytes()
            if native_cli_status != 0:
                raise RuntimeError("new native replay delivery was not admitted idempotently")
            native_replay_admission = validate_native_replay_admission_result(
                admission_result,
                native_cli_result,
                expected_delivery_id=native_replay_binding["replay_delivery_id"],
                mission_journal_before=mission_journal_before,
                mission_journal_after=mission_journal_after,
                admission_file_before=admission_file_before,
                admission_file_after=admission_file_after,
            )
            if len(emitted.read_journal_records(inbox_path)) != 2:
                raise RuntimeError("native replay did not append exactly one receiver event")

        stage = "verify_issue_and_disable_hook"
        issue_readback = _expect_api(
            owner_api, "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/1", {200}
        )
        if issue_readback.get("state") != "open":
            raise RuntimeError("Forgejo issue readback no longer matches its open mission state")
        _expect_api(
            owner_api,
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
            {200, 204},
            emitted.hook_deactivation_patch(("issues",)),
        )
        hook_api_readback = _expect_api(
            owner_api, "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}", {200}
        )
        hook_readback = mission_hook_readback_record(
            hook_id, hook_api_readback, hook_target
        )
        hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
        if not hook_disabled:
            raise RuntimeError("Forgejo hook readback did not confirm deactivation")
        if not hook_readback["configuration_matches_expected"]:
            raise RuntimeError("Forgejo issue hook readback differs from its created configuration")
        stage = "complete"
    except Exception as error:
        failure = type(error).__name__
        failure_detail = safe_failure_detail(error, tokens, webhook_secret)
    finally:
        if hook_id is not None and owner_api is not None and not hook_disabled:
            try:
                _expect_api(
                    owner_api,
                    "PATCH",
                    f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                    {200, 204},
                    emitted.hook_deactivation_patch(("issues",)),
                )
                hook_api_readback = _expect_api(
                    owner_api, "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}", {200}
                )
                readback = mission_hook_readback_record(
                    hook_id, hook_api_readback, hook_target
                )
                hook_disabled = readback["deactivation_readback_confirmed"] is True
                if not hook_disabled:
                    cleanup_failure = "hook_deactivation_not_confirmed"
                elif not readback["configuration_matches_expected"]:
                    cleanup_failure = "hook_configuration_readback_mismatch"
            except Exception as error:
                cleanup_failure = type(error).__name__
        receiver_stop = webhook_helpers.stop_process(receiver)
        server_stop = webhook_helpers.stop_process(server)
        if receiver_log_stream is not None and not receiver_log_stream.closed:
            receiver_log_stream.close()
        server_log.close()

    assert receiver_stop is not None and server_stop is not None
    cleanup_readback = {
        "forgejo": {
            "expected_address": "127.0.0.1:3132",
            "stop": server_stop,
            "listener_addresses_after_stop": (
                webhook_helpers.listener_addresses_for_port(3132) if server is not None else []
            ),
            "listener_checked": server is not None,
        },
        "receiver": {
            "expected_address": f"127.0.0.1:{receiver_port}" if receiver_port is not None else None,
            "stop": receiver_stop,
            "listener_addresses_after_stop": (
                webhook_helpers.listener_addresses_for_port(receiver_port)
                if receiver_port is not None
                else []
            ),
            "listener_checked": receiver is not None,
        },
        "hook_disabled_before_server_stop": hook_disabled,
        "cleanup_failure": cleanup_failure,
    }
    cleanup_readback["tracked_processes_exited"] = bool(
        server_stop["stopped"] and receiver_stop["stopped"]
    )
    cleanup_readback["expected_tcp_listeners_absent"] = not any(
        cleanup_readback[service]["listener_addresses_after_stop"]
        for service in ("forgejo", "receiver")
    )
    webhook_helpers.write_safe_json(evidence_dir / "port-cleanup-readback.json", cleanup_readback, tokens)
    webhook_helpers.write_safe_json(
        evidence_dir / "hook-readback.json",
        mission_hook_readback_record(hook_id, hook_api_readback, hook_target),
        tokens,
    )
    if branch_protection_readback is not None:
        webhook_helpers.write_safe_json(
            evidence_dir / "branch-protection-readback.json", branch_protection_readback, tokens
        )
    if failure is not None or cleanup_failure is not None:
        webhook_helpers.write_safe_json(
            evidence_dir / "failure.json",
            {
                "verdict": "failed",
                "stage": stage,
                "error_type": failure,
                "error_detail": failure_detail,
                "cleanup_failure": cleanup_failure,
            },
            tokens,
        )
        webhook_helpers.write_checksums(evidence_dir)
        raise RuntimeError(f"live Forgejo webhook mission-admission E2E failed at {stage}") from None

    assert event is not None and task_binding is not None
    assert admission_result is not None and replay_result is not None
    assert invalid_credential_result is not None
    assert mission_binary is not None and receiver_binary is not None and build_log is not None
    if (
        not cleanup_readback["tracked_processes_exited"]
        or not cleanup_readback["expected_tcp_listeners_absent"]
        or not hook_disabled
    ):
        webhook_helpers.write_safe_json(
            evidence_dir / "failure.json",
            {"verdict": "failed", "stage": "cleanup_readback", "error_type": "CleanupNotConfirmed"},
            tokens,
        )
        webhook_helpers.write_checksums(evidence_dir)
        raise RuntimeError("local Forgejo, receiver or hook did not stop cleanly")

    source_provenance = _source_provenance(repo)
    safe_result = {
        "schema_version": 1,
        "verdict": "passed",
        "forgejo": {
            "version": version.stdout.decode("utf-8", "replace").strip(),
            "binary_sha256": forgejo_hash,
            "listener": "127.0.0.1:3132",
            "test_only_webhook_allowlist": "127.0.0.1",
        },
        "repository": {
            "full_name": f"{OWNER}/{REPOSITORY}",
            "private": True,
            "branch_protection_readback_verified": branch_protection_readback is not None,
            "reviewer_login": REVIEWER,
            "merge_whitelist_usernames": [OWNER],
            "merge_whitelist_teams": [],
            "reader": {
                "login": webhook_helpers.READER,
                "admin": False,
                "push": permissions.get("push"),
                "pull": permissions.get("pull"),
                "token_scopes": READ_SCOPES.split(","),
            },
        },
        "mission": {
            "issue_number": issue_number,
            "issue_state": issue_readback.get("state"),
            "profile_id": "mission-012-clamp",
            "requirement_path": requirement_path,
            "requirement_sha256": hashlib.sha256(requirement).hexdigest(),
            "protected_base_commit": base_commit,
            "allocation_id": allocation_id,
            "admission": admission_result,
            "admission_file_sha256": webhook_helpers.sha256_file(admission_path),
            "mission_journal_sha256": webhook_helpers.sha256_file(mission_journal),
            "admission_file_mode": oct(admission_path.stat().st_mode & 0o777),
            "mission_journal_mode": oct(mission_journal.stat().st_mode & 0o777),
            "agent_runtime_invoked": False,
        },
        "webhook": {
            "emitter": "Forgejo v17 actual issue hook task",
            **mission_hook_readback_record(hook_id, hook_api_readback, hook_target),
            "event_type": "issues",
            "event": event,
            "task_binding": task_binding,
            "task_readback": issue_tasks,
            "native_replay_task_readback": native_task_readback,
            "receiver_journal_records": 2 if native_replay_binding is not None else 1,
            "native_replay_binding": native_replay_binding,
            "secret_recorded": False,
        },
        "refusals": {
            "invalid_host_credential": {
                "exit_status": invalid_status,
                "error_code": invalid_credential_result.get("error", {}).get("code"),
                "mission_journal_created": False,
                "admission_file_created": False,
            },
            "replay": {
                "verdict": replay_result.get("verdict"),
                "allocation_replayed": replay_result.get("allocation_replayed"),
                "mission_journal_unchanged": True,
                "admission_file_unchanged": True,
                "receiver_inbox_records": 1,
            },
        },
        "native_replay_admission": native_replay_admission,
        "process_cleanup": cleanup_readback,
        "source_head_commit": source_provenance["head_commit"],
        "limitations": [
            "This run proves actual Forgejo webhook delivery through the receiver into versioned mission admission; it does not execute an agent, create a pull request, verify a candidate, or merge it.",
            "No LLM, AgentOS, RAG, or memory integration was invoked; no productivity or token-cost comparison was measured.",
            "Forgejo, receiver, and the trusted admission CLI share the host UID. Candidate code is stored as context but is not executed in this run.",
            "Forgejo hook-task evidence uses its local v17 SQLite table and is not a supported public API.",
            "The loopback webhook allowlist applies only to this disposable instance.",
        ],
    }
    provenance = {
        **source_provenance,
        "forgejo_binary_sha256": forgejo_hash,
        "built_executable_sha256": built_executable_hashes(
            state,
            mission_binary,
            receiver_binary,
            include_mcp_host=bool(getattr(args, "build_mcp_host", False)),
        ),
        "mcp_host_built": bool(getattr(args, "build_mcp_host", False)),
        "isolated_build_log_sha256": hashlib.sha256(build_log).hexdigest(),
        "private_state_dir": str(state),
        "private_state_contents_in_evidence": False,
        "webhook_secret_recorded": False,
    }
    webhook_helpers.write_safe_json(evidence_dir / "result.json", safe_result, tokens)
    webhook_helpers.write_safe_json(evidence_dir / "admission.json", admission_result, tokens)
    webhook_helpers.write_safe_json(evidence_dir / "replay.json", replay_result, tokens)
    if native_replay_binding is not None and native_replay_admission is not None:
        webhook_helpers.write_safe_json(
            evidence_dir / "native-replay-binding.json", native_replay_binding, tokens
        )
        webhook_helpers.write_safe_json(
            evidence_dir / "native-replay-admission.json", native_replay_admission, tokens
        )
    webhook_helpers.write_safe_json(evidence_dir / "invalid-credential.json", invalid_credential_result, tokens)
    webhook_helpers.write_safe_json(evidence_dir / "webhook-event.json", event, tokens)
    webhook_helpers.write_safe_json(evidence_dir / "forgejo-task-binding.json", task_binding, tokens)
    webhook_helpers.write_safe_json(evidence_dir / "provenance.json", provenance, tokens)
    webhook_helpers.write_checksums(evidence_dir)
    return safe_result


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(SCRIPT_DIR.parent))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument("--allocation-id", default="mission012-live-webhook-001")
    parser.add_argument(
        "--build-mcp-host",
        action="store_true",
        help="also build the exact MCP host consumed by the following live-agent stage",
    )
    parser.add_argument(
        "--native-replay-admission",
        action="store_true",
        help="exercise Forgejo native replay with a new UUID and confirm admission reuses one claim",
    )
    return parser


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    return argument_parser().parse_args(argv)


def main() -> int:
    args = parse_args()
    result = run(args)
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
