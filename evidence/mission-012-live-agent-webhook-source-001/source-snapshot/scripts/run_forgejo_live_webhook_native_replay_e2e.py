#!/usr/bin/env python3
"""Exercise Forgejo v17's authenticated native webhook replay on loopback."""

from __future__ import annotations

import argparse
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import sqlite3
import subprocess
import sys
import threading
import time
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urljoin, urlsplit
from urllib.request import HTTPCookieProcessor, ProxyHandler, Request, build_opener
from http.cookiejar import CookieJar

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_e2e as helpers  # noqa: E402
import run_forgejo_live_webhook_reconcile_emitted as emitted  # noqa: E402

OWNER = helpers.OWNER
REPOSITORY = helpers.REPOSITORY
ORIGIN = helpers.ORIGIN
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
PASSWORD_RE = re.compile(rb"generated random password is '([^']+)'\s*")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class CountingWebhookProxy:
    """Forward one local webhook path while recording request metadata only."""

    def __init__(self, host: str, port: int, upstream_host: str, upstream_port: int) -> None:
        self.host = host
        self.port = port
        self.upstream_host = upstream_host
        self.upstream_port = upstream_port
        self._records: list[dict[str, Any]] = []
        self._lock = threading.Lock()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *args: Any) -> None:
                return

            def do_POST(self) -> None:
                self._forward()

            def _forward(self) -> None:
                length_text = self.headers.get("Content-Length", "")
                try:
                    length = int(length_text)
                except ValueError:
                    length = -1
                record: dict[str, Any] = {
                    "method": self.command,
                    "path": self.path,
                    "delivery_id": self.headers.get("X-Forgejo-Delivery"),
                    "event_type": self.headers.get("X-Forgejo-Event"),
                    "request_body_length": length if length >= 0 else None,
                    "request_body_sha256": None,
                    "forwarded_status": None,
                }
                if self.command != "POST" or self.path != "/forgejo/webhook":
                    owner._append(record)
                    self._reply(404, b"")
                    return
                if length < 0 or length > 1024 * 1024:
                    owner._append(record)
                    self._reply(413, b"")
                    return
                body = self.rfile.read(length)
                if len(body) != length:
                    owner._append(record)
                    self._reply(400, b"")
                    return
                record["request_body_sha256"] = hashlib.sha256(body).hexdigest()
                hop_by_hop = {
                    "connection",
                    "content-length",
                    "host",
                    "keep-alive",
                    "proxy-authenticate",
                    "proxy-authorization",
                    "te",
                    "trailer",
                    "transfer-encoding",
                    "upgrade",
                }
                forwarded_headers = {
                    key: value
                    for key, value in self.headers.items()
                    if key.lower() not in hop_by_hop
                }
                upstream = http.client.HTTPConnection(
                    owner.upstream_host,
                    owner.upstream_port,
                    timeout=10,
                )
                try:
                    upstream.request("POST", self.path, body=body, headers=forwarded_headers)
                    response = upstream.getresponse()
                    response_body = response.read(1024 * 1024 + 1)
                    if len(response_body) > 1024 * 1024:
                        raise RuntimeError("local webhook response exceeded the proxy limit")
                    record["forwarded_status"] = response.status
                    response_headers = {
                        key.lower(): value
                        for key, value in response.getheaders()
                        if key.lower() not in hop_by_hop and key.lower() != "content-length"
                    }
                except Exception:
                    record["forward_failed"] = True
                    owner._append(record)
                    self._reply(502, b"")
                    return
                finally:
                    upstream.close()
                owner._append(record)
                self.send_response(record["forwarded_status"])
                for key, value in response_headers.items():
                    self.send_header(key, value)
                self.send_header("Content-Length", str(len(response_body)))
                self.send_header("Connection", "close")
                self.end_headers()
                if response_body:
                    self.wfile.write(response_body)
                self.close_connection = True

            def _reply(self, status: int, body: bytes) -> None:
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Connection", "close")
                self.end_headers()
                if body:
                    self.wfile.write(body)
                self.close_connection = True

        self._server = ThreadingHTTPServer((host, port), Handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(
            target=self._server.serve_forever,
            name="forgejo-webhook-counting-proxy",
            daemon=True,
        )

    def start(self) -> None:
        self._thread.start()

    def snapshot(self) -> list[dict[str, Any]]:
        with self._lock:
            return [dict(item) for item in self._records]

    def stop(self) -> dict[str, Any]:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)
        return {"stopped": not self._thread.is_alive()}

    def _append(self, record: dict[str, Any]) -> None:
        with self._lock:
            self._records.append(dict(record))


def canonical_json_sha256(value: Any) -> str:
    """Hash a JSON value by stable semantic serialization, independent of key order."""
    try:
        encoded = json.dumps(
            value,
            ensure_ascii=False,
            allow_nan=False,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
    except (TypeError, ValueError):
        raise ValueError("payload is not a canonicalizable JSON value") from None
    return hashlib.sha256(encoded).hexdigest()


def validate_native_replay_task(
    original: dict[str, Any], replay: dict[str, Any], journal_record: dict[str, Any]
) -> dict[str, Any]:
    """Bind a failed Forgejo task to a successful native replay and receiver receipt."""
    if not all(isinstance(item, dict) for item in (original, replay, journal_record)):
        raise ValueError("native replay evidence must be structured objects")
    original_uuid = original.get("uuid")
    replay_uuid = replay.get("uuid")
    hook_id = original.get("hook_id")
    if (
        not isinstance(original_uuid, str)
        or not original_uuid
        or not isinstance(replay_uuid, str)
        or not replay_uuid
        or original_uuid == replay_uuid
        or not isinstance(hook_id, int)
        or isinstance(hook_id, bool)
        or hook_id < 1
        or replay.get("hook_id") != hook_id
        or original.get("event_type") != "issues"
        or replay.get("event_type") != original.get("event_type")
    ):
        raise ValueError("native replay task identity or webhook scope is invalid")
    if (
        original.get("is_delivered") is not True
        or original.get("is_succeed") is not False
        or type(original.get("response_status")) is not int
        or not 400 <= original["response_status"] < 600
    ):
        raise ValueError("original webhook task is not a recorded failed delivery")
    if (
        replay.get("is_delivered") is not True
        or replay.get("is_succeed") is not True
        or type(replay.get("response_status")) is not int
        or replay["response_status"] != 202
    ):
        raise ValueError("native replay task did not complete with HTTP 202")
    original_payload_sha = original.get("payload_sha256")
    replay_payload_sha = replay.get("payload_sha256")
    original_semantic_sha = original.get("payload_semantic_sha256")
    replay_semantic_sha = replay.get("payload_semantic_sha256")
    if (
        not isinstance(original_payload_sha, str)
        or not SHA256_RE.fullmatch(original_payload_sha)
        or replay_payload_sha != original_payload_sha
        or not isinstance(original_semantic_sha, str)
        or not SHA256_RE.fullmatch(original_semantic_sha)
        or replay_semantic_sha != original_semantic_sha
    ):
        raise ValueError("native replay payload differs from the original failed task")

    sequence = journal_record.get("sequence")
    envelope = journal_record.get("payload")
    if (
        type(sequence) is not int
        or sequence < 1
        or journal_record.get("kind") != "forgejo.webhook.v1.issues"
        or journal_record.get("idempotency_key") != f"forgejo-delivery-v1:{replay_uuid}"
        or not isinstance(envelope, dict)
        or envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "issues"
        or envelope.get("delivery_id") != replay_uuid
        or not isinstance(envelope.get("payload"), dict)
    ):
        raise ValueError("receiver journal does not bind to the native replay task")
    try:
        journal_payload_sha = canonical_json_sha256(envelope["payload"])
    except ValueError:
        raise ValueError("receiver journal payload is not canonical JSON") from None
    if journal_payload_sha != original_semantic_sha:
        raise ValueError("receiver journal payload differs from the Forgejo task payload")
    return {
        "original_uuid": original_uuid,
        "replay_uuid": replay_uuid,
        "new_delivery_id": True,
        "payload_unchanged": True,
        "payload_semantic_sha256": journal_payload_sha,
        "http_status": 202,
        "sequence": sequence,
    }


def validate_native_replay_chain(
    original: dict[str, Any],
    replays: list[dict[str, Any]],
    journal_records: list[dict[str, Any]],
) -> dict[str, Any]:
    """Require repeated manual replays to have distinct UUIDs and ordered receipts."""
    if (
        not isinstance(original, dict)
        or not isinstance(replays, list)
        or not replays
        or not isinstance(journal_records, list)
        or len(replays) != len(journal_records)
    ):
        raise ValueError("native replay chain evidence is incomplete")
    original_uuid = original.get("uuid")
    if not isinstance(original_uuid, str) or not original_uuid:
        raise ValueError("native replay chain original UUID is invalid")
    seen_uuids = {original_uuid}
    deliveries: list[dict[str, Any]] = []
    for expected_sequence, (replay, record) in enumerate(
        zip(replays, journal_records, strict=True), start=1
    ):
        binding = validate_native_replay_task(original, replay, record)
        if binding["replay_uuid"] in seen_uuids:
            raise ValueError("native replay chain contains a repeated delivery UUID")
        if binding["sequence"] != expected_sequence:
            raise ValueError("native replay receipts are missing or out of sequence")
        seen_uuids.add(binding["replay_uuid"])
        deliveries.append(binding)
    return {
        "original_uuid": original_uuid,
        "replay_count": len(deliveries),
        "replay_uuids": [item["replay_uuid"] for item in deliveries],
        "distinct_new_delivery_ids": True,
        "all_payloads_unchanged": True,
        "deliveries": deliveries,
    }


def validate_inactive_hook_replay(
    hook_readback: dict[str, Any],
    replay_http_status: int,
    original: dict[str, Any],
    replay_task: dict[str, Any],
    proxy_records_before: list[dict[str, Any]],
    proxy_records_after: list[dict[str, Any]],
    journal_records_before: list[dict[str, Any]],
    journal_records_after: list[dict[str, Any]],
) -> dict[str, Any]:
    """Bind an accepted replay POST to an inactive, skipped delivery task."""
    if not isinstance(hook_readback, dict) or not all(
        isinstance(item, dict)
        for item in (original, replay_task)
    ):
        raise ValueError("inactive replay evidence must be structured objects")
    hook_id = original.get("hook_id")
    original_uuid = original.get("uuid")
    replay_uuid = replay_task.get("uuid")
    if (
        type(hook_id) is not int
        or hook_id < 1
        or hook_readback.get("id") != hook_id
        or hook_readback.get("active") is not False
        or type(replay_http_status) is not int
        or replay_http_status not in {302, 303}
        or not isinstance(original_uuid, str)
        or not original_uuid
        or not isinstance(replay_uuid, str)
        or not replay_uuid
        or replay_uuid == original_uuid
        or replay_task.get("hook_id") != hook_id
        or replay_task.get("event_type") != original.get("event_type")
    ):
        raise ValueError("inactive native replay route or task identity is invalid")
    if (
        replay_task.get("is_delivered") is not True
        or replay_task.get("is_succeed") is not False
        or type(replay_task.get("response_status")) is not int
        or replay_task["response_status"] != 0
    ):
        raise ValueError("inactive native replay task was not processed as a skipped delivery")
    if (
        not isinstance(original.get("payload_sha256"), str)
        or replay_task.get("payload_sha256") != original.get("payload_sha256")
        or not isinstance(original.get("payload_semantic_sha256"), str)
        or replay_task.get("payload_semantic_sha256")
        != original.get("payload_semantic_sha256")
    ):
        raise ValueError("inactive native replay payload differs from its original task")
    if (
        not isinstance(proxy_records_before, list)
        or not isinstance(proxy_records_after, list)
        or proxy_records_after != proxy_records_before
        or not isinstance(journal_records_before, list)
        or not isinstance(journal_records_after, list)
        or journal_records_after != journal_records_before
    ):
        raise ValueError("inactive replay unexpectedly reached the HTTP proxy or receiver")
    return {
        "replay_http_status": replay_http_status,
        "hook_active": False,
        "task_created": True,
        "task_uuid": replay_uuid,
        "task_marked_delivered": True,
        "task_succeeded": False,
        "forgejo_response_status": 0,
        "payload_unchanged": True,
        "proxy_request_count_before": len(proxy_records_before),
        "proxy_request_count_after": len(proxy_records_after),
        "receiver_journal_count_before": len(journal_records_before),
        "receiver_journal_count_after": len(journal_records_after),
        "receiver_journal_unchanged": True,
    }


def validate_proxy_delivery_attempts(
    proxy_records: list[dict[str, Any]], tasks: list[dict[str, Any]]
) -> dict[str, Any]:
    """Bind every counted HTTP POST and raw body hash to its Forgejo task."""
    if (
        not isinstance(proxy_records, list)
        or not proxy_records
        or not isinstance(tasks, list)
        or len(proxy_records) != len(tasks)
        or not all(isinstance(item, dict) for item in [*proxy_records, *tasks])
    ):
        raise ValueError("proxy requests and Forgejo tasks are incomplete or misaligned")
    seen_delivery_ids: set[str] = set()
    delivery_ids: list[str] = []
    body_hashes: list[str] = []
    for record, task in zip(proxy_records, tasks, strict=True):
        delivery_id = task.get("uuid")
        task_hash = task.get("payload_sha256")
        request_hash = record.get("request_body_sha256")
        task_status = task.get("response_status")
        if (
            not isinstance(delivery_id, str)
            or not delivery_id
            or delivery_id in seen_delivery_ids
            or record.get("method") != "POST"
            or record.get("path") != "/forgejo/webhook"
            or record.get("delivery_id") != delivery_id
            or record.get("event_type") != task.get("event_type")
            or type(task_status) is not int
            or type(record.get("forwarded_status")) is not int
            or record["forwarded_status"] != task_status
            or type(record.get("request_body_length")) is not int
            or record["request_body_length"] < 1
            or not isinstance(task_hash, str)
            or not SHA256_RE.fullmatch(task_hash)
            or request_hash != task_hash
        ):
            raise ValueError("proxy request is not bound to the Forgejo task and exact body hash")
        seen_delivery_ids.add(delivery_id)
        delivery_ids.append(delivery_id)
        body_hashes.append(request_hash)
    return {
        "delivery_count": len(delivery_ids),
        "delivery_ids": delivery_ids,
        "raw_payload_hashes": body_hashes,
        "raw_payload_hashes_match": True,
    }


def local_web_request(
    opener: Any,
    method: str,
    path: str,
    body: bytes | None = None,
    headers: dict[str, str] | None = None,
    *,
    origin: str = ORIGIN,
) -> tuple[int, dict[str, str], bytes]:
    origin = validate_loopback_origin(origin)
    if not isinstance(path, str) or not path.startswith("/") or path.startswith("//"):
        raise ValueError("Forgejo web path must be local and absolute")
    if any(character in path for character in "\r\n"):
        raise ValueError("Forgejo web path contains a control character")
    request = Request(
        origin + path,
        data=body,
        headers=headers or {},
        method=method,
    )
    try:
        with opener.open(request, timeout=5) as response:
            data = response.read(1024 * 1024 + 1)
            if len(data) > 1024 * 1024:
                raise RuntimeError("Forgejo web response exceeded the local E2E limit")
            return response.status, {k.lower(): v for k, v in response.headers.items()}, data
    except HTTPError as error:
        data = error.read(1024 * 1024 + 1)
        if len(data) > 1024 * 1024:
            error.close()
            raise RuntimeError("Forgejo web error response exceeded the local E2E limit") from None
        response_headers = {k.lower(): v for k, v in error.headers.items()}
        status = error.code
        error.close()
        return status, response_headers, data
    except URLError:
        raise RuntimeError("Forgejo local web session request failed") from None


def validate_loopback_origin(origin: str) -> str:
    """Accept only a canonical HTTP origin bound to IPv4 loopback."""
    if not isinstance(origin, str):
        raise ValueError("Forgejo origin must be a local HTTP origin")
    try:
        parsed = urlsplit(origin)
        port = parsed.port
    except ValueError:
        raise ValueError("Forgejo origin must be a local HTTP origin") from None
    if (
        parsed.scheme != "http"
        or parsed.hostname != "127.0.0.1"
        or port is None
        or not 1 <= port <= 65535
        or parsed.path
        or parsed.query
        or parsed.fragment
        or parsed.username is not None
        or parsed.password is not None
        or origin != f"http://127.0.0.1:{port}"
    ):
        raise ValueError("Forgejo origin must be a canonical HTTP loopback origin")
    return origin


def login_web_session(username: str, password: str, *, origin: str = ORIGIN) -> Any:
    origin = validate_loopback_origin(origin)
    cookie_jar = CookieJar()
    opener = build_opener(
        ProxyHandler({}),
        HTTPCookieProcessor(cookie_jar),
        helpers.NoRedirect(),
    )
    status, _, _ = local_web_request(opener, "GET", "/user/login", origin=origin)
    if status != 200:
        raise RuntimeError("Forgejo login page was not available")
    form_body = urlencode({"user_name": username, "password": password, "remember": "on"}).encode()
    status, _, _ = local_web_request(
        opener,
        "POST",
        "/user/login",
        form_body,
        {
            "Content-Type": "application/x-www-form-urlencoded",
            "Origin": origin,
            "Sec-Fetch-Site": "same-origin",
            "Referer": origin + "/user/login",
        },
        origin=origin,
    )
    if status not in {200, 302, 303}:
        raise RuntimeError("Forgejo did not accept the local web login")
    return opener


def hook_task_readback(database: Path, hook_id: int) -> list[dict[str, Any]]:
    uri = database.resolve(strict=True).as_uri() + "?mode=ro"
    try:
        with sqlite3.connect(uri, uri=True, timeout=5) as connection:
            rows = connection.execute(
                "SELECT id, hook_id, uuid, event_type, is_delivered, is_succeed, "
                "payload_content, response_content FROM hook_task "
                "WHERE hook_id = ? ORDER BY id ASC",
                (hook_id,),
            ).fetchall()
    except sqlite3.Error:
        raise RuntimeError("read-only Forgejo hook-task readback failed") from None
    result: list[dict[str, Any]] = []
    for task_id, task_hook_id, uuid, event_type, delivered, succeeded, payload, response in rows:
        try:
            payload_value = json.loads(payload) if isinstance(payload, str) else None
        except json.JSONDecodeError:
            payload_value = None
        if not isinstance(payload_value, dict):
            raise RuntimeError("Forgejo hook task does not contain a JSON object payload")
        try:
            response_value = json.loads(response) if response else {}
        except json.JSONDecodeError:
            response_value = {}
        status = response_value.get("status") if isinstance(response_value, dict) else None
        result.append(
            {
                "task_id": task_id,
                "hook_id": task_hook_id,
                "uuid": uuid,
                "event_type": str(event_type),
                "is_delivered": delivered == 1,
                "is_succeed": succeeded == 1,
                "payload_sha256": hashlib.sha256(str(payload).encode("utf-8")).hexdigest(),
                "payload_semantic_sha256": canonical_json_sha256(payload_value),
                "response_status": status if type(status) is int else None,
            }
        )
    return result


def wait_for_task_count(
    database: Path,
    hook_id: int,
    expected_count: int,
    final_status: int,
    timeout: float = 10.0,
) -> list[dict[str, Any]]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        tasks = hook_task_readback(database, hook_id)
        if (
            len(tasks) == expected_count
            and tasks[-1]["is_delivered"]
            and tasks[-1]["response_status"] == final_status
        ):
            return tasks
        time.sleep(0.1)
    raise RuntimeError("Forgejo hook-task readback did not reach the expected delivery state")


def start_receiver(
    binary: Path,
    state: Path,
    log_handle: Any,
    address: str,
    secret_path: Path,
    journal_path: Path,
) -> subprocess.Popen[bytes]:
    process = subprocess.Popen(
        [str(binary), address, str(secret_path), str(journal_path)],
        cwd=state,
        env=helpers.private_command_environment(state),
        stdout=log_handle,
        stderr=subprocess.STDOUT,
        close_fds=True,
    )
    helpers.wait_for_listener(process, int(address.rpartition(":")[2]))
    return process


def safe_hook_readback(owner_api: Any, hook_id: int) -> dict[str, Any]:
    hook, _ = owner_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}")
    config = hook.get("config") if isinstance(hook, dict) else None
    config = config if isinstance(config, dict) else {}
    return {
        "id": hook.get("id") if isinstance(hook, dict) else None,
        "active": hook.get("active") if isinstance(hook, dict) else None,
        "events": hook.get("events") if isinstance(hook, dict) else None,
        "target_url": config.get("url"),
        "secret_value_recorded": False,
    }


def source_hashes(repo: Path) -> dict[str, str]:
    paths = (
        "scripts/run_forgejo_live_webhook_native_replay_e2e.py",
        "tests/test_forgejo_webhook_native_replay_e2e.py",
        "scripts/run_forgejo_live_webhook_reconcile_e2e.py",
        "scripts/run_forgejo_live_webhook_reconcile_emitted.py",
        "scripts/run_forgejo_live_pr_merge_e2e.py",
    )
    return {
        name: helpers.sha256_file(repo / name)
        for name in paths
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    forgejo_arg = Path(args.forgejo_binary)
    if forgejo_arg.is_symlink():
        raise RuntimeError("Forgejo binary must not be a symlink")
    forgejo = forgejo_arg.resolve(strict=True)
    repo = Path(args.repo_root).resolve(strict=True)
    state = Path(args.state_dir)
    evidence_dir = Path(args.evidence_dir).resolve()
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("state dir must be a new direct child of /tmp")
    if evidence_dir.exists():
        raise RuntimeError("refusing to overwrite an existing evidence directory")
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
    emitted.private_config_with_test_allowlist(state)
    evidence_dir.mkdir(mode=0o700, parents=True)
    forgejo_log = (state / "log/forgejo-server.log").open("wb")
    first_receiver_log = (state / "log/receiver-reject.log").open("wb")
    replay_receiver_log = (state / "log/receiver-replay.log").open("wb")
    server: subprocess.Popen[bytes] | None = None
    receiver: subprocess.Popen[bytes] | None = None
    receiver_port: int | None = None
    receiver_bin: Path | None = None
    proxy_port: int | None = None
    proxy: CountingWebhookProxy | None = None
    proxy_stop: dict[str, Any] | None = None
    proxy_listeners: list[str] = []
    build_info: dict[str, Any] | None = None
    owner_api: Any = None
    owner_token: str | None = None
    owner_password: str | None = None
    hook_id: int | None = None
    hook_deactivated = False
    server_stop: dict[str, Any] | None = None
    receiver_stop_history: list[dict[str, Any]] = []
    cleanup_failure: str | None = None
    stage = "isolated_build"
    failure: str | None = None
    result: dict[str, Any] | None = None
    task_rows: list[dict[str, Any]] = []
    journal_records: list[dict[str, Any]] = []
    journal_path = state / "data/native-replay-inbox.jsonl"
    database = state / "data/forgejo.db"
    webhook_secret: str | None = None
    invalid_secret: str | None = None
    receiver_address: str | None = None
    replay_chain_summary: dict[str, Any] | None = None
    inactive_replay_summary: dict[str, Any] | None = None
    proxy_records: list[dict[str, Any]] = []

    try:
        receiver_bin, _, build_info = helpers.build_local_binaries(repo, state, evidence_dir)
        stage = "start_forgejo"
        server = subprocess.Popen(
            [str(forgejo), "--work-path", str(state), "--config", str(state / "custom/conf/app.ini"), "web"],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=forgejo_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        helpers.forgejo_helpers.wait_for_health(server)
        helpers.forgejo_helpers.assert_forgejo_listener(server)

        stage = "create_private_repository_and_owner_session"
        password_output = helpers.admin_command(
            forgejo,
            state,
            [
                "admin", "user", "create", "--username", OWNER,
                "--random-password", "--email", f"{OWNER}@local.invalid",
                "--admin", "--must-change-password=false",
            ],
        )
        matches = PASSWORD_RE.findall(password_output)
        if len(matches) != 1:
            raise RuntimeError("Forgejo did not return one generated owner password")
        owner_password = matches[0].decode("utf-8")
        owner_token = helpers.create_access_token(
            forgejo,
            state,
            OWNER,
            "native-webhook-replay-e2e",
            "read:user,write:user,read:repository,write:repository,write:issue",
        )
        owner_api = helpers.LocalForgejoApi(owner_token)
        owner_readback, _ = owner_api.request("GET", "/user")
        if owner_readback.get("login") != OWNER or owner_readback.get("is_admin") is not True:
            raise RuntimeError("Forgejo did not confirm the local webhook owner account")
        repo_readback, _ = owner_api.request(
            "POST",
            "/user/repos",
            {
                "name": REPOSITORY,
                "description": "Private local webhook native replay verification",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if repo_readback.get("full_name") != f"{OWNER}/{REPOSITORY}" or repo_readback.get("private") is not True:
            raise RuntimeError("Forgejo did not create the expected private local repository")

        stage = "configure_failed_webhook_delivery"
        webhook_secret = secrets.token_hex(32)
        invalid_secret = secrets.token_hex(32)
        if invalid_secret == webhook_secret:
            invalid_secret = secrets.token_hex(32)
        invalid_secret_path = state / "webhook-invalid.secret"
        valid_secret_path = state / "webhook-valid.secret"
        helpers.forgejo_helpers.write_private(invalid_secret_path, invalid_secret.encode("ascii"))
        helpers.forgejo_helpers.write_private(valid_secret_path, webhook_secret.encode("ascii"))
        receiver_port = helpers.select_receiver_port()
        receiver_address = f"127.0.0.1:{receiver_port}"
        receiver = start_receiver(
            receiver_bin, state, first_receiver_log, receiver_address, invalid_secret_path, journal_path
        )
        proxy_port = helpers.select_receiver_port()
        proxy = CountingWebhookProxy(
            "127.0.0.1",
            proxy_port,
            "127.0.0.1",
            receiver_port,
        )
        proxy.start()
        hook_target = f"http://127.0.0.1:{proxy_port}/forgejo/webhook"
        hook, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/hooks",
            {
                "type": "gitea",
                "config": {"url": hook_target, "content_type": "json", "secret": webhook_secret},
                "events": ["issues"],
                "active": True,
            },
        )
        hook_id = hook.get("id")
        if not isinstance(hook_id, int) or hook_id < 1 or hook.get("active") is not True:
            raise RuntimeError("Forgejo did not create the expected active webhook")
        issue, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {"title": "Native webhook replay test", "body": "One failed delivery is replayed natively."},
        )
        issue_number = issue.get("number")
        if not isinstance(issue_number, int) or isinstance(issue_number, bool) or issue_number < 1:
            raise RuntimeError("Forgejo did not return a valid issue number")
        tasks_after_failure = wait_for_task_count(database, hook_id, 1, 401)
        original_task = tasks_after_failure[0]
        if original_task["is_succeed"] is not False or original_task["is_delivered"] is not True:
            raise RuntimeError("Forgejo did not record the failed webhook delivery")
        if journal_path.exists() and journal_path.stat().st_size != 0:
            raise RuntimeError("receiver accepted an event with the intentionally invalid secret")
        initial_proxy_records = proxy.snapshot()
        if (
            len(initial_proxy_records) != 1
            or initial_proxy_records[0].get("delivery_id") != original_task["uuid"]
            or initial_proxy_records[0].get("forwarded_status") != 401
            or not isinstance(initial_proxy_records[0].get("request_body_sha256"), str)
        ):
            raise RuntimeError("proxy did not observe the single initial failed delivery")
        time.sleep(1.0)
        observed_task_count = len(hook_task_readback(database, hook_id))
        requests_after_observation = proxy.snapshot()
        if observed_task_count != 1 or requests_after_observation != initial_proxy_records:
            raise RuntimeError("Forgejo retried the initial webhook task without a manual replay")

        stage = "restart_receiver_with_correct_webhook_secret"
        failed_receiver_stop = helpers.stop_process(receiver)
        failed_receiver_listeners = helpers.listener_addresses_for_port(receiver_port)
        receiver_stop_history.append(
            {
                "phase": "before_native_replay",
                "stop": failed_receiver_stop,
                "listener_addresses_after_stop": failed_receiver_listeners,
            }
        )
        receiver = None
        if not failed_receiver_stop.get("stopped") or failed_receiver_listeners:
            raise RuntimeError("failed receiver did not stop cleanly before native replay")
        receiver = start_receiver(
            receiver_bin, state, replay_receiver_log, receiver_address, valid_secret_path, journal_path
        )

        stage = "authenticated_native_replay"
        hook_details_path = f"/{OWNER}/{REPOSITORY}/settings/hooks/{hook_id}"
        replay_path = f"/{OWNER}/{REPOSITORY}/settings/hooks/{hook_id}/replay/{original_task['uuid']}"
        no_session_opener = build_opener(ProxyHandler({}), helpers.NoRedirect())
        no_session_status, no_session_headers, _ = local_web_request(
            no_session_opener,
            "POST",
            replay_path,
            b"",
            {
                "Origin": ORIGIN,
                "Sec-Fetch-Site": "same-origin",
                "Referer": ORIGIN + hook_details_path,
            },
        )
        no_session_location = urlsplit(
            urljoin(ORIGIN + replay_path, no_session_headers.get("location", ""))
        )
        tasks_after_no_session = hook_task_readback(database, hook_id)
        if (
            no_session_status not in {302, 303}
            or no_session_location.path != "/user/login"
            or len(tasks_after_no_session) != 1
            or proxy.snapshot() != initial_proxy_records
        ):
            raise RuntimeError("native replay without a session was not redirected without creating a task")

        web_session = login_web_session(OWNER, owner_password)
        details_status, _, _ = local_web_request(web_session, "GET", hook_details_path)
        if details_status != 200:
            raise RuntimeError("authenticated owner could not read the local webhook settings page")
        rejected_status, _, _ = local_web_request(
            web_session,
            "POST",
            replay_path,
            b"",
            {
                "Origin": "https://cross-origin.invalid",
                "Sec-Fetch-Site": "cross-site",
                "Referer": "https://cross-origin.invalid/",
            },
        )
        tasks_after_cross_origin = hook_task_readback(database, hook_id)
        if (
            rejected_status != 403
            or len(tasks_after_cross_origin) != 1
            or proxy.snapshot() != initial_proxy_records
        ):
            raise RuntimeError("cross-origin replay attempt was not refused without creating a task")
        unknown_uuid_path = (
            f"/{OWNER}/{REPOSITORY}/settings/hooks/{hook_id}/replay/does-not-exist-{secrets.token_hex(8)}"
        )
        unknown_uuid_status, _, _ = local_web_request(
            web_session,
            "POST",
            unknown_uuid_path,
            b"",
            {
                "Origin": ORIGIN,
                "Sec-Fetch-Site": "same-origin",
                "Referer": ORIGIN + hook_details_path,
            },
        )
        tasks_after_unknown_uuid = hook_task_readback(database, hook_id)
        if (
            unknown_uuid_status != 404
            or len(tasks_after_unknown_uuid) != 1
            or proxy.snapshot() != initial_proxy_records
        ):
            raise RuntimeError("native replay of a missing UUID was not refused without creating a task")

        replay_attempts: list[dict[str, Any]] = []
        tasks_after_replay = tasks_after_unknown_uuid
        for attempt in range(2):
            replay_status, replay_headers, _ = local_web_request(
                web_session,
                "POST",
                replay_path,
                b"",
                {
                    "Origin": ORIGIN,
                    "Sec-Fetch-Site": "same-origin",
                    "Referer": ORIGIN + hook_details_path,
                },
            )
            location = replay_headers.get("location")
            resolved_location = urljoin(ORIGIN + replay_path, location or "")
            parsed_location = urlsplit(resolved_location)
            if (
                replay_status not in {302, 303}
                or parsed_location.scheme != "http"
                or parsed_location.netloc != "127.0.0.1:3130"
                or not parsed_location.path.endswith(f"/settings/hooks/{hook_id}")
            ):
                raise RuntimeError("native replay route did not return its local success redirect")
            replay_attempts.append(
                {"http_status": replay_status, "redirect_path": parsed_location.path}
            )
            tasks_after_replay = wait_for_task_count(database, hook_id, attempt + 2, 202)

        replay_tasks = tasks_after_replay[1:]
        journal_records = emitted.wait_for_journal_count(journal_path, 2)
        if len(journal_records) != 2:
            raise RuntimeError("receiver did not journal exactly two events after the repeated native replay")
        replay_chain_summary = validate_native_replay_chain(
            original_task, replay_tasks, journal_records
        )
        payload_summaries = [
            emitted.parse_live_delivery_record(
                journal_record,
                owner=OWNER,
                repository=REPOSITORY,
                issue_number=issue_number,
                expected_action="opened",
            )
            for journal_record in journal_records
        ]
        if [item["delivery_id"] for item in payload_summaries] != replay_chain_summary["replay_uuids"]:
            raise RuntimeError("receiver payload parser did not bind each native replay UUID")
        proxy_records_before_inactive = proxy.snapshot()
        expected_proxy_ids = [original_task["uuid"], *replay_chain_summary["replay_uuids"]]
        if (
            [item.get("delivery_id") for item in proxy_records_before_inactive]
            != expected_proxy_ids
            or [item.get("forwarded_status") for item in proxy_records_before_inactive]
            != [401, 202, 202]
            or any(item.get("path") != "/forgejo/webhook" for item in proxy_records_before_inactive)
        ):
            raise RuntimeError("HTTP proxy did not observe exactly the initial and replay deliveries")
        proxy_delivery_binding = validate_proxy_delivery_attempts(
            proxy_records_before_inactive,
            [original_task, *replay_tasks],
        )

        stage = "deactivate_hook_before_native_replay"
        owner_api.request(
            "PATCH",
            f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
            {"active": False, "events": ["issues"]},
        )
        inactive_hook_before = safe_hook_readback(owner_api, hook_id)
        if inactive_hook_before.get("active") is not False:
            raise RuntimeError("Forgejo did not confirm the hook inactive before native replay")
        inactive_journal_before = emitted.read_journal_records(journal_path)
        if inactive_journal_before != journal_records:
            raise RuntimeError("receiver journal changed after the successful replay chain")
        inactive_replay_status, inactive_replay_headers, _ = local_web_request(
            web_session,
            "POST",
            replay_path,
            b"",
            {
                "Origin": ORIGIN,
                "Sec-Fetch-Site": "same-origin",
                "Referer": ORIGIN + hook_details_path,
            },
        )
        inactive_location = urlsplit(
            urljoin(ORIGIN + replay_path, inactive_replay_headers.get("location", ""))
        )
        if (
            inactive_replay_status not in {302, 303}
            or inactive_location.scheme != "http"
            or inactive_location.netloc != "127.0.0.1:3130"
            or not inactive_location.path.endswith(f"/settings/hooks/{hook_id}")
        ):
            raise RuntimeError("inactive hook replay did not return the local success redirect")
        tasks_after_inactive_replay = wait_for_task_count(database, hook_id, 4, 0)
        inactive_task = tasks_after_inactive_replay[-1]
        inactive_hook_after = safe_hook_readback(owner_api, hook_id)
        proxy_records_after_inactive = proxy.snapshot()
        journal_records_after_inactive = emitted.read_journal_records(journal_path)
        inactive_replay_summary = validate_inactive_hook_replay(
            inactive_hook_after,
            inactive_replay_status,
            original_task,
            inactive_task,
            proxy_records_before_inactive,
            proxy_records_after_inactive,
            inactive_journal_before,
            journal_records_after_inactive,
        )
        if inactive_journal_before != journal_records_after_inactive:
            raise RuntimeError("inactive hook replay added a receiver journal event")
        task_rows = tasks_after_inactive_replay
        journal_records = journal_records_after_inactive
        proxy_records = proxy_records_after_inactive
        result = {
            "schema_version": 1,
            "verdict": "passed",
            "forgejo_version": FORGEJO_VERSION,
            "repository": f"{OWNER}/{REPOSITORY}",
            "issue_number": issue_number,
            "hook_id": hook_id,
            "initial_delivery": {
                "delivery_id": original_task["uuid"],
                "http_status": original_task["response_status"],
                "is_delivered": original_task["is_delivered"],
                "is_succeed": original_task["is_succeed"],
                "receiver_journal_records": 0,
            },
            "task_count_observation": {
                "observation_seconds": 1.0,
                "hook_task_count_before_manual_replay": observed_task_count,
                "http_request_count_observed": True,
                "proxy_request_count_after_observation": len(requests_after_observation),
            },
            "no_session_replay_attempt": {
                "http_status": no_session_status,
                "redirect_path": no_session_location.path,
                "hook_task_count_after_attempt": len(tasks_after_no_session),
                "created_task": False,
            },
            "cross_origin_replay_attempt": {
                "http_status": rejected_status,
                "hook_task_count_after_attempt": len(tasks_after_cross_origin),
                "created_task": False,
            },
            "unknown_uuid_replay_attempt": {
                "http_status": unknown_uuid_status,
                "hook_task_count_after_attempt": len(tasks_after_unknown_uuid),
                "created_task": False,
            },
            "native_replay": {
                "route": "POST /{owner}/{repo}/settings/hooks/{id}/replay/{uuid}",
                "authenticated_as": OWNER,
                "post_count_for_same_original_uuid": len(replay_attempts),
                "post_results": replay_attempts,
                "idempotent_for_repeated_post": False,
                "binding": replay_chain_summary,
            },
            "inactive_hook_replay": inactive_replay_summary,
            "proxy_delivery_binding": proxy_delivery_binding,
            "proxy_delivery_attempts": proxy_records,
            "receiver_events": payload_summaries,
            "journal_records_after_replay_and_inactive_replay": len(journal_records),
        }

    except Exception as error:
        failure = f"{type(error).__name__}: {error}"
    finally:
        if owner_api is not None and hook_id is not None:
            try:
                owner_api.request(
                    "PATCH",
                    f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                    {"active": False, "events": ["issues"]},
                )
                hook_readback = safe_hook_readback(owner_api, hook_id)
                hook_deactivated = hook_readback.get("active") is False
                if not hook_deactivated:
                    cleanup_failure = "webhook deactivation readback was not inactive"
            except Exception:
                hook_readback = {"id": hook_id, "active": None, "secret_value_recorded": False}
                cleanup_failure = "webhook deactivation/readback failed"
        else:
            hook_readback = None

        if proxy is not None:
            try:
                proxy_stop = proxy.stop()
                proxy_listeners = (
                    helpers.listener_addresses_for_port(proxy_port)
                    if proxy_port is not None
                    else []
                )
                if not proxy_stop.get("stopped") or proxy_listeners:
                    cleanup_failure = cleanup_failure or "HTTP proxy thread/listener remained after cleanup"
            except Exception:
                cleanup_failure = cleanup_failure or "HTTP proxy cleanup readback failed"

        if receiver is not None:
            try:
                receiver_stop = helpers.stop_process(receiver)
                listeners = (
                    helpers.listener_addresses_for_port(receiver_port)
                    if receiver_port is not None
                    else []
                )
                receiver_stop_history.append(
                    {
                        "phase": "final_cleanup",
                        "stop": receiver_stop,
                        "listener_addresses_after_stop": listeners,
                    }
                )
                if not receiver_stop.get("stopped") or listeners:
                    cleanup_failure = cleanup_failure or "receiver process/listener remained after cleanup"
            except Exception:
                cleanup_failure = cleanup_failure or "receiver cleanup readback failed"
        if server is not None:
            try:
                server_stop = helpers.stop_process(server)
                forgejo_listeners = helpers.listener_addresses_for_port(3130)
                if not server_stop.get("stopped") or forgejo_listeners:
                    cleanup_failure = cleanup_failure or "Forgejo process/listener remained after cleanup"
            except Exception:
                cleanup_failure = cleanup_failure or "Forgejo cleanup readback failed"

        secrets_to_redact = [
            value for value in (owner_token, owner_password, webhook_secret, invalid_secret) if value
        ]
        result_record = result or {
            "schema_version": 1,
            "verdict": "failed",
            "stage": stage,
            "error": failure or "run did not produce a result",
        }
        result_record["process_cleanup"] = {
            "webhook_deactivated_before_server_stop": hook_deactivated,
            "cleanup_failure": cleanup_failure,
            "http_proxy_stop": proxy_stop,
            "http_proxy_listener_addresses_after_stop": proxy_listeners,
            "http_proxy_thread_exited": proxy is None or proxy_stop is not None and proxy_stop.get("stopped") is True,
            "receiver_stops": receiver_stop_history,
            "forgejo_stop": server_stop,
            "tracked_processes_exited": bool(server is None or server.poll() is not None)
            and bool(receiver is None or receiver.poll() is not None)
            and (proxy is None or (proxy_stop is not None and proxy_stop.get("stopped") is True)),
        }
        try:
            helpers.write_safe_json(evidence_dir / "result.json", result_record, secrets_to_redact)
            if hook_readback is not None:
                helpers.write_safe_json(evidence_dir / "hook-readback.json", hook_readback, secrets_to_redact)
            if task_rows:
                helpers.write_safe_json(evidence_dir / "hook-tasks.json", task_rows, secrets_to_redact)
            if journal_records:
                helpers.write_safe_json(evidence_dir / "webhook-events.json", journal_records, secrets_to_redact)
            if proxy is not None:
                helpers.write_safe_json(
                    evidence_dir / "http-proxy.json",
                    {"schema_version": 1, "records": proxy.snapshot()},
                    secrets_to_redact,
                )
            if replay_chain_summary is not None:
                helpers.write_safe_json(
                    evidence_dir / "native-replay.json",
                    {
                        "schema_version": 1,
                        "original_task": task_rows[0],
                        "replay_tasks": task_rows[1:],
                        "replay_binding": replay_chain_summary,
                        "no_session_refusal_status": result_record.get("no_session_replay_attempt", {}).get("http_status"),
                        "cross_origin_refusal_status": result_record.get("cross_origin_replay_attempt", {}).get("http_status"),
                        "unknown_uuid_refusal_status": result_record.get("unknown_uuid_replay_attempt", {}).get("http_status"),
                        "inactive_hook_replay": inactive_replay_summary,
                        "inactive_replay_task": task_rows[-1] if inactive_replay_summary is not None else None,
                    },
                    secrets_to_redact,
                )
            if build_info is not None:
                helpers.write_safe_json(evidence_dir / "build-provenance.json", build_info, secrets_to_redact)
            git_head = subprocess.run(
                ["git", "-C", str(repo), "rev-parse", "HEAD"],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                timeout=5,
                check=True,
            ).stdout.strip()
            git_status = subprocess.run(
                ["git", "-C", str(repo), "status", "--short"],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                timeout=5,
                check=True,
            ).stdout
            provenance = {
                "forgejo_version": version.stdout.strip(),
                "forgejo_binary_sha256": helpers.sha256_file(forgejo),
                "forgejo_source_commit_examined": "0d791421f85ca13a63b665053d81ea765fbbc571",
                "forgejo_binary_build_commit_established": False,
                "source_head_commit": git_head,
                "source_worktree_dirty": bool(git_status.strip()),
                "source_sha256": source_hashes(repo),
                "private_state_dir": str(state),
                "private_state_dir_is_not_in_evidence": True,
                "build": build_info,
                "credentials_recorded": False,
            }
            helpers.write_safe_json(evidence_dir / "provenance.json", provenance, secrets_to_redact)
            helpers.write_checksums(evidence_dir)
        except Exception as error:
            cleanup_failure = cleanup_failure or f"evidence writing failed: {type(error).__name__}"
        if cleanup_failure is not None and result_record.get("verdict") == "passed":
            result_record["verdict"] = "failed"
            result_record["cleanup_failure"] = cleanup_failure
            try:
                helpers.write_safe_json(evidence_dir / "result.json", result_record, secrets_to_redact)
                helpers.write_checksums(evidence_dir)
            except Exception:
                pass
    if failure is not None:
        raise RuntimeError(f"Forgejo native webhook replay E2E failed at {stage}: {failure}")
    if cleanup_failure is not None:
        raise RuntimeError(f"Forgejo native webhook replay cleanup failed: {cleanup_failure}")
    return result_record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", required=True)
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    args = parser.parse_args()
    try:
        value = run(args)
    except Exception as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print(json.dumps(value, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
