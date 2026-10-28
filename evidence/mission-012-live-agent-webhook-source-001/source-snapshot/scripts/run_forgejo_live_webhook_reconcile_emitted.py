#!/usr/bin/env python3
"""Run Forgejo-emitted issue and pull-request webhooks through the receiver."""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import http.client
import json
import os
from pathlib import Path
import re
import secrets
import signal
import shutil
import socket
import sqlite3
import stat
import subprocess
import sys
import time
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_e2e as helpers  # noqa: E402

OWNER = helpers.OWNER
REPOSITORY = helpers.REPOSITORY
READER = helpers.READER
ORIGIN = helpers.ORIGIN
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
DELIVERY_ID_RE = re.compile(r"^[A-Za-z0-9_-]{1,128}$")
MAX_JOURNAL_BYTES = 4 * 1024 * 1024
COMMIT_SHA_RE = re.compile(r"^(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})$")
HOOK_EVENT_SELECTORS = ("issues", "pull_request")
ISSUE_HOOK_EVENTS = frozenset(
    {
        "issues",
        "issue_assign",
        "issue_label",
        "issue_milestone",
        "issue_comment",
    }
)
PULL_REQUEST_HOOK_EVENTS = frozenset(
    {
        "pull_request",
        "pull_request_assign",
        "pull_request_label",
        "pull_request_milestone",
        "pull_request_comment",
        "pull_request_review_approved",
        "pull_request_review_rejected",
        "pull_request_review_comment",
        "pull_request_sync",
        "pull_request_review_request",
    }
)
HOOK_EVENT_GROUPS = {
    "issues": ISSUE_HOOK_EVENTS,
    "pull_request": PULL_REQUEST_HOOK_EVENTS,
}


def effective_hook_events(event_selectors: tuple[str, ...]) -> frozenset[str]:
    if (
        not event_selectors
        or len(event_selectors) != len(set(event_selectors))
        or any(selector not in HOOK_EVENT_GROUPS for selector in event_selectors)
    ):
        raise ValueError("unsupported or duplicate Forgejo webhook event selector")
    return frozenset().union(*(HOOK_EVENT_GROUPS[selector] for selector in event_selectors))


def parse_live_delivery_record(
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    issue_number: int,
    expected_action: str,
) -> dict[str, Any]:
    """Validate a receiver journal record for one expected live Forgejo issue event."""
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != "forgejo.webhook.v1.issues":
        raise ValueError("journal record is not an issues event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "issues"
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook payload is invalid")
    repository_ref = payload.get("repository")
    issue = payload.get("issue")
    if not isinstance(repository_ref, dict) or not isinstance(issue, dict):
        raise ValueError("webhook issue reference is missing")
    if repository_ref.get("full_name") != f"{owner}/{repository}":
        raise ValueError("webhook repository does not match the configured repository")
    reported_number = issue.get("number")
    if (
        not isinstance(reported_number, int)
        or isinstance(reported_number, bool)
        or reported_number != issue_number
    ):
        raise ValueError("webhook issue number does not match the configured issue")
    action = payload.get("action")
    if action != expected_action or expected_action not in {"opened", "closed", "edited"}:
        raise ValueError("webhook action does not match the expected transition")
    payload_state = issue.get("state")
    if payload_state not in {"open", "closed"}:
        raise ValueError("webhook issue state is invalid")
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "issues",
        "action": action,
        "issue_number": issue_number,
        "payload_state": payload_state,
    }


def parse_live_pull_request_delivery_record(
    record: dict[str, Any],
    *,
    owner: str,
    repository: str,
    pull_request_number: int,
    expected_action: str,
    expected_event: str = "pull_request",
) -> dict[str, Any]:
    """Validate a PR or PR-review notification without treating it as approval proof."""
    review_types = {
        "pull_request_approved": "pull_request_review_approved",
        "pull_request_rejected": "pull_request_review_rejected",
        "pull_request_comment": "pull_request_review_comment",
    }
    allowed_actions = {
        "pull_request": {"opened", "closed"},
        **{event: {"reviewed"} for event in review_types},
    }
    if (
        not isinstance(expected_event, str)
        or not isinstance(expected_action, str)
        or expected_event not in allowed_actions
        or expected_action not in allowed_actions[expected_event]
    ):
        raise ValueError("expected Forgejo pull-request event/action pair is unsupported")
    if not isinstance(record, dict):
        raise ValueError("journal record must be an object")
    sequence = record.get("sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1:
        raise ValueError("journal sequence is invalid")
    if record.get("kind") != f"forgejo.webhook.v1.{expected_event}":
        raise ValueError("journal record is not the expected pull-request event")
    envelope = record.get("payload")
    if not isinstance(envelope, dict):
        raise ValueError("journal payload envelope is invalid")
    delivery_id = envelope.get("delivery_id")
    if (
        envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != expected_event
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
    ):
        raise ValueError("journal delivery envelope is invalid")
    payload = envelope.get("payload")
    if not isinstance(payload, dict):
        raise ValueError("webhook payload is invalid")
    repository_ref = payload.get("repository")
    pull_request = payload.get("pull_request")
    if not isinstance(repository_ref, dict) or not isinstance(pull_request, dict):
        raise ValueError("webhook pull-request reference is missing")
    if repository_ref.get("full_name") != f"{owner}/{repository}":
        raise ValueError("webhook repository does not match the configured repository")
    reported_number = pull_request.get("number")
    if (
        not isinstance(reported_number, int)
        or isinstance(reported_number, bool)
        or reported_number != pull_request_number
        or pull_request_number < 1
    ):
        raise ValueError("webhook pull-request number does not match the configured request")
    action = payload.get("action")
    if action != expected_action:
        raise ValueError("webhook action does not match the expected transition")
    payload_state = pull_request.get("state")
    payload_merged = pull_request.get("merged")
    if payload_state not in {"open", "closed"} or not isinstance(payload_merged, bool):
        raise ValueError("webhook pull-request state is invalid")
    head = pull_request.get("head")
    base = pull_request.get("base")
    head_sha = head.get("sha") if isinstance(head, dict) else None
    base_sha = base.get("sha") if isinstance(base, dict) else None
    if (
        not isinstance(head_sha, str)
        or not COMMIT_SHA_RE.fullmatch(head_sha)
        or not isinstance(base_sha, str)
        or not COMMIT_SHA_RE.fullmatch(base_sha)
    ):
        raise ValueError("webhook pull-request commit SHA is invalid")
    review_type = None
    if expected_event in review_types:
        review = payload.get("review")
        review_type = review.get("type") if isinstance(review, dict) else None
        if review_type != review_types[expected_event]:
            raise ValueError("Forgejo review notification type does not match its event")
    result = {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": expected_event,
        "action": action,
        "pull_request_number": pull_request_number,
        "payload_state": payload_state,
        "payload_merged": payload_merged,
        "head_sha": head_sha,
        "base_sha": base_sha,
    }
    if review_type is not None:
        result["review_type"] = review_type
    return result


def read_journal_records(path: Path) -> list[dict[str, Any]]:
    try:
        size = path.stat().st_size
    except FileNotFoundError:
        return []
    if size > MAX_JOURNAL_BYTES:
        raise RuntimeError("local webhook journal exceeded the E2E read limit")
    raw = path.read_bytes()
    if raw and not raw.endswith(b"\n"):
        raise RuntimeError("local webhook journal ended with a partial record")
    records: list[dict[str, Any]] = []
    for line_number, line in enumerate(raw.splitlines(), 1):
        if not line:
            raise RuntimeError(f"local webhook journal contains an empty record at line {line_number}")
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            raise RuntimeError(f"local webhook journal has invalid JSON at line {line_number}") from None
        if not isinstance(value, dict):
            raise RuntimeError(f"local webhook journal record {line_number} is not an object")
        records.append(value)
    return records


def wait_for_journal_count(path: Path, expected: int, timeout: float = 20.0) -> list[dict[str, Any]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        records = read_journal_records(path)
        if len(records) >= expected:
            return records
        time.sleep(0.1)
    raise RuntimeError("Forgejo did not deliver the expected webhook event to the receiver")


def validate_hook_task_bindings(
    tasks: list[dict[str, Any]], events: list[dict[str, Any]]
) -> None:
    """Bind Forgejo task UUIDs to accepted receiver events independent of delivery order."""
    if not tasks or len(tasks) != len(events):
        raise ValueError("Forgejo task and receiver event counts do not match")
    task_types = {
        "issues": {"issues"},
        "pull_request": {
            "pull_request", "pull_request_assign", "pull_request_label",
            "pull_request_milestone", "pull_request_sync", "pull_request_review_request",
        },
        "pull_request_approved": {"pull_request_review_approved"},
        "pull_request_rejected": {"pull_request_review_rejected"},
        "pull_request_comment": {"pull_request_review_comment"},
    }
    by_delivery = {
        event.get("delivery_id"): event for event in events if isinstance(event, dict)
    }
    if len(by_delivery) != len(events) or any(not isinstance(key, str) for key in by_delivery):
        raise ValueError("receiver journal contains duplicate or invalid delivery IDs")
    seen: set[str] = set()
    for task in tasks:
        if (
            not isinstance(task, dict)
            or not isinstance(task.get("uuid"), str)
            or task.get("uuid") in seen
        ):
            raise ValueError("Forgejo delivery tasks contain duplicate or invalid UUIDs")
        event = by_delivery.get(task["uuid"])
        event_name = event.get("event") if isinstance(event, dict) else None
        if (
            event_name not in task_types
            or task.get("event_type") not in task_types.get(event_name, set())
            or task.get("is_delivered") is not True
            or task.get("is_succeed") is not True
            or task.get("response_status") != 202
        ):
            raise ValueError("Forgejo delivery task does not match its receiver journal event")
        seen.add(task["uuid"])


def validate_reconciliation_delivery_binding(
    event: dict[str, Any], result: dict[str, Any]
) -> None:
    """Ensure a reconciliation result belongs to the delivery that was requested."""
    delivery_id = event.get("delivery_id") if isinstance(event, dict) else None
    result_delivery_id = result.get("delivery_id") if isinstance(result, dict) else None
    if not isinstance(delivery_id, str) or not delivery_id or result_delivery_id != delivery_id:
        raise ValueError("reconciliation result delivery ID does not match the requested event")


def validate_duplicate_delivery_replay(
    event: dict[str, Any],
    status: int,
    response: dict[str, Any],
    records_before: list[dict[str, Any]],
    records_after: list[dict[str, Any]],
) -> None:
    """Require a redelivery to bind to the same event without extending the journal."""
    delivery_id = event.get("delivery_id") if isinstance(event, dict) else None
    sequence = event.get("sequence") if isinstance(event, dict) else None
    if (
        not isinstance(delivery_id, str)
        or not delivery_id
        or not isinstance(sequence, int)
        or isinstance(sequence, bool)
        or not isinstance(response, dict)
        or type(status) is not int
        or status != 202
        or response.get("delivery_id") != delivery_id
    ):
        raise ValueError("duplicate replay response delivery ID or HTTP status is invalid")
    if (
        response.get("accepted") is not True
        or response.get("duplicate") is not True
        or response.get("reconcile_required") is not True
        or type(response.get("sequence")) is not int
        or response.get("sequence") != sequence
    ):
        raise ValueError("receiver did not confirm the matching duplicate delivery")
    if records_before != records_after:
        raise ValueError("receiver journal changed during duplicate delivery replay")


def validate_journal_snapshot_unchanged(before: bytes, after: bytes) -> None:
    """Reject byte-level journal rewrites even when their parsed JSON is equal."""
    if not isinstance(before, bytes) or not isinstance(after, bytes) or before != after:
        raise ValueError("receiver journal bytes changed during duplicate delivery replay")


def kill_process_with_sigkill(process: subprocess.Popen[bytes] | None) -> dict[str, Any]:
    """Send SIGKILL to a running process and report its observed termination signal."""
    if process is None:
        return {
            "started": False,
            "stopped": True,
            "returncode": None,
            "stop_action": "not_started",
            "stop_signal": None,
        }

    action = "already_exited"
    if process.poll() is None:
        action = "sigkill"
        try:
            process.kill()
        except ProcessLookupError:
            action = "exited_during_sigkill"
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            raise RuntimeError("receiver did not exit after SIGKILL") from None

    returncode = process.returncode
    return {
        "started": True,
        "stopped": process.poll() is not None,
        "returncode": returncode,
        "stop_action": action,
        "stop_signal": "SIGKILL" if returncode == -signal.SIGKILL else None,
    }


def send_redelivered_webhook(
    receiver_address: str,
    secret: bytes,
    delivery_id: str,
    event_name: str,
    payload: dict[str, Any],
) -> tuple[int, dict[str, Any]]:
    """Resend one captured journal payload to the loopback receiver with its HMAC."""
    host, separator, port_text = receiver_address.rpartition(":")
    if (
        separator != ":"
        or host != "127.0.0.1"
        or not port_text.isascii()
        or not port_text.isdecimal()
        or not 1 <= int(port_text) <= 65535
        or not isinstance(secret, bytes)
        or not secret
        or not isinstance(delivery_id, str)
        or not DELIVERY_ID_RE.fullmatch(delivery_id)
        or not isinstance(event_name, str)
        or event_name not in HOOK_EVENT_GROUPS
        or not isinstance(payload, dict)
    ):
        raise ValueError("redelivery target or webhook event is invalid")
    try:
        body = json.dumps(
            payload,
            ensure_ascii=False,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
    except (TypeError, ValueError):
        raise ValueError("redelivery payload is not valid JSON") from None
    digest = hmac.new(secret, body, hashlib.sha256).hexdigest()
    connection = http.client.HTTPConnection(host, int(port_text), timeout=5)
    try:
        connection.request(
            "POST",
            "/forgejo/webhook",
            body=body,
            headers={
                "Content-Type": "application/json",
                "X-Forgejo-Delivery": delivery_id,
                "X-Forgejo-Event": event_name,
                "X-Forgejo-Signature": digest,
            },
        )
        response = connection.getresponse()
        response_body = response.read(64 * 1024 + 1)
        if len(response_body) > 64 * 1024:
            raise RuntimeError("receiver redelivery response exceeded the local limit")
        try:
            value = json.loads(response_body) if response_body else {}
        except json.JSONDecodeError:
            raise RuntimeError("receiver redelivery response was not JSON") from None
        if not isinstance(value, dict):
            raise RuntimeError("receiver redelivery response was not an object")
        return response.status, value
    except (OSError, http.client.HTTPException):
        raise RuntimeError("receiver redelivery request failed") from None
    finally:
        connection.close()


def hook_task_readback(database: Path, hook_id: int) -> list[dict[str, Any]]:
    if not database.is_file():
        raise RuntimeError("local Forgejo SQLite database is missing")
    uri = database.resolve().as_uri() + "?mode=ro"
    try:
        with sqlite3.connect(uri, uri=True, timeout=2.0) as db:
            rows = db.execute(
                "SELECT id, uuid, event_type, is_delivered, is_succeed, response_content "
                "FROM hook_task WHERE hook_id = ? ORDER BY id ASC",
                (hook_id,),
            ).fetchall()
    except sqlite3.Error:
        raise RuntimeError("read-only Forgejo webhook-task readback failed") from None
    tasks: list[dict[str, Any]] = []
    for task_id, delivery_uuid, event_type, delivered, succeeded, response_content in rows:
        try:
            response = json.loads(response_content) if response_content else {}
        except json.JSONDecodeError:
            response = {}
        status = response.get("status") if isinstance(response, dict) else None
        tasks.append(
            {
                "task_id": task_id,
                "uuid": delivery_uuid,
                "event_type": event_type,
                "is_delivered": delivered == 1,
                "is_succeed": succeeded == 1,
                "response_status": status if isinstance(status, int) else None,
            }
        )
    return tasks


def wait_for_hook_tasks(database: Path, hook_id: int, expected: int, timeout: float = 10.0) -> list[dict[str, Any]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        tasks = hook_task_readback(database, hook_id)
        if len(tasks) >= expected and all(
            task["is_delivered"] and task["is_succeed"] and task["response_status"] == 202
            for task in tasks[:expected]
        ):
            return tasks
        time.sleep(0.1)
    raise RuntimeError("Forgejo did not record successful HTTP 202 webhook deliveries")


def hook_deactivation_patch(
    event_selectors: tuple[str, ...] = HOOK_EVENT_SELECTORS,
) -> dict[str, Any]:
    """Disable the hook without letting Forgejo reset omitted event fields."""
    effective_hook_events(event_selectors)
    return {"active": False, "events": list(event_selectors)}


def hook_readback_record(
    hook_id: int | None,
    api_readback: dict[str, Any] | None,
    expected_target: str | None,
    expected_event_selectors: tuple[str, ...] = HOOK_EVENT_SELECTORS,
) -> dict[str, Any]:
    readback = api_readback if isinstance(api_readback, dict) else {}
    readback_id = readback.get("id")
    if not isinstance(readback_id, int) or isinstance(readback_id, bool):
        readback_id = None
    config = readback.get("config")
    config = config if isinstance(config, dict) else {}
    raw_events = readback.get("events")
    events_readback_valid = isinstance(raw_events, list) and all(
        isinstance(event, str) for event in raw_events
    )
    events = (
        [event for event in raw_events if isinstance(event, str)]
        if isinstance(raw_events, list)
        else None
    )
    hook_identity_matches = hook_id is not None and readback_id == hook_id
    readback_active = readback.get("active")
    active_after_run = (
        readback_active if hook_identity_matches and isinstance(readback_active, bool) else None
    )
    deactivation_confirmed = hook_identity_matches and readback_active is False
    hook_type = readback.get("type") if isinstance(readback.get("type"), str) else None
    target = config.get("url") if isinstance(config.get("url"), str) else None
    content_type = (
        config.get("content_type") if isinstance(config.get("content_type"), str) else None
    )
    configuration_matches = bool(
        hook_identity_matches
        and hook_type == "gitea"
        and target == expected_target
        and content_type == "json"
        and events_readback_valid
        and events is not None
        and len(events) == len(effective_hook_events(expected_event_selectors))
        and set(events) == effective_hook_events(expected_event_selectors)
    )
    return {
        "hook_id": hook_id,
        "api_hook_id": readback_id,
        "type": hook_type,
        "events": events,
        "events_readback_valid": events_readback_valid,
        "target": target,
        "content_type": content_type,
        "configuration_matches_expected": configuration_matches,
        "active_after_run": active_after_run,
        "deactivation_readback_confirmed": (
            deactivation_confirmed if hook_id is not None else None
        ),
        "secret_value_recorded": False,
    }


def private_config_with_test_allowlist(state: Path) -> None:
    config = state / "custom/conf/app.ini"
    flags = os.O_RDWR | os.O_APPEND | os.O_CLOEXEC
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    fd = os.open(config, flags)
    try:
        metadata = os.fstat(fd)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or metadata.st_nlink != 1
            or metadata.st_mode & 0o077
        ):
            raise RuntimeError("private Forgejo config has unsafe ownership or permissions")
        os.lseek(fd, 0, os.SEEK_SET)
        existing = bytearray()
        while chunk := os.read(fd, 65536):
            existing.extend(chunk)
            if len(existing) > 1024 * 1024:
                raise RuntimeError("private Forgejo config exceeds the local E2E size limit")
        if re.search(rb"(?m)^\s*\[webhook\]\s*$", existing, re.IGNORECASE):
            raise RuntimeError("private Forgejo config already contains a webhook section")
        addition = b"\n[webhook]\nALLOWED_HOST_LIST = 127.0.0.1\n"
        view = memoryview(addition)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                raise OSError("short write while appending the test-only webhook allowlist")
            view = view[written:]
        os.fchmod(fd, 0o600)
        os.fsync(fd)
    finally:
        os.close(fd)


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
    if shutil.which("rustc") is None or shutil.which("bwrap") is None:
        raise RuntimeError("rustc and Bubblewrap are required for the isolated local source build")
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
    private_config_with_test_allowlist(state)
    evidence_dir.mkdir(mode=0o700, parents=True)
    tokens: list[str] = []
    server_log = (state / "log/forgejo-server.log").open("wb")
    receiver_log = (state / "log/webhook-receiver.log").open("wb")
    server: subprocess.Popen | None = None
    receiver: subprocess.Popen | None = None
    receiver_port: int | None = None
    receiver_bin: Path | None = None
    reconcile_bin: Path | None = None
    build_provenance: dict[str, Any] | None = None
    server_stop: dict[str, Any] | None = None
    receiver_stop: dict[str, Any] | None = None
    receiver_stop_history: list[dict[str, Any]] = []
    owner_api: helpers.LocalForgejoApi | None = None
    hook_id: int | None = None
    hook_api_readback: dict[str, Any] | None = None
    hook_disabled = False
    cleanup_failure: str | None = None
    stage = "isolated_build"
    failure: str | None = None
    safe_result: dict[str, Any] | None = None
    journal_path = state / "data/webhook-inbox.jsonl"
    expected_hook_url: str | None = None
    issue_number: int | None = None
    pull_request_number: int | None = None
    receiver_address: str | None = None
    reader_token: str | None = None
    reader_scopes = "read:user,read:repository,read:issue"
    reader_account: dict[str, Any] | None = None
    permissions: dict[str, Any] | None = None
    webhook_secret: str | None = None
    open_event: dict[str, Any] | None = None
    closed_event: dict[str, Any] | None = None
    edited_events: list[dict[str, Any]] | None = None
    pull_request_open_event: dict[str, Any] | None = None
    pull_request_closed_event: dict[str, Any] | None = None
    open_reconciliation: dict[str, Any] | None = None
    closed_reconciliation: dict[str, Any] | None = None
    edited_reconciliations: list[dict[str, Any]] | None = None
    pull_request_open_reconciliation: dict[str, Any] | None = None
    pull_request_closed_reconciliation: dict[str, Any] | None = None
    invalid_result: dict[str, Any] | None = None
    invalid_status: int | None = None
    duplicate_replay: dict[str, Any] | None = None
    hook_tasks: list[dict[str, Any]] = []

    try:
        receiver_bin, reconcile_bin, build_provenance = helpers.build_local_binaries(
            repo, state, evidence_dir
        )
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
        bootstrap_token = helpers.create_access_token(
            forgejo, state, OWNER, "emitted-webhook-bootstrap", "write:user,write:repository"
        )
        tokens.append(bootstrap_token)
        bootstrap_api = helpers.LocalForgejoApi(bootstrap_token)
        created_repo, _ = bootstrap_api.request(
            "POST",
            "/user/repos",
            {
                "name": REPOSITORY,
                "description": "Private local Forgejo-emitted webhook reconciliation E2E",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if created_repo.get("private") is not True or created_repo.get("full_name") != f"{OWNER}/{REPOSITORY}":
            raise RuntimeError("Forgejo did not create the expected private repository")
        tokens.remove(bootstrap_token)
        bootstrap_api = None
        bootstrap_token = None

        owner_token = helpers.create_access_token(
            forgejo,
            state,
            OWNER,
            "emitted-webhook-owner",
            "read:user,read:repository,read:issue,write:repository,write:issue",
        )
        tokens.append(owner_token)
        owner_api = helpers.LocalForgejoApi(owner_token)
        helpers.admin_command(
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
        reader_token = helpers.create_access_token(
            forgejo, state, READER, "emitted-webhook-read-only", reader_scopes
        )
        tokens.append(reader_token)
        reader_api = helpers.LocalForgejoApi(reader_token)
        reader_account, _ = reader_api.request("GET", "/user")
        if reader_account.get("login") != READER or reader_account.get("is_admin") is not False:
            raise RuntimeError("Forgejo did not confirm the API reader is a non-admin")
        owner_api.request(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{READER}",
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
            raise RuntimeError("Forgejo did not confirm private read-only repository permissions")

        stage = "start_receiver_and_create_hook"
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
                "events": ["issues", "pull_request"],
                "active": True,
            },
        )
        hook_id = hook_response.get("id")
        if not isinstance(hook_id, int) or hook_id < 1 or hook_response.get("active") is not True:
            raise RuntimeError("Forgejo did not create the expected active webhook")

        stage = "deliver_opened_issue_event"
        created_issue, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {"title": "Forgejo-emitted reconciliation", "body": "The receiver must journal this real event."},
        )
        issue_number = created_issue.get("number")
        if not isinstance(issue_number, int) or isinstance(issue_number, bool) or issue_number < 1:
            raise RuntimeError("Forgejo did not return the expected created issue number")
        open_records = wait_for_journal_count(journal_path, 1)
        first_open_candidate = open_records[0]
        if not isinstance(first_open_candidate.get("payload"), dict):
            raise RuntimeError("Forgejo delivery journal did not contain a payload")
        open_event = parse_live_delivery_record(
            first_open_candidate,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
            expected_action="opened",
        )

        stage = "deliver_closed_issue_event"
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
            {"state": "closed"},
        )
        all_records = wait_for_journal_count(journal_path, 2)
        if len(all_records) != 2:
            raise RuntimeError("Forgejo delivered an unexpected number of issue events")
        closed_event = parse_live_delivery_record(
            all_records[1],
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
            expected_action="closed",
        )
        if open_event["delivery_id"] == closed_event["delivery_id"]:
            raise RuntimeError("Forgejo reused a delivery ID across distinct issue events")
        if open_event["sequence"] >= closed_event["sequence"]:
            raise RuntimeError("receiver journal sequence did not preserve issue transition order")

        stage = "deliver_edited_closed_issue_event"
        edited_title = "Forgejo-emitted reconciliation (edited)"
        edited_body = "Editing a closed issue emits a notification; reconcile from Forgejo's current API state."
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}",
            {"title": edited_title, "body": edited_body},
        )
        records_after_issue_edit = wait_for_journal_count(journal_path, 4)
        if len(records_after_issue_edit) != 4:
            raise RuntimeError("Forgejo delivered an unexpected number of issue events before PR creation")
        edited_events = [
            parse_live_delivery_record(
                record,
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="edited",
            )
            for record in records_after_issue_edit[2:]
        ]
        if (
            len({event["delivery_id"] for event in edited_events}) != 2
            or any(event["payload_state"] != "closed" for event in edited_events)
            or edited_events[0]["sequence"] <= closed_event["sequence"]
            or edited_events[0]["sequence"] >= edited_events[1]["sequence"]
        ):
            raise RuntimeError("Forgejo edited-issue events did not preserve closed state and delivery order")

        stage = "create_pull_request_candidate"
        branch_name = "agent/webhook-e2e"
        main_branch, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main"
        )
        base_sha_at_creation = main_branch.get("commit", {}).get("id")
        if not isinstance(base_sha_at_creation, str) or not COMMIT_SHA_RE.fullmatch(base_sha_at_creation):
            raise RuntimeError("Forgejo main branch readback did not contain a valid commit SHA")
        owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/branches",
            {"new_branch_name": branch_name, "old_branch_name": "main"},
        )
        candidate_content = (
            "# Forgejo webhook E2E\n\n"
            "This file is a real candidate change used to exercise pull-request events.\n"
        ).encode("utf-8")
        owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/contents/webhook-e2e.txt",
            {
                "message": "Add webhook E2E candidate file",
                "content": base64.b64encode(candidate_content).decode("ascii"),
                "branch": branch_name,
            },
        )
        candidate_file, _ = owner_api.request(
            "GET",
            f"/repos/{OWNER}/{REPOSITORY}/contents/webhook-e2e.txt?ref={branch_name}",
        )
        if (
            not isinstance(candidate_file, dict)
            or base64.b64decode(candidate_file.get("content", ""), validate=True)
            != candidate_content
        ):
            raise RuntimeError("Forgejo source readback differs from the PR candidate change")
        created_pull_request, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/pulls",
            {
                "title": "Exercise emitted pull-request webhooks",
                "body": "Verify that the live receiver journals and reconciles PR transitions.",
                "head": branch_name,
                "base": "main",
            },
        )
        pull_request_number = created_pull_request.get("number")
        if (
            not isinstance(pull_request_number, int)
            or isinstance(pull_request_number, bool)
            or pull_request_number < 1
        ):
            raise RuntimeError("Forgejo did not return the expected pull-request number")
        pull_request_path = f"/repos/{OWNER}/{REPOSITORY}/pulls/{pull_request_number}"
        opened_pull_request, _ = owner_api.request("GET", pull_request_path)
        opened_head_sha = opened_pull_request.get("head", {}).get("sha")
        opened_base_sha = opened_pull_request.get("base", {}).get("sha")
        if (
            opened_pull_request.get("state") != "open"
            or opened_pull_request.get("merged") is not False
            or not isinstance(opened_head_sha, str)
            or not COMMIT_SHA_RE.fullmatch(opened_head_sha)
            or opened_head_sha == base_sha_at_creation
            or opened_base_sha != base_sha_at_creation
        ):
            raise RuntimeError("Forgejo PR API readback did not confirm the candidate and base commits")

        stage = "deliver_opened_pull_request_event"
        records_after_open_pr = wait_for_journal_count(journal_path, 5)
        if len(records_after_open_pr) != 5:
            raise RuntimeError("Forgejo delivered an unexpected event before PR close")
        pull_request_open_event = parse_live_pull_request_delivery_record(
            records_after_open_pr[4],
            owner=OWNER,
            repository=REPOSITORY,
            pull_request_number=pull_request_number,
            expected_action="opened",
        )
        if (
            pull_request_open_event["payload_state"] != "open"
            or pull_request_open_event["payload_merged"] is not False
            or pull_request_open_event["head_sha"] != opened_head_sha
            or pull_request_open_event["base_sha"] != opened_base_sha
        ):
            raise RuntimeError("emitted opened-PR payload differs from Forgejo's PR readback")

        stage = "deliver_closed_pull_request_event"
        owner_api.request("PATCH", pull_request_path, {"state": "closed"})
        all_records = wait_for_journal_count(journal_path, 6)
        if len(all_records) != 6:
            raise RuntimeError("Forgejo delivered an unexpected number of issue and PR events")
        pull_request_closed_event = parse_live_pull_request_delivery_record(
            all_records[5],
            owner=OWNER,
            repository=REPOSITORY,
            pull_request_number=pull_request_number,
            expected_action="closed",
        )
        if (
            pull_request_open_event["delivery_id"] == pull_request_closed_event["delivery_id"]
            or pull_request_open_event["sequence"] >= pull_request_closed_event["sequence"]
            or pull_request_closed_event["payload_state"] != "closed"
            or pull_request_closed_event["payload_merged"] is not False
            or pull_request_closed_event["head_sha"] != opened_head_sha
            or pull_request_closed_event["base_sha"] != opened_base_sha
        ):
            raise RuntimeError("Forgejo PR transition events do not preserve the same candidate commits")

        stage = "read_forgejo_delivery_tasks"
        hook_tasks = wait_for_hook_tasks(state / "data/forgejo.db", hook_id, 6)
        all_events = [
            open_event,
            closed_event,
            *edited_events,
            pull_request_open_event,
            pull_request_closed_event,
        ]
        if len(hook_tasks) != 6:
            raise RuntimeError("Forgejo recorded an unexpected number of webhook task events")
        validate_hook_task_bindings(hook_tasks, all_events)

        stage = "redeliver_emitted_event_after_receiver_restart"
        journal_bytes_before_replay = journal_path.read_bytes()
        records_before_replay = read_journal_records(journal_path)
        if len(records_before_replay) != 6 or records_before_replay != all_records:
            raise RuntimeError("receiver journal changed before duplicate redelivery")
        receiver_stop = kill_process_with_sigkill(receiver)
        addresses_after_first_stop = helpers.listener_addresses_for_port(receiver_port)
        receiver_stop_history.append(
            {
                "phase": "sigkill_before_duplicate_redelivery",
                "stop": receiver_stop,
                "listener_addresses_after_stop": addresses_after_first_stop,
                "listener_checked": True,
            }
        )
        if (
            not receiver_stop["stopped"]
            or receiver_stop["stop_signal"] != "SIGKILL"
            or addresses_after_first_stop
        ):
            raise RuntimeError("receiver was not stopped by SIGKILL before duplicate redelivery")
        receiver = None
        receiver = subprocess.Popen(
            [str(receiver_bin), receiver_address, str(secret_path), str(journal_path)],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=receiver_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        helpers.wait_for_listener(receiver, receiver_port)
        first_envelope = all_records[0].get("payload")
        if not isinstance(first_envelope, dict) or not isinstance(first_envelope.get("payload"), dict):
            raise RuntimeError("captured Forgejo delivery payload is invalid")
        replay_status, replay_response = send_redelivered_webhook(
            receiver_address,
            webhook_secret.encode("ascii"),
            open_event["delivery_id"],
            open_event["event"],
            first_envelope["payload"],
        )
        records_after_replay = wait_for_journal_count(journal_path, 6)
        journal_bytes_after_replay = journal_path.read_bytes()
        if len(records_after_replay) != 6:
            raise RuntimeError("duplicate Forgejo redelivery changed the journal size")
        validate_duplicate_delivery_replay(
            open_event,
            replay_status,
            replay_response,
            records_before_replay,
            records_after_replay,
        )
        validate_journal_snapshot_unchanged(
            journal_bytes_before_replay,
            journal_bytes_after_replay,
        )
        duplicate_replay = {
            "delivery_id": open_event["delivery_id"],
            "sequence": open_event["sequence"],
            "http_status": replay_status,
            "response": replay_response,
            "journal_records_before": len(records_before_replay),
            "journal_records_after": len(records_after_replay),
            "journal_bytes_before": len(journal_bytes_before_replay),
            "journal_bytes_after": len(journal_bytes_after_replay),
            "journal_sha256_before": hashlib.sha256(journal_bytes_before_replay).hexdigest(),
            "journal_sha256_after": hashlib.sha256(journal_bytes_after_replay).hexdigest(),
            "receiver_restarted_with_same_journal": True,
            "payload_source": "verified Forgejo-emitted receiver journal envelope",
            "payload_encoding": "JSON re-serialized from the journaled object; byte-for-byte original request body is not retained",
        }
        helpers.write_safe_json(
            evidence_dir / "duplicate-delivery-replay.json", duplicate_replay, tokens
        )

        stage = "reconcile_issue_and_pull_request_events"
        open_status, open_reconciliation = helpers.run_reconcile(
            reconcile_bin, state, journal_path, open_event["delivery_id"], reader_token
        )
        closed_status, closed_reconciliation = helpers.run_reconcile(
            reconcile_bin, state, journal_path, closed_event["delivery_id"], reader_token
        )
        edited_reconciliations = []
        edited_statuses = []
        for event in edited_events:
            status, result = helpers.run_reconcile(
                reconcile_bin, state, journal_path, event["delivery_id"], reader_token
            )
            edited_statuses.append(status)
            edited_reconciliations.append(result)
        pull_request_open_status, pull_request_open_reconciliation = helpers.run_reconcile(
            reconcile_bin,
            state,
            journal_path,
            pull_request_open_event["delivery_id"],
            reader_token,
        )
        pull_request_closed_status, pull_request_closed_reconciliation = helpers.run_reconcile(
            reconcile_bin,
            state,
            journal_path,
            pull_request_closed_event["delivery_id"],
            reader_token,
        )
        for event, status, result in (
            (open_event, open_status, open_reconciliation),
            (closed_event, closed_status, closed_reconciliation),
        ):
            validate_reconciliation_delivery_binding(event, result)
            if (
                status != 0
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or result.get("resource", {}).get("number") != issue_number
                or result.get("resource", {}).get("owner") != OWNER
                or result.get("resource", {}).get("repository") != REPOSITORY
                or result.get("canonical_state", {}).get("state") != "closed"
            ):
                raise RuntimeError("reconciliation did not return current Forgejo issue state")
        for event, status, result in zip(
            edited_events, edited_statuses, edited_reconciliations, strict=True
        ):
            validate_reconciliation_delivery_binding(event, result)
            if (
                status != 0
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or result.get("resource", {}).get("number") != issue_number
                or result.get("resource", {}).get("owner") != OWNER
                or result.get("resource", {}).get("repository") != REPOSITORY
                or result.get("canonical_state", {}).get("state") != "closed"
            ):
                raise RuntimeError("reconciliation did not return current Forgejo state for an edited issue")
        if (
            open_event["payload_state"] != "open"
            or closed_event["payload_state"] != "closed"
            or any(event["payload_state"] != "closed" for event in edited_events)
        ):
            raise RuntimeError("Forgejo's emitted payloads did not represent the expected state transitions")
        final_pull_request, _ = owner_api.request("GET", pull_request_path)
        final_head_sha = final_pull_request.get("head", {}).get("sha")
        final_base_sha = final_pull_request.get("base", {}).get("sha")
        if (
            final_pull_request.get("state") != "closed"
            or final_pull_request.get("merged") is not False
            or final_head_sha != opened_head_sha
            or final_base_sha != opened_base_sha
        ):
            raise RuntimeError("Forgejo API readback did not confirm the closed, unmerged PR candidate")
        for event, status, result in (
            (pull_request_open_event, pull_request_open_status, pull_request_open_reconciliation),
            (pull_request_closed_event, pull_request_closed_status, pull_request_closed_reconciliation),
        ):
            validate_reconciliation_delivery_binding(event, result)
            canonical = result.get("canonical_state", {})
            if (
                status != 0
                or result.get("verdict") != "current_state_read"
                or result.get("upstream_read_attempts") != 1
                or result.get("resource", {}).get("kind") != "pull_request"
                or result.get("resource", {}).get("number") != pull_request_number
                or result.get("resource", {}).get("owner") != OWNER
                or result.get("resource", {}).get("repository") != REPOSITORY
                or canonical.get("state") != "closed"
                or canonical.get("merged") is not False
                or canonical.get("head_sha") != final_head_sha
                or canonical.get("base_sha") != final_base_sha
            ):
                raise RuntimeError("reconciliation did not return the current canonical Forgejo PR state")

        stage = "refuse_invalid_reader_credential"
        invalid_token = "invalid-" + secrets.token_urlsafe(24)
        tokens.append(invalid_token)
        invalid_status, invalid_result = helpers.run_reconcile(
            reconcile_bin, state, journal_path, open_event["delivery_id"], invalid_token
        )
        validate_reconciliation_delivery_binding(open_event, invalid_result)
        if (
            invalid_status == 0
            or invalid_result.get("verdict") != "inconclusive"
            or invalid_result.get("error", {}).get("code") != "upstream_unauthorized"
            or invalid_result.get("error", {}).get("retryable") is not False
            or invalid_result.get("upstream_read_attempts") != 1
            or "canonical_state" in invalid_result
        ):
            raise RuntimeError("invalid Forgejo credential did not remain inconclusive")
        post_issue, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        )
        if post_issue.get("state") != "closed":
            raise RuntimeError("Forgejo API readback did not confirm the issue is closed")
        if post_issue.get("title") != edited_title or post_issue.get("body") != edited_body:
            raise RuntimeError("Forgejo API readback did not confirm the edited issue content")

        stage = "disable_test_webhook"
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
            hook_deactivation_patch(),
        )
        hook_api_readback, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
        )
        hook_readback = hook_readback_record(hook_id, hook_api_readback, expected_hook_url)
        hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
        if not hook_disabled:
            raise RuntimeError("Forgejo hook readback did not confirm deactivation")
        if not hook_readback["configuration_matches_expected"]:
            raise RuntimeError("Forgejo hook readback differs from the created hook configuration")

        source_head = subprocess.run(
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
                "binary_sha256": helpers.sha256_file(forgejo),
                "listener": "127.0.0.1:3130",
                "test_only_webhook_allowlist": "127.0.0.1",
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
            "webhook": {
                "emitter": "Forgejo v17 actual webhook task",
                **hook_readback,
                "delivery_events": all_events,
                "forgejo_task_readback": hook_tasks,
                "duplicate_replay": duplicate_replay,
                "secret_recorded": False,
            },
            "issue": {
                "number": issue_number,
                "state_after_run": post_issue.get("state"),
                "old_open_event_payload_state": open_event["payload_state"],
                "edited_events": edited_events,
                "reconciled_current_state": open_reconciliation.get("canonical_state", {}).get("state"),
                "edited_event_reconciliations": edited_reconciliations,
            },
            "pull_request": {
                "number": pull_request_number,
                "head_sha": final_head_sha,
                "base_sha": final_base_sha,
                "state_after_run": final_pull_request.get("state"),
                "merged_after_run": final_pull_request.get("merged"),
                "opened_event_payload_state": pull_request_open_event["payload_state"],
                "closed_event_payload_state": pull_request_closed_event["payload_state"],
                "reconciled_current_state": pull_request_closed_reconciliation.get(
                    "canonical_state", {}
                ).get("state"),
            },
            "reconciliation": {
                "old_open_event": open_reconciliation,
                "new_closed_event": closed_reconciliation,
                "edited_closed_events": edited_reconciliations,
                "old_open_pull_request_event": pull_request_open_reconciliation,
                "new_closed_pull_request_event": pull_request_closed_reconciliation,
            },
            "invalid_credential": {
                "exit_status": invalid_status,
                "result": invalid_result,
                "credential_material_recorded": False,
            },
            "source_head_commit": source_head,
            "limitations": [
                "The webhook allowlist is loopback-only and belongs to this disposable local instance.",
                "Forgejo, receiver and CLI run as the current host user; hostile candidate code is not executed.",
                "No LLM, AgentOS, RAG or memory service was used; no comparative productivity or cost result is measured.",
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
                    hook_deactivation_patch(),
                )
                hook_api_readback, _ = owner_api.request(
                    "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
                )
                hook_readback = hook_readback_record(
                    hook_id, hook_api_readback, expected_hook_url
                )
                hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
                if not hook_disabled:
                    cleanup_failure = "hook_deactivation_not_confirmed"
            except Exception as error:
                cleanup_failure = type(error).__name__
        receiver_stop = helpers.stop_process(receiver)
        receiver_stop_history.append(
            {
                "phase": "final_cleanup",
                "stop": receiver_stop,
                "listener_addresses_after_stop": (
                    helpers.listener_addresses_for_port(receiver_port)
                    if receiver is not None and receiver_port is not None
                    else []
                ),
                "listener_checked": receiver is not None and receiver_port is not None,
            }
        )
        server_stop = helpers.stop_process(server)
        receiver_log.close()
        server_log.close()

    assert receiver_stop is not None and server_stop is not None
    cleanup_readback = {
        "forgejo": {
            "expected_address": "127.0.0.1:3130",
            "stop": server_stop,
            "listener_addresses_after_stop": (
                helpers.listener_addresses_for_port(3130) if server is not None else []
            ),
            "listener_checked": server is not None,
        },
        "receiver": {
            "expected_address": f"127.0.0.1:{receiver_port}" if receiver_port is not None else None,
            "stop": receiver_stop,
            "stops": receiver_stop_history,
            "listener_addresses_after_stop": (
                helpers.listener_addresses_for_port(receiver_port)
                if receiver is not None and receiver_port is not None
                else []
            ),
            "listener_checked": receiver is not None and receiver_port is not None,
        },
        "hook_disabled_before_server_stop": hook_disabled,
        "cleanup_failure": cleanup_failure,
    }
    cleanup_readback["tracked_processes_exited"] = bool(
        server_stop["stopped"]
        and all(entry["stop"]["stopped"] for entry in receiver_stop_history)
    )
    cleanup_readback["expected_tcp_listeners_absent"] = (
        not cleanup_readback["forgejo"]["listener_addresses_after_stop"]
        and not cleanup_readback["receiver"]["listener_addresses_after_stop"]
        and all(not entry["listener_addresses_after_stop"] for entry in receiver_stop_history)
    )
    helpers.write_safe_json(evidence_dir / "port-cleanup-readback.json", cleanup_readback, tokens)
    helpers.write_safe_json(
        evidence_dir / "hook-readback.json",
        hook_readback_record(hook_id, hook_api_readback, expected_hook_url),
        tokens,
    )
    helpers.write_safe_json(evidence_dir / "forgejo-hook-tasks.json", hook_tasks, tokens)

    if failure is not None or cleanup_failure is not None:
        helpers.write_safe_json(
            evidence_dir / "failure.json",
            {
                "verdict": "failed",
                "stage": stage,
                "error_type": failure,
                "cleanup_failure": cleanup_failure,
            },
            tokens,
        )
        helpers.write_checksums(evidence_dir)
        raise RuntimeError(f"Forgejo emitted webhook E2E failed at {stage}") from None

    assert safe_result is not None
    assert build_provenance is not None
    assert open_event is not None and closed_event is not None and edited_events is not None
    assert pull_request_open_event is not None and pull_request_closed_event is not None
    assert open_reconciliation is not None and closed_reconciliation is not None
    assert edited_reconciliations is not None
    assert pull_request_open_reconciliation is not None and pull_request_closed_reconciliation is not None
    assert invalid_status is not None and invalid_result is not None
    if (
        not cleanup_readback["tracked_processes_exited"]
        or not cleanup_readback["expected_tcp_listeners_absent"]
        or not hook_disabled
    ):
        helpers.write_safe_json(
            evidence_dir / "failure.json",
            {"verdict": "failed", "stage": "cleanup_readback", "error_type": "CleanupNotConfirmed"},
            tokens,
        )
        helpers.write_checksums(evidence_dir)
        raise RuntimeError("local Forgejo, receiver or hook did not stop cleanly")

    safe_result["process_cleanup"] = cleanup_readback
    helpers.write_safe_json(evidence_dir / "result.json", safe_result, tokens)
    helpers.write_safe_json(
        evidence_dir / "webhook-events.json",
        [open_event, closed_event, *edited_events, pull_request_open_event, pull_request_closed_event],
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "open-event-reconciliation.json", open_reconciliation, tokens
    )
    helpers.write_safe_json(
        evidence_dir / "closed-event-reconciliation.json", closed_reconciliation, tokens
    )
    helpers.write_safe_json(
        evidence_dir / "edited-closed-event-reconciliations.json", edited_reconciliations, tokens
    )
    helpers.write_safe_json(
        evidence_dir / "pull-request-open-event-reconciliation.json",
        pull_request_open_reconciliation,
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "pull-request-closed-event-reconciliation.json",
        pull_request_closed_reconciliation,
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "invalid-credential-result.json",
        {"exit_status": invalid_status, "result": invalid_result},
        tokens,
    )
    helpers.write_safe_json(
        evidence_dir / "reader-role-readback.json",
        safe_result["repository"]["api_reader"],
        tokens,
    )
    source_files = sorted(
        {
            repo / "Cargo.toml",
            repo / "Cargo.lock",
            Path(__file__).resolve(),
            (SCRIPT_DIR / "run_forgejo_live_webhook_reconcile_e2e.py").resolve(),
            (SCRIPT_DIR / "run_forgejo_live_pr_merge_e2e.py").resolve(),
            (SCRIPT_DIR / "run_forgejo_mission_delivery_process_e2e.py").resolve(),
            (repo / "tests/test_forgejo_emitted_webhook_e2e.py").resolve(),
            *(repo / "src").rglob("*.rs"),
        }
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
                "scripts/run_forgejo_live_webhook_reconcile_emitted.py",
                "scripts/run_forgejo_live_pr_merge_e2e.py",
                "scripts/run_forgejo_mission_delivery_process_e2e.py",
                "tests/test_forgejo_emitted_webhook_e2e.py",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=10,
            check=True,
        ).stdout.splitlines(),
        "source_sha256": {
            str(path.relative_to(repo)): helpers.sha256_file(path) for path in source_files
        },
        "forgejo_binary_sha256": safe_result["forgejo"]["binary_sha256"],
        "built_executable_sha256": {
            "forgejo-webhook-local": helpers.sha256_file(receiver_bin),
            "forgejo-reconcile-local": helpers.sha256_file(reconcile_bin),
        },
        "cargo_build": build_provenance,
        "port_cleanup_readback_sha256": helpers.sha256_file(
            evidence_dir / "port-cleanup-readback.json"
        ),
        "hook_readback_sha256": helpers.sha256_file(evidence_dir / "hook-readback.json"),
        "private_state_dir": str(state),
        "private_state_dir_is_not_in_evidence": True,
        "webhook_secret_recorded": False,
    }
    helpers.write_safe_json(evidence_dir / "provenance.json", provenance, tokens)
    helpers.write_checksums(evidence_dir)
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
