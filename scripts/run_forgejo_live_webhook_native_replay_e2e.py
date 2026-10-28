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
from urllib.parse import quote, urlencode, urljoin, urlsplit
from urllib.request import HTTPCookieProcessor, ProxyHandler, Request, build_opener
from http.cookiejar import CookieJar

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_e2e as helpers  # noqa: E402
import run_forgejo_live_webhook_reconcile_emitted as emitted  # noqa: E402
import run_forgejo_native_push_webhook_reconcile as push_reconcile  # noqa: E402

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
                    "forwarded_retry_after": None,
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
                    record["forwarded_retry_after"] = response_headers.get("retry-after")
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


class ReceiverQueuePressure:
    """Hold the real receiver's four workers and sixteen accepted queue slots."""

    REQUEST_WORKERS = 4
    QUEUED_CONNECTIONS = 16

    def __init__(self, port: int) -> None:
        if type(port) is not int or not 1 <= port <= 65535:
            raise ValueError("receiver port must be a valid TCP port")
        self.port = port
        self.partial_clients: list[socket.socket] = []
        self.queued_clients: list[socket.socket] = []
        self._released = False

    def occupy(self) -> None:
        try:
            for _ in range(self.REQUEST_WORKERS):
                client = socket.create_connection(("127.0.0.1", self.port), timeout=5)
                client.settimeout(10)
                client.sendall(
                    b"POST /forgejo/webhook HTTP/1.1\r\n"
                    b"Host: 127.0.0.1\r\n"
                    b"Content-Length: 128\r\n\r\npartial"
                )
                self.partial_clients.append(client)
            time.sleep(0.25)
            for _ in range(self.QUEUED_CONNECTIONS):
                client = socket.create_connection(("127.0.0.1", self.port), timeout=5)
                client.settimeout(10)
                client.sendall(
                    b"GET /forgejo/webhook HTTP/1.1\r\n"
                    b"Host: 127.0.0.1\r\n"
                    b"Content-Length: 0\r\n\r\n"
                )
                client.shutdown(socket.SHUT_WR)
                self.queued_clients.append(client)
            time.sleep(0.25)
        except Exception:
            self.close()
            raise

    @staticmethod
    def _read_status(client: socket.socket) -> int:
        response = http.client.HTTPResponse(client)
        response.begin()
        if len(response.read(4096)) >= 4096:
            raise RuntimeError("receiver overload probe returned an oversized response")
        status = response.status
        client.close()
        return status

    def drain(self) -> dict[str, Any]:
        if self._released:
            raise RuntimeError("receiver queue pressure was already released")
        try:
            partial_statuses = [self._read_status(client) for client in self.partial_clients]
            queued_statuses = [self._read_status(client) for client in self.queued_clients]
            if partial_statuses != [408] * self.REQUEST_WORKERS:
                raise RuntimeError("partial receiver probes did not all expire with HTTP 408")
            if queued_statuses != [405] * self.QUEUED_CONNECTIONS:
                raise RuntimeError("queued receiver probes did not all drain with HTTP 405")
            self._released = True
            return {
                "partial_worker_requests": len(partial_statuses),
                "partial_worker_statuses": partial_statuses,
                "queued_probe_requests": len(queued_statuses),
                "queued_probe_statuses": queued_statuses,
                "drained": True,
            }
        finally:
            self.close()

    def close(self) -> None:
        for client in [*self.partial_clients, *self.queued_clients]:
            try:
                client.close()
            except OSError:
                pass
        self._released = True


class ReconciliationApiObservationProxy:
    """Count a bounded local adapter-to-Forgejo request window without logging credentials."""

    MAX_BODY_BYTES = 8 * 1024 * 1024

    def __init__(self, listen_port: int = 3130, upstream_port: int = 3131) -> None:
        if listen_port != 3130 or upstream_port != 3131:
            raise ValueError("API observation proxy ports are fixed to the local E2E profile")
        self.listen_port = listen_port
        self.upstream_port = upstream_port
        self._records: list[dict[str, Any]] = []
        self._reader_token: str | None = None
        self._lock = threading.Lock()
        self._allowed_read_routes: frozenset[tuple[str, str]] | None = None
        self._request_barrier: OneShotApiGetBarrier | None = None
        self._started = False
        owner = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, _format: str, *_args: object) -> None:
                return

            def _respond(self, status: int, body: bytes) -> None:
                try:
                    self.send_response(status)
                    self.send_header("Content-Type", "text/plain; charset=utf-8")
                    self.send_header("Content-Length", str(len(body)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    if self.command != "HEAD" and body:
                        self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError, OSError):
                    pass
                finally:
                    self.close_connection = True

            def _forward(self) -> None:
                if not self.path.startswith("/") or any(char in self.path for char in "\r\n"):
                    self._respond(400, b"invalid local API proxy path")
                    return
                if self.headers.get("Transfer-Encoding"):
                    self._respond(400, b"chunked requests are not supported by the local observer")
                    return
                try:
                    body_length = int(self.headers.get("Content-Length", "0"))
                except ValueError:
                    body_length = -1
                if not 0 <= body_length <= owner.MAX_BODY_BYTES:
                    self._respond(413, b"local API proxy request exceeds its bound")
                    return
                body = self.rfile.read(body_length) if body_length else None
                if body is not None and len(body) != body_length:
                    self._respond(400, b"incomplete local API proxy request")
                    return

                authorization = self.headers.get("Authorization", "")
                with owner._lock:
                    reader_token = owner._reader_token
                    allowed_read_routes = owner._allowed_read_routes
                    request_barrier = owner._request_barrier
                record = {
                    "method": self.command,
                    "path": self.path,
                    "status": None,
                    "reader_credential_match": reader_token is not None
                    and authorization == f"token {reader_token}",
                    "forwarded": False,
                    "gate": "allowlisted" if allowed_read_routes is not None else "open_setup",
                    "barrier": "none",
                }
                with owner._lock:
                    record_index = len(owner._records)
                    owner._records.append(record)

                route = (self.command, self.path)
                if allowed_read_routes is not None and route not in allowed_read_routes:
                    owner._finish_record(record_index, status=403, forwarded=False, barrier="not_allowlisted")
                    self._respond(403, b"route is outside the local reconciliation read allowlist")
                    return

                barrier_claimed = request_barrier is not None and request_barrier.claim(
                    self.command, self.path
                )
                if barrier_claimed:
                    owner._finish_record(
                        record_index, status=None, forwarded=False, barrier="held"
                    )
                    if not request_barrier.wait_for_decision():
                        owner._finish_record(
                            record_index, status=503, forwarded=False, barrier="cancelled_before_upstream"
                        )
                        self._respond(503, b"local reconciliation request cancelled at test barrier")
                        return

                excluded = {"connection", "keep-alive", "proxy-connection", "transfer-encoding"}
                forwarded_headers = {
                    key: value for key, value in self.headers.items() if key.lower() not in excluded
                }
                forwarded_headers["Host"] = f"127.0.0.1:{owner.upstream_port}"
                forwarded_headers["Connection"] = "close"
                connection = http.client.HTTPConnection(
                    "127.0.0.1", owner.upstream_port, timeout=10
                )
                status = 502
                response_headers: list[tuple[str, str]] = []
                try:
                    connection.request(self.command, self.path, body=body, headers=forwarded_headers)
                    response = connection.getresponse()
                    response_body = response.read(owner.MAX_BODY_BYTES + 1)
                    status = response.status
                    response_headers = response.getheaders()
                    if len(response_body) > owner.MAX_BODY_BYTES:
                        status = 502
                        response_body = b"local Forgejo API response exceeds observer bound"
                        response_headers = []
                except (OSError, http.client.HTTPException):
                    response_body = b"local Forgejo API upstream unavailable"
                    response_headers = []
                finally:
                    connection.close()

                owner._finish_record(
                    record_index,
                    status=status,
                    forwarded=True,
                    barrier="released_to_upstream" if barrier_claimed else "none",
                )

                try:
                    self.send_response(status)
                    excluded_response = {"connection", "content-length", "date", "server", "transfer-encoding"}
                    for key, value in response_headers:
                        if key.lower() not in excluded_response:
                            self.send_header(key, value)
                    self.send_header("Content-Length", str(len(response_body)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    if self.command != "HEAD" and response_body:
                        self.wfile.write(response_body)
                except (BrokenPipeError, ConnectionResetError, OSError):
                    pass
                finally:
                    self.close_connection = True

            do_GET = _forward
            do_HEAD = _forward
            do_POST = _forward
            do_PATCH = _forward
            do_PUT = _forward
            do_DELETE = _forward
            do_OPTIONS = _forward

        self._server = ThreadingHTTPServer(("127.0.0.1", listen_port), Handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    def set_reader_token(self, token: str) -> None:
        if not isinstance(token, str) or not token:
            raise ValueError("read-only API token must be a nonempty string")
        with self._lock:
            self._reader_token = token

    def set_allowed_read_routes(self, routes: set[tuple[str, str]]) -> None:
        if (
            not isinstance(routes, set)
            or not 1 <= len(routes) <= 8
            or any(
                not isinstance(route, tuple)
                or len(route) != 2
                or route[0] != "GET"
                or not isinstance(route[1], str)
                or not route[1].startswith("/api/v1/repos/")
                or len(route[1]) > 4096
                or "?" in route[1]
                or any(char in route[1] for char in "\r\n#")
                for route in routes
            )
        ):
            raise ValueError("reconciliation route policy must be a small set of exact repository GET paths")
        with self._lock:
            self._allowed_read_routes = frozenset(routes)

    def set_request_barrier(self, barrier: "OneShotApiGetBarrier") -> None:
        if not isinstance(barrier, OneShotApiGetBarrier):
            raise TypeError("API request barrier has the wrong local test type")
        with self._lock:
            if self._request_barrier is not None:
                raise RuntimeError("an API request barrier is already installed")
            self._request_barrier = barrier

    def _finish_record(
        self, index: int, *, status: int | None, forwarded: bool, barrier: str
    ) -> None:
        with self._lock:
            if not 0 <= index < len(self._records):
                raise RuntimeError("API proxy request record disappeared")
            self._records[index].update(
                {"status": status, "forwarded": forwarded, "barrier": barrier}
            )

    def start(self) -> None:
        self._thread.start()
        self._started = True

    def snapshot(self) -> list[dict[str, Any]]:
        with self._lock:
            return [dict(item) for item in self._records]

    def stop(self) -> dict[str, Any]:
        if not self._started:
            self._server.server_close()
            return {"stopped": True}
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)
        self._started = False
        return {"stopped": not self._thread.is_alive()}


class OneShotApiGetBarrier:
    """Hold one exact local API GET until the owning test releases or drops it."""

    def __init__(self, path: str, *, timeout: float = 30.0) -> None:
        if (
            not isinstance(path, str)
            or not path.startswith("/api/v1/repos/")
            or len(path) > 4096
            or "?" in path
            or any(char in path for char in "\r\n#")
            or not isinstance(timeout, (int, float))
            or isinstance(timeout, bool)
            or not 0.1 <= timeout <= 60.0
        ):
            raise ValueError("API GET barrier binding or timeout is invalid")
        self.path = path
        self.timeout = float(timeout)
        self._lock = threading.Lock()
        self._claimed = False
        self._waiter_started = False
        self._decision: bool | None = None
        self._reached = threading.Event()
        self._release = threading.Event()
        self._finished = threading.Event()

    def claim(self, method: str, path: str) -> bool:
        if method != "GET" or path != self.path:
            return False
        with self._lock:
            if self._claimed:
                return False
            self._claimed = True
            return True

    def wait_for_decision(self) -> bool:
        with self._lock:
            if not self._claimed or self._waiter_started:
                raise RuntimeError("API GET barrier has no unique claimed request")
            self._waiter_started = True
        self._reached.set()
        released = self._release.wait(self.timeout)
        with self._lock:
            if not released:
                self._decision = False
            decision = self._decision is True
        self._finished.set()
        return decision

    def wait_until_reached(self, timeout: float) -> bool:
        return self._reached.wait(timeout)

    def release(self, *, forward: bool) -> bool:
        if type(forward) is not bool:
            raise ValueError("API GET barrier decision must be boolean")
        with self._lock:
            if not self._claimed or not self._reached.is_set() or self._release.is_set():
                return False
            self._decision = forward
            self._release.set()
            return True

    def wait_until_finished(self, timeout: float) -> bool:
        return self._finished.wait(timeout)

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            decision = self._decision
            return {
                "path": self.path,
                "claimed": self._claimed,
                "reached": self._reached.is_set(),
                "released": self._release.is_set(),
                "forward": decision,
                "finished": self._finished.is_set(),
            }


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


def validate_native_replay_reconciliation_batch(
    exit_status: int,
    requested_delivery_ids: list[str],
    result: dict[str, Any],
    *,
    owner: str,
    repository: str,
    issue_number: int,
    current_issue: dict[str, Any],
) -> list[dict[str, Any]]:
    """Bind distinct replay UUIDs to the same fresh Forgejo issue snapshot."""
    if type(exit_status) is not int or exit_status != 0:
        raise ValueError("native replay reconciliation did not exit successfully")
    if (
        not isinstance(requested_delivery_ids, list)
        or not 2 <= len(requested_delivery_ids) <= 100
        or any(not isinstance(item, str) or not item for item in requested_delivery_ids)
        or len(set(requested_delivery_ids)) != len(requested_delivery_ids)
    ):
        raise ValueError("native replay delivery selection is invalid")
    if not isinstance(current_issue, dict) or (
        type(issue_number) is not int
        or issue_number < 1
        or type(current_issue.get("number")) is not int
        or current_issue.get("number") != issue_number
        or current_issue.get("state") not in {"open", "closed"}
        or not isinstance(current_issue.get("updated_at"), str)
        or not current_issue["updated_at"]
    ):
        raise ValueError("current issue readback is invalid")
    expected_html_url = f"{ORIGIN}/{owner}/{repository}/issues/{issue_number}"
    if current_issue.get("html_url") != expected_html_url:
        raise ValueError("current issue readback is not bound to the configured repository")
    if not isinstance(result, dict) or (
        type(result.get("schema_version")) is not int
        or result.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("verdict") != "current_state_read"
    ):
        raise ValueError("native replay reconciliation batch envelope is invalid")
    attempts = result.get("upstream_read_attempts")
    if type(attempts) is not int or attempts != 1:
        raise ValueError("duplicate issue deliveries must share one upstream read")
    rows = result.get("results")
    if not isinstance(rows, list) or len(rows) != len(requested_delivery_ids):
        raise ValueError("native replay reconciliation result count is invalid")
    for delivery_id, row in zip(requested_delivery_ids, rows, strict=True):
        resource = row.get("resource") if isinstance(row, dict) else None
        state = row.get("canonical_state") if isinstance(row, dict) else None
        if (
            not isinstance(row, dict)
            or row.get("delivery_id") != delivery_id
            or row.get("verdict") != "current_state_read"
            or not isinstance(resource, dict)
            or resource.get("kind") != "issue"
            or resource.get("owner") != owner
            or resource.get("repository") != repository
            or type(resource.get("number")) is not int
            or resource.get("number") != issue_number
            or not isinstance(state, dict)
            or state.get("state") != current_issue.get("state")
            or state.get("updated_at") != current_issue.get("updated_at")
            or state.get("html_url") != expected_html_url
        ):
            raise ValueError("native replay result delivery ID order or current issue readback mismatch")
    return rows


def validate_counted_reconciliation_api_reads(
    requests: list[dict[str, Any]], *, owner: str, repository: str, issue_number: int
) -> dict[str, Any]:
    """Require the independent observer to see exactly one authenticated issue GET."""
    if type(issue_number) is not int or issue_number < 1:
        raise ValueError("issue number for API observation is invalid")
    if not isinstance(requests, list) or len(requests) != 1 or not isinstance(requests[0], dict):
        raise ValueError("reconciliation must produce exactly one observed upstream request")
    request = requests[0]
    expected_path = f"/api/v1/repos/{owner}/{repository}/issues/{issue_number}"
    if set(request) != {"method", "path", "status", "reader_credential_match"}:
        raise ValueError("observed API request contains missing or sensitive fields")
    if (
        request.get("method") != "GET"
        or request.get("path") != expected_path
        or type(request.get("status")) is not int
        or request.get("status") != 200
        or request.get("reader_credential_match") is not True
    ):
        raise ValueError("observed upstream request is not the authorized current issue read")
    return {
        "request_count": 1,
        "credential_role": "read_only_non_admin",
        "requests": [dict(request)],
    }


def validate_private_not_found_reconciliation_batch(
    exit_status: int,
    requested_delivery_ids: list[str],
    result: dict[str, Any],
    *,
    owner: str,
    repository: str,
    issue_number: int,
) -> list[dict[str, Any]]:
    """Require each replay to remain inconclusive after a nonretryable private 404."""
    if type(issue_number) is not int or issue_number < 1:
        raise ValueError("issue number for private not-found result is invalid")
    if type(exit_status) is not int or exit_status != 1:
        raise ValueError("private not-found reconciliation must exit unsuccessfully")
    if (
        not isinstance(requested_delivery_ids, list)
        or not 2 <= len(requested_delivery_ids) <= 100
        or any(not isinstance(item, str) or not item for item in requested_delivery_ids)
        or len(set(requested_delivery_ids)) != len(requested_delivery_ids)
    ):
        raise ValueError("private not-found delivery selection is invalid")
    if not isinstance(result, dict) or (
        type(result.get("schema_version")) is not int
        or result.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("verdict") != "inconclusive"
        or type(result.get("upstream_read_attempts")) is not int
        or result.get("upstream_read_attempts") != 1
    ):
        raise ValueError("private not-found reconciliation envelope is invalid")
    rows = result.get("results")
    if not isinstance(rows, list) or len(rows) != len(requested_delivery_ids):
        raise ValueError("private not-found result count is invalid")
    for delivery_id, row in zip(requested_delivery_ids, rows, strict=True):
        error = row.get("error") if isinstance(row, dict) else None
        details = error.get("details") if isinstance(error, dict) else None
        resource = details.get("resource") if isinstance(details, dict) else None
        if (
            not isinstance(row, dict)
            or row.get("verdict") != "inconclusive"
            or row.get("delivery_id") != delivery_id
            or not isinstance(error, dict)
            or error.get("code") != "resource_not_found"
            or error.get("retryable") is not False
            or not isinstance(error.get("action"), str)
            or "HTTP 404" not in error["action"]
            or "repository access" not in error["action"]
            or not isinstance(details, dict)
            or details.get("upstream") != "forgejo"
            or not isinstance(resource, dict)
            or resource.get("kind") != "issue"
            or resource.get("owner") != owner
            or resource.get("repository") != repository
            or type(resource.get("number")) is not int
            or resource.get("number") != issue_number
        ):
            raise ValueError("private not-found result is missing an actionable refusal")
    return rows


def validate_counted_private_not_found_reads(
    requests: list[dict[str, Any]], *, owner: str, repository: str, issue_number: int
) -> dict[str, Any]:
    """Require exactly one observed, reader-authenticated private issue 404."""
    if type(issue_number) is not int or issue_number < 1:
        raise ValueError("issue number for private API observation is invalid")
    if not isinstance(requests, list) or len(requests) != 1 or not isinstance(requests[0], dict):
        raise ValueError("private reconciliation must produce exactly one observed request")
    request = requests[0]
    expected_path = f"/api/v1/repos/{owner}/{repository}/issues/{issue_number}"
    if set(request) != {"method", "path", "status", "reader_credential_match"}:
        raise ValueError("observed private API request contains missing or sensitive fields")
    if (
        request.get("method") != "GET"
        or request.get("path") != expected_path
        or type(request.get("status")) is not int
        or request.get("status") != 404
        or request.get("reader_credential_match") is not True
    ):
        raise ValueError("observed request is not the reader-authenticated private issue 404")
    return {
        "request_count": 1,
        "credential_role": "read_only_non_admin",
        "requests": [dict(request)],
    }


def validate_mixed_issue_reconciliation_batch(
    exit_status: int,
    requested_delivery_ids: list[str],
    result: dict[str, Any],
    *,
    owner: str,
    repository: str,
    resource_numbers: list[int],
    current_issues: dict[int, dict[str, Any]],
) -> list[dict[str, Any]]:
    """Bind a mixed existing/missing issue batch to its ordered API results."""
    if (
        not isinstance(requested_delivery_ids, list)
        or len(requested_delivery_ids) != 2
        or any(not isinstance(item, str) or not item for item in requested_delivery_ids)
        or len(set(requested_delivery_ids)) != 2
        or not isinstance(resource_numbers, list)
        or len(resource_numbers) != len(requested_delivery_ids)
        or any(type(number) is not int or number < 1 for number in resource_numbers)
        or len(set(resource_numbers)) != len(resource_numbers)
    ):
        raise ValueError("mixed issue reconciliation selection is invalid")
    if not isinstance(result, dict) or (
        type(result.get("schema_version")) is not int
        or result.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
    ):
        raise ValueError("mixed issue reconciliation envelope is invalid")
    rows = result.get("results")
    if not isinstance(rows, list) or len(rows) != len(requested_delivery_ids):
        raise ValueError("mixed issue reconciliation result count is invalid")
    attempts = result.get("upstream_read_attempts")
    if type(attempts) is not int or attempts != len(set(resource_numbers)):
        raise ValueError("mixed issue batch did not read each distinct resource once")

    expected_verdict = "inconclusive" if exit_status == 1 else "current_state_read"
    if type(exit_status) is not int or exit_status not in {0, 1}:
        raise ValueError("mixed issue reconciliation exit status is invalid")
    if result.get("verdict") != expected_verdict:
        raise ValueError("mixed issue reconciliation batch verdict is inconsistent")

    checked: list[dict[str, Any]] = []
    for delivery_id, number, row in zip(
        requested_delivery_ids, resource_numbers, rows, strict=True
    ):
        if not isinstance(row, dict) or row.get("delivery_id") != delivery_id:
            raise ValueError("mixed issue result delivery order is invalid")
        current_issue = current_issues.get(number)
        if current_issue is None:
            error = row.get("error")
            details = error.get("details") if isinstance(error, dict) else None
            resource = details.get("resource") if isinstance(details, dict) else None
            if (
                row.get("verdict") != "inconclusive"
                or not isinstance(error, dict)
                or error.get("code") != "resource_not_found"
                or error.get("retryable") is not False
                or not isinstance(resource, dict)
                or resource.get("kind") != "issue"
                or resource.get("owner") != owner
                or resource.get("repository") != repository
                or type(resource.get("number")) is not int
                or resource.get("number") != number
                or row.get("canonical_state") is not None
            ):
                raise ValueError("missing issue result is not an explicit terminal 404")
            continue

        expected_url = f"{ORIGIN}/{owner}/{repository}/issues/{number}"
        canonical = row.get("canonical_state")
        resource = row.get("resource")
        if (
            current_issue.get("number") != number
            or current_issue.get("state") not in {"open", "closed"}
            or not isinstance(current_issue.get("updated_at"), str)
            or current_issue.get("html_url") != expected_url
            or row.get("verdict") != "current_state_read"
            or not isinstance(resource, dict)
            or resource.get("kind") != "issue"
            or resource.get("owner") != owner
            or resource.get("repository") != repository
            or type(resource.get("number")) is not int
            or resource.get("number") != number
            or not isinstance(canonical, dict)
            or canonical.get("state") != current_issue.get("state")
            or canonical.get("updated_at") != current_issue.get("updated_at")
            or canonical.get("html_url") != expected_url
        ):
            raise ValueError("existing issue result differs from its independent readback")
        checked.append(row)
    if exit_status == 0 and len(checked) != len(rows):
        raise ValueError("successful mixed batch contains an unresolved resource")
    if exit_status == 1 and len(checked) != len(rows) - 1:
        raise ValueError("inconclusive mixed batch did not isolate exactly one missing resource")
    return rows


def validate_mixed_issue_api_reads(
    requests: list[dict[str, Any]],
    *,
    owner: str,
    repository: str,
    resource_numbers: list[int],
    expected_statuses: list[int],
) -> dict[str, Any]:
    """Require one authenticated API observation for each ordered issue resource."""
    if (
        not isinstance(resource_numbers, list)
        or len(resource_numbers) != 2
        or any(type(number) is not int or number < 1 for number in resource_numbers)
        or len(set(resource_numbers)) != len(resource_numbers)
        or not isinstance(expected_statuses, list)
        or len(expected_statuses) != len(resource_numbers)
        or any(type(status) is not int or status not in {200, 404} for status in expected_statuses)
        or not isinstance(requests, list)
        or len(requests) != len(resource_numbers)
    ):
        raise ValueError("mixed issue API observation selection is invalid")
    checked: list[dict[str, Any]] = []
    for request, number, status in zip(
        requests, resource_numbers, expected_statuses, strict=True
    ):
        expected_path = f"/api/v1/repos/{owner}/{repository}/issues/{number}"
        if (
            not isinstance(request, dict)
            or set(request) != {"method", "path", "status", "reader_credential_match"}
            or request.get("method") != "GET"
            or request.get("path") != expected_path
            or type(request.get("status")) is not int
            or request.get("status") != status
            or request.get("reader_credential_match") is not True
        ):
            raise ValueError("observed mixed batch API request did not match its resource")
        checked.append(dict(request))
    return {
        "request_count": len(checked),
        "credential_role": "read_only_non_admin",
        "requests": checked,
    }


def validate_native_deleted_issue_binding(
    task: dict[str, Any],
    event: dict[str, Any],
    *,
    issue_number: int,
    owner_readback_status: int,
) -> dict[str, Any]:
    """Bind a received native issue-created event to a post-delete owner 404."""
    if (
        not isinstance(task, dict)
        or not isinstance(event, dict)
        or type(issue_number) is not int
        or issue_number < 1
        or type(owner_readback_status) is not int
        or owner_readback_status != 404
    ):
        raise ValueError("native deleted-issue evidence selection is invalid")
    delivery_id = task.get("uuid")
    if (
        task.get("event_type") != "issues"
        or task.get("is_delivered") is not True
        or task.get("is_succeed") is not True
        or task.get("response_status") != 202
        or not isinstance(delivery_id, str)
        or not delivery_id
        or event.get("delivery_id") != delivery_id
        or event.get("event") != "issues"
        or event.get("action") != "opened"
        or type(event.get("issue_number")) is not int
        or event.get("issue_number") != issue_number
        or event.get("payload_state") != "open"
        or type(event.get("sequence")) is not int
        or event.get("sequence") < 1
    ):
        raise ValueError("native issue-created task is not bound to the deleted issue readback")
    return {
        "delivery_id": delivery_id,
        "issue_number": issue_number,
        "event": "issues",
        "action": "opened",
        "forgejo_task_status": 202,
        "owner_readback_status": owner_readback_status,
        "journal_sequence": event["sequence"],
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


def validate_overload_recovery(
    failed_task: dict[str, Any],
    proxy_record: dict[str, Any],
    queue_drain: dict[str, Any],
    journal_records_before_manual_replay: list[dict[str, Any]],
) -> dict[str, Any]:
    """Bind a real Forgejo 503 attempt to the receiver's retry contract and drained queue."""
    if not all(
        isinstance(value, dict)
        for value in (failed_task, proxy_record, queue_drain)
    ) or not isinstance(journal_records_before_manual_replay, list):
        raise ValueError("overload recovery evidence must be structured objects")
    if (
        failed_task.get("is_delivered") is not True
        or failed_task.get("is_succeed") is not False
        or failed_task.get("response_status") != 503
        or proxy_record.get("delivery_id") != failed_task.get("uuid")
        or proxy_record.get("forwarded_status") != 503
        or proxy_record.get("forwarded_retry_after") != "5"
        or not isinstance(proxy_record.get("request_body_sha256"), str)
        or not SHA256_RE.fullmatch(proxy_record["request_body_sha256"])
        or queue_drain.get("partial_worker_requests") != 4
        or queue_drain.get("partial_worker_statuses") != [408, 408, 408, 408]
        or queue_drain.get("queued_probe_requests") != 16
        or queue_drain.get("queued_probe_statuses") != [405] * 16
        or queue_drain.get("drained") is not True
        or journal_records_before_manual_replay
    ):
        raise ValueError("Forgejo 503 was not recovered after a verified receiver drain")
    return {
        "forgejo_task_status": 503,
        "receiver_retry_after_seconds": 5,
        "failed_delivery_id": failed_task["uuid"],
        "rejected_attempt_journaled": False,
        "partial_requests_timed_out": 4,
        "queued_probes_drained": 16,
        "manual_native_replay_required": True,
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


def run_native_deleted_resource_scenario(
    *,
    owner_api: Any,
    reader_api: Any,
    reader_token: str,
    state: Path,
    journal_path: Path,
    database: Path,
    hook_id: int,
    first_issue_number: int,
    reconcile_bin: Path,
    api_observation_proxy: ReconciliationApiObservationProxy,
    webhook_proxy: CountingWebhookProxy,
) -> tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]:
    """Exercise two native issue events, then delete one before canonical reads."""
    sibling_issue_number = first_issue_number + 1
    deleted_issue_number = first_issue_number + 2
    sibling_issue, _ = owner_api.request(
        "POST",
        f"/repos/{OWNER}/{REPOSITORY}/issues",
        {"title": "Existing sibling", "body": "Must remain readable after a sibling returns 404."},
    )
    if sibling_issue.get("number") != sibling_issue_number:
        raise RuntimeError("Forgejo sibling issue number is not contiguous in the disposable repository")
    sibling_tasks = wait_for_task_count(database, hook_id, 2, 202)
    sibling_task = sibling_tasks[-1]
    journal_records = emitted.wait_for_journal_count(journal_path, 1)
    sibling_event = emitted.parse_live_delivery_record(
        journal_records[-1],
        owner=OWNER,
        repository=REPOSITORY,
        issue_number=sibling_issue_number,
        expected_action="opened",
    )
    if sibling_task.get("uuid") != sibling_event.get("delivery_id"):
        raise RuntimeError("native sibling issue event is not bound to its Forgejo task")

    deleted_issue, _ = owner_api.request(
        "POST",
        f"/repos/{OWNER}/{REPOSITORY}/issues",
        {"title": "Delete after native delivery", "body": "Deleted before reconciliation."},
    )
    if deleted_issue.get("number") != deleted_issue_number:
        raise RuntimeError("Forgejo deleted-resource fixture issue number is not contiguous")
    deleted_tasks = wait_for_task_count(database, hook_id, 3, 202)
    deleted_task = deleted_tasks[-1]
    journal_records = emitted.wait_for_journal_count(journal_path, 2)
    deleted_event = emitted.parse_live_delivery_record(
        journal_records[-1],
        owner=OWNER,
        repository=REPOSITORY,
        issue_number=deleted_issue_number,
        expected_action="opened",
    )
    if deleted_task.get("uuid") != deleted_event.get("delivery_id"):
        raise RuntimeError("native deleted-issue event is not bound to its Forgejo task")

    delete_response, _ = owner_api.request(
        "DELETE", f"/repos/{OWNER}/{REPOSITORY}/issues/{deleted_issue_number}"
    )
    delete_http_status = getattr(owner_api, "last_status", None)
    if delete_response is not None or delete_http_status != 204:
        raise RuntimeError("Forgejo issue deletion did not return the expected HTTP 204 empty response")
    deleted_path = f"/repos/{OWNER}/{REPOSITORY}/issues/{deleted_issue_number}"
    try:
        owner_api.request("GET", deleted_path)
    except RuntimeError as error:
        if str(error) != f"Forgejo API GET {deleted_path} returned HTTP 404":
            raise
        owner_readback_status = 404
    else:
        raise RuntimeError("owner API still read the issue after deletion")
    native_deleted_binding = validate_native_deleted_issue_binding(
        deleted_task,
        deleted_event,
        issue_number=deleted_issue_number,
        owner_readback_status=owner_readback_status,
    )
    sibling_readback, _ = reader_api.request(
        "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{sibling_issue_number}"
    )
    if sibling_readback.get("number") != sibling_issue_number:
        raise RuntimeError("read-only identity could not independently read the sibling issue")

    delivery_ids = [deleted_event["delivery_id"], sibling_event["delivery_id"]]
    resource_numbers = [deleted_issue_number, sibling_issue_number]
    journal_before = journal_path.read_bytes()
    api_before = api_observation_proxy.snapshot()
    exit_status, batch, remaining_credential = push_reconcile.run_reconcile_batch(
        reconcile_bin,
        state,
        journal_path,
        delivery_ids,
        reader_token,
    )
    api_after = api_observation_proxy.snapshot()
    api_requests = api_after[len(api_before) :]
    api_observation = validate_mixed_issue_api_reads(
        api_requests,
        owner=OWNER,
        repository=REPOSITORY,
        resource_numbers=resource_numbers,
        expected_statuses=[404, 200],
    )
    if remaining_credential:
        raise RuntimeError("native deleted-resource reconciliation did not consume its credential")
    checked_rows = validate_mixed_issue_reconciliation_batch(
        exit_status,
        delivery_ids,
        batch,
        owner=OWNER,
        repository=REPOSITORY,
        resource_numbers=resource_numbers,
        current_issues={sibling_issue_number: sibling_readback},
    )
    if journal_path.read_bytes() != journal_before:
        raise RuntimeError("native deleted-resource reconciliation modified the append-only journal")

    proxy_records = webhook_proxy.snapshot()
    selected_proxy_records = [
        record
        for record in proxy_records
        if record.get("delivery_id") in {sibling_task["uuid"], deleted_task["uuid"]}
    ]
    proxy_binding = validate_proxy_delivery_attempts(
        selected_proxy_records, [sibling_task, deleted_task]
    )
    report = {
        "schema_version": 1,
        "verdict": "passed",
        "forgejo_version": FORGEJO_VERSION,
        "repository": f"{OWNER}/{REPOSITORY}",
        "native_events": {
            "sibling": {"task": sibling_task, "journal_event": sibling_event},
            "deleted_resource": {"task": deleted_task, "journal_event": deleted_event},
            "proxy_binding": proxy_binding,
        },
        "deletion": {
            "created_issue_readback": deleted_issue,
            "delete_response_empty": delete_response is None,
            "delete_http_status": delete_http_status,
            "owner_post_delete_status": owner_readback_status,
        },
        "reconciliation": {
            "delivery_ids": delivery_ids,
            "resource_order": resource_numbers,
            "exit_status": exit_status,
            "batch_result": batch,
            "validated_result_count": len(checked_rows),
            "api_observation": api_observation,
            "native_deleted_binding": native_deleted_binding,
            "sibling_readback": sibling_readback,
            "journal_unchanged": True,
            "credential_consumed": not remaining_credential,
        },
    }
    return report, api_observation, api_requests


def source_hashes(repo: Path) -> dict[str, str]:
    paths = (
        "scripts/run_forgejo_live_webhook_native_replay_e2e.py",
        "tests/test_forgejo_webhook_native_replay_e2e.py",
        "scripts/run_forgejo_live_webhook_reconcile_e2e.py",
        "scripts/run_forgejo_live_webhook_reconcile_emitted.py",
        "scripts/run_forgejo_native_push_webhook_reconcile.py",
        "tests/test_forgejo_emitted_webhook_e2e.py",
        "scripts/run_forgejo_live_pr_merge_e2e.py",
    )
    return {
        name: helpers.sha256_file(repo / name)
        for name in paths
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    overload_mode = bool(getattr(args, "receiver_overload_first_delivery", False))
    if overload_mode and bool(getattr(args, "native_deleted_resource_only", False)):
        raise RuntimeError("receiver overload replay cannot be combined with the deleted-resource-only mode")
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
    for port in (3130, 3131):
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
    helpers.forgejo_helpers.create_app_ini(state)
    app_ini = state / "custom/conf/app.ini"
    app_ini_text = app_ini.read_text(encoding="utf-8")
    if app_ini_text.count("HTTP_PORT = 3130") != 1:
        raise RuntimeError("Forgejo fixture config did not contain one expected HTTP port")
    app_ini.write_text(
        app_ini_text.replace("HTTP_PORT = 3130", "HTTP_PORT = 3131"), encoding="utf-8"
    )
    emitted.private_config_with_test_allowlist(state)
    evidence_dir.mkdir(mode=0o700, parents=True)
    forgejo_log = (state / "log/forgejo-server.log").open("wb")
    first_receiver_log = (state / "log/receiver-reject.log").open("wb")
    replay_receiver_log = (state / "log/receiver-replay.log").open("wb")
    server: subprocess.Popen[bytes] | None = None
    receiver: subprocess.Popen[bytes] | None = None
    receiver_port: int | None = None
    receiver_bin: Path | None = None
    reconcile_bin: Path | None = None
    proxy_port: int | None = None
    proxy: CountingWebhookProxy | None = None
    proxy_stop: dict[str, Any] | None = None
    proxy_listeners: list[str] = []
    api_observation_proxy: ReconciliationApiObservationProxy | None = None
    api_observation_proxy_stop: dict[str, Any] | None = None
    api_observation_proxy_listeners: list[str] = []
    api_reconciliation_observation: dict[str, Any] | None = None
    api_requests_during_reconciliation: list[dict[str, Any]] = []
    api_private_404_observation: dict[str, Any] | None = None
    api_requests_during_private_404: list[dict[str, Any]] = []
    api_private_404_recovery_observation: dict[str, Any] | None = None
    api_requests_during_private_404_recovery: list[dict[str, Any]] = []
    api_mixed_resource_observation: dict[str, Any] | None = None
    api_requests_during_mixed_resource: list[dict[str, Any]] = []
    api_mixed_resource_recovery_observation: dict[str, Any] | None = None
    api_requests_during_mixed_resource_recovery: list[dict[str, Any]] = []
    api_native_deleted_observation: dict[str, Any] | None = None
    api_requests_during_native_deleted_reconciliation: list[dict[str, Any]] = []
    build_info: dict[str, Any] | None = None
    owner_api: Any = None
    owner_token: str | None = None
    reader_api: Any = None
    reader_token: str | None = None
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
    receiver_queue_pressure: ReceiverQueuePressure | None = None
    receiver_queue_drain: dict[str, Any] | None = None
    overload_recovery_binding: dict[str, Any] | None = None
    replay_reconciliation: dict[str, Any] | None = None
    private_404_reconciliation: dict[str, Any] | None = None
    private_404_recovery: dict[str, Any] | None = None
    mixed_resource_reconciliation: dict[str, Any] | None = None
    proxy_records: list[dict[str, Any]] = []

    try:
        receiver_bin, reconcile_bin, build_info = helpers.build_local_binaries(repo, state, evidence_dir)
        stage = "start_forgejo"
        server = subprocess.Popen(
            [str(forgejo), "--work-path", str(state), "--config", str(state / "custom/conf/app.ini"), "web"],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=forgejo_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        api_observation_proxy = ReconciliationApiObservationProxy()
        api_observation_proxy.start()
        helpers.forgejo_helpers.wait_for_health(server)
        helpers.forgejo_helpers.assert_forgejo_listener(server, port=3131)

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

        stage = "create_read_only_reconciliation_identity"
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
            "native-replay-reconciliation-read-only",
            "read:user,read:repository,read:issue",
        )
        if api_observation_proxy is None:
            raise RuntimeError("independent Forgejo API observer is unavailable")
        api_observation_proxy.set_reader_token(reader_token)
        reader_api = helpers.LocalForgejoApi(reader_token)
        owner_api.request(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{helpers.READER}",
            {"permission": "read"},
        )
        reader_account, _ = reader_api.request("GET", "/user")
        reader_repo, _ = reader_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}")
        reader_permissions = reader_repo.get("permissions")
        if (
            reader_account.get("login") != helpers.READER
            or reader_account.get("is_admin") is not False
            or reader_repo.get("private") is not True
            or not isinstance(reader_permissions, dict)
            or reader_permissions.get("pull") is not True
            or reader_permissions.get("push") is not False
        ):
            raise RuntimeError("Forgejo did not confirm the non-admin read-only replay identity")

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
        initial_receiver_secret = valid_secret_path if overload_mode else invalid_secret_path
        receiver = start_receiver(
            receiver_bin, state, first_receiver_log, receiver_address, initial_receiver_secret, journal_path
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
        if overload_mode:
            stage = "fill_real_receiver_queue_before_issue_delivery"
            receiver_queue_pressure = ReceiverQueuePressure(receiver_port)
            receiver_queue_pressure.occupy()

        issue, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {"title": "Native webhook replay test", "body": "One failed delivery is replayed natively."},
        )
        issue_number = issue.get("number")
        if not isinstance(issue_number, int) or isinstance(issue_number, bool) or issue_number < 1:
            raise RuntimeError("Forgejo did not return a valid issue number")
        stage = "wait_for_first_failed_issue_delivery"
        initial_failure_status = 503 if overload_mode else 401
        tasks_after_failure = wait_for_task_count(
            database, hook_id, 1, initial_failure_status
        )
        original_task = tasks_after_failure[0]
        if original_task["is_succeed"] is not False or original_task["is_delivered"] is not True:
            raise RuntimeError("Forgejo did not record the failed webhook delivery")
        if journal_path.exists() and journal_path.stat().st_size != 0:
            raise RuntimeError("receiver journaled the initial failed issue delivery")
        initial_proxy_records = proxy.snapshot()
        if (
            len(initial_proxy_records) != 1
            or initial_proxy_records[0].get("delivery_id") != original_task["uuid"]
            or initial_proxy_records[0].get("forwarded_status") != initial_failure_status
            or not isinstance(initial_proxy_records[0].get("request_body_sha256"), str)
        ):
            raise RuntimeError("proxy did not observe the single initial failed delivery")
        if overload_mode:
            if initial_proxy_records[0].get("forwarded_retry_after") != "5":
                raise RuntimeError("receiver 503 did not preserve Retry-After through Forgejo")
            if receiver_queue_pressure is None:
                raise RuntimeError("receiver overload fixture was lost before queue drain")
            receiver_queue_drain = receiver_queue_pressure.drain()
            receiver_queue_pressure = None
            overload_recovery_binding = validate_overload_recovery(
                original_task,
                initial_proxy_records[0],
                receiver_queue_drain,
                emitted.read_journal_records(journal_path),
            )
        observation_seconds = 6.0 if overload_mode else 1.0
        time.sleep(observation_seconds)
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

        if args.native_deleted_resource_only:
            stage = "native_deleted_resource_reconciliation"
            if (
                owner_api is None
                or reader_api is None
                or reader_token is None
                or hook_id is None
                or reconcile_bin is None
                or api_observation_proxy is None
                or proxy is None
            ):
                raise RuntimeError("native deleted-resource fixture setup is incomplete")
            result, api_native_deleted_observation, api_requests_during_native_deleted_reconciliation = (
                run_native_deleted_resource_scenario(
                    owner_api=owner_api,
                    reader_api=reader_api,
                    reader_token=reader_token,
                    state=state,
                    journal_path=journal_path,
                    database=database,
                    hook_id=hook_id,
                    first_issue_number=issue_number,
                    reconcile_bin=reconcile_bin,
                    api_observation_proxy=api_observation_proxy,
                    webhook_proxy=proxy,
                )
            )
            task_rows = hook_task_readback(database, hook_id)
            journal_records = emitted.read_journal_records(journal_path)
            return result

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
            != [initial_failure_status, 202, 202]
            or any(item.get("path") != "/forgejo/webhook" for item in proxy_records_before_inactive)
        ):
            raise RuntimeError("HTTP proxy did not observe exactly the initial and replay deliveries")
        proxy_delivery_binding = validate_proxy_delivery_attempts(
            proxy_records_before_inactive,
            [original_task, *replay_tasks],
        )

        stage = "reconcile_duplicate_native_replays_as_one_issue_read"
        if reconcile_bin is None or reader_api is None or reader_token is None:
            raise RuntimeError("read-only reconciler build or credential is unavailable")
        current_issue, _ = reader_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        )
        journal_before_reconciliation = journal_path.read_bytes()
        replay_delivery_ids = replay_chain_summary["replay_uuids"]
        api_requests_before_reconciliation = api_observation_proxy.snapshot()
        reconcile_status, reconcile_batch, remaining_reader_credential = (
            push_reconcile.run_reconcile_batch(
                reconcile_bin,
                state,
                journal_path,
                replay_delivery_ids,
                reader_token,
            )
        )
        api_requests_after_reconciliation = api_observation_proxy.snapshot()
        api_requests_during_reconciliation = api_requests_after_reconciliation[
            len(api_requests_before_reconciliation) :
        ]
        api_reconciliation_observation = validate_counted_reconciliation_api_reads(
            api_requests_during_reconciliation,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
        )
        if remaining_reader_credential:
            raise RuntimeError("successful replay reconciliation did not consume its host credential")
        checked_replay_results = validate_native_replay_reconciliation_batch(
            reconcile_status,
            replay_delivery_ids,
            reconcile_batch,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
            current_issue=current_issue,
        )
        journal_after_reconciliation = journal_path.read_bytes()
        if journal_after_reconciliation != journal_before_reconciliation:
            raise RuntimeError("native replay reconciliation modified the append-only journal")
        replay_reconciliation = {
            "exit_status": reconcile_status,
            "delivery_ids": replay_delivery_ids,
            "batch_result": reconcile_batch,
            "canonical_issue_readback": {
                "number": current_issue.get("number"),
                "state": current_issue.get("state"),
                "updated_at": current_issue.get("updated_at"),
                "html_url": current_issue.get("html_url"),
            },
            "credential_source": "read_only_private_inherited_pipe_fd",
            "credential_consumed": True,
            "credential_recorded": False,
            "validated_result_count": len(checked_replay_results),
            "independent_api_observation": api_reconciliation_observation,
            "journal_unchanged": True,
        }

        stage = "revoke_reader_access_and_reconcile_private_404"
        owner_api.request(
            "DELETE",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{quote(helpers.READER, safe='')}",
        )
        journal_before_private_404 = journal_path.read_bytes()
        api_requests_before_private_404 = api_observation_proxy.snapshot()
        denied_status, denied_batch, remaining_denied_credential = (
            push_reconcile.run_reconcile_batch(
                reconcile_bin,
                state,
                journal_path,
                replay_delivery_ids,
                reader_token,
            )
        )
        api_requests_after_private_404 = api_observation_proxy.snapshot()
        api_requests_during_private_404 = api_requests_after_private_404[
            len(api_requests_before_private_404) :
        ]
        api_private_404_observation = validate_counted_private_not_found_reads(
            api_requests_during_private_404,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
        )
        if remaining_denied_credential:
            raise RuntimeError("private 404 reconciliation did not consume its host credential")
        checked_denied_results = validate_private_not_found_reconciliation_batch(
            denied_status,
            replay_delivery_ids,
            denied_batch,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
        )
        owner_issue_after_reader_denied, _ = owner_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        )
        if (
            not isinstance(owner_issue_after_reader_denied, dict)
            or owner_issue_after_reader_denied.get("number") != issue_number
            or owner_issue_after_reader_denied.get("state") != current_issue.get("state")
            or owner_issue_after_reader_denied.get("html_url") != current_issue.get("html_url")
        ):
            raise RuntimeError("owner could not confirm the issue still exists after reader 404")
        journal_after_private_404 = journal_path.read_bytes()
        if journal_after_private_404 != journal_before_private_404:
            raise RuntimeError("private 404 reconciliation modified the append-only journal")
        private_404_reconciliation = {
            "exit_status": denied_status,
            "delivery_ids": replay_delivery_ids,
            "batch_result": denied_batch,
            "credential_source": "read_only_private_inherited_pipe_fd",
            "credential_consumed": True,
            "credential_recorded": False,
            "validated_result_count": len(checked_denied_results),
            "permission_change": "owner_deleted_read_only_collaborator",
            "independent_api_observation": api_private_404_observation,
            "owner_readback_confirms_issue_exists": True,
            "journal_unchanged": True,
        }

        stage = "restore_reader_access_and_retry_private_reconciliation"
        owner_api.request(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{quote(helpers.READER, safe='')}",
            {"permission": "read"},
        )
        journal_before_private_404_recovery = journal_path.read_bytes()
        api_requests_before_private_404_recovery = api_observation_proxy.snapshot()
        recovered_status, recovered_batch, remaining_recovery_credential = (
            push_reconcile.run_reconcile_batch(
                reconcile_bin,
                state,
                journal_path,
                replay_delivery_ids,
                reader_token,
            )
        )
        api_requests_after_private_404_recovery = api_observation_proxy.snapshot()
        api_requests_during_private_404_recovery = api_requests_after_private_404_recovery[
            len(api_requests_before_private_404_recovery) :
        ]
        api_private_404_recovery_observation = validate_counted_reconciliation_api_reads(
            api_requests_during_private_404_recovery,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
        )
        if remaining_recovery_credential:
            raise RuntimeError("private 404 recovery did not consume its host credential")
        checked_recovered_results = validate_native_replay_reconciliation_batch(
            recovered_status,
            replay_delivery_ids,
            recovered_batch,
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=issue_number,
            current_issue=owner_issue_after_reader_denied,
        )
        journal_after_private_404_recovery = journal_path.read_bytes()
        if journal_after_private_404_recovery != journal_before_private_404_recovery:
            raise RuntimeError("private 404 recovery modified the append-only journal")
        private_404_recovery = {
            "exit_status": recovered_status,
            "delivery_ids": replay_delivery_ids,
            "same_delivery_ids_after_permission_restore": True,
            "batch_result": recovered_batch,
            "credential_source": "read_only_private_inherited_pipe_fd",
            "credential_consumed": True,
            "credential_recorded": False,
            "validated_result_count": len(checked_recovered_results),
            "permission_change": "owner_restored_read_only_collaborator",
            "independent_api_observation": api_private_404_recovery_observation,
            "owner_readback_confirms_issue_exists": True,
            "journal_unchanged": True,
        }

        stage = "reconcile_distinct_issue_resources_across_missing_resource_recovery"
        if receiver_address is None or webhook_secret is None or reader_api is None:
            raise RuntimeError("local receiver or read-only identity is unavailable")
        missing_issue_number = issue_number + 1
        synthetic_delivery_id = f"mixed-resource-{secrets.token_hex(16)}"
        synthetic_payload = json.dumps(
            {
                "action": "opened",
                "repository": {"full_name": f"{OWNER}/{REPOSITORY}"},
                "issue": {"number": missing_issue_number, "state": "open"},
            },
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
        synthetic_receipt = helpers.send_signed_webhook(
            receiver_address,
            webhook_secret.encode("ascii"),
            synthetic_delivery_id,
            synthetic_payload,
        )
        if (
            synthetic_receipt.get("http_status") != 202
            or not isinstance(synthetic_receipt.get("body"), dict)
            or synthetic_receipt["body"].get("accepted") is not True
            or synthetic_receipt["body"].get("duplicate") is not False
            or synthetic_receipt["body"].get("sequence") != 3
        ):
            raise RuntimeError("synthetic signed missing-issue fixture was not journaled once")
        journal_records = emitted.wait_for_journal_count(journal_path, 3)
        synthetic_event = emitted.parse_live_delivery_record(
            journal_records[-1],
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=missing_issue_number,
            expected_action="opened",
        )
        if synthetic_event.get("delivery_id") != synthetic_delivery_id:
            raise RuntimeError("synthetic fixture journal receipt changed its delivery ID")

        mixed_delivery_ids = [synthetic_delivery_id, replay_delivery_ids[0]]
        mixed_resource_numbers = [missing_issue_number, issue_number]
        journal_before_mixed_resource = journal_path.read_bytes()
        api_requests_before_mixed_resource = api_observation_proxy.snapshot()
        mixed_status, mixed_batch, remaining_mixed_credential = (
            push_reconcile.run_reconcile_batch(
                reconcile_bin,
                state,
                journal_path,
                mixed_delivery_ids,
                reader_token,
            )
        )
        api_requests_after_mixed_resource = api_observation_proxy.snapshot()
        api_requests_during_mixed_resource = api_requests_after_mixed_resource[
            len(api_requests_before_mixed_resource) :
        ]
        api_mixed_resource_observation = validate_mixed_issue_api_reads(
            api_requests_during_mixed_resource,
            owner=OWNER,
            repository=REPOSITORY,
            resource_numbers=mixed_resource_numbers,
            expected_statuses=[404, 200],
        )
        if remaining_mixed_credential:
            raise RuntimeError("mixed resource reconciliation did not consume its credential")
        checked_mixed_results = validate_mixed_issue_reconciliation_batch(
            mixed_status,
            mixed_delivery_ids,
            mixed_batch,
            owner=OWNER,
            repository=REPOSITORY,
            resource_numbers=mixed_resource_numbers,
            current_issues={issue_number: current_issue},
        )
        if journal_path.read_bytes() != journal_before_mixed_resource:
            raise RuntimeError("mixed 404 reconciliation modified the append-only journal")

        created_issue, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/issues",
            {
                "title": "Create the missing mixed-batch resource",
                "body": "This issue is created after the observed 404.",
            },
        )
        if (
            created_issue.get("number") != missing_issue_number
            or created_issue.get("state") != "open"
        ):
            raise RuntimeError("Forgejo did not create the issue that previously returned 404")
        tasks_after_created_issue = wait_for_task_count(database, hook_id, 4, 202)
        created_issue_task = tasks_after_created_issue[-1]
        journal_records = emitted.wait_for_journal_count(journal_path, 4)
        created_issue_event = emitted.parse_live_delivery_record(
            journal_records[-1],
            owner=OWNER,
            repository=REPOSITORY,
            issue_number=missing_issue_number,
            expected_action="opened",
        )
        if created_issue_event.get("delivery_id") != created_issue_task.get("uuid"):
            raise RuntimeError("native issue-created webhook is not bound to its Forgejo task")
        refreshed_issue_one, _ = reader_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{issue_number}"
        )
        refreshed_issue_two, _ = reader_api.request(
            "GET", f"/repos/{OWNER}/{REPOSITORY}/issues/{missing_issue_number}"
        )
        journal_before_mixed_recovery = journal_path.read_bytes()
        api_requests_before_mixed_recovery = api_observation_proxy.snapshot()
        mixed_recovery_status, mixed_recovery_batch, remaining_mixed_recovery_credential = (
            push_reconcile.run_reconcile_batch(
                reconcile_bin,
                state,
                journal_path,
                mixed_delivery_ids,
                reader_token,
            )
        )
        api_requests_after_mixed_recovery = api_observation_proxy.snapshot()
        api_requests_during_mixed_resource_recovery = api_requests_after_mixed_recovery[
            len(api_requests_before_mixed_recovery) :
        ]
        api_mixed_resource_recovery_observation = validate_mixed_issue_api_reads(
            api_requests_during_mixed_resource_recovery,
            owner=OWNER,
            repository=REPOSITORY,
            resource_numbers=mixed_resource_numbers,
            expected_statuses=[200, 200],
        )
        if remaining_mixed_recovery_credential:
            raise RuntimeError("mixed resource recovery did not consume its credential")
        checked_mixed_recovery_results = validate_mixed_issue_reconciliation_batch(
            mixed_recovery_status,
            mixed_delivery_ids,
            mixed_recovery_batch,
            owner=OWNER,
            repository=REPOSITORY,
            resource_numbers=mixed_resource_numbers,
            current_issues={
                issue_number: refreshed_issue_one,
                missing_issue_number: refreshed_issue_two,
            },
        )
        if journal_path.read_bytes() != journal_before_mixed_recovery:
            raise RuntimeError("mixed resource recovery modified the append-only journal")
        mixed_resource_reconciliation = {
            "missing_resource_event": {
                "source": "HMAC-signed synthetic issue event sent directly to the local receiver",
                "forgejo_native_delivery": False,
                "delivery_id": synthetic_delivery_id,
                "issue_number": missing_issue_number,
                "journal_sequence": synthetic_event["sequence"],
                "payload_sha256": hashlib.sha256(synthetic_payload).hexdigest(),
                "receiver_status": synthetic_receipt["http_status"],
            },
            "delivery_ids_reused_unchanged": mixed_delivery_ids,
            "resource_order": mixed_resource_numbers,
            "first_attempt": {
                "exit_status": mixed_status,
                "batch_result": mixed_batch,
                "validated_result_count": len(checked_mixed_results),
                "api_observation": api_mixed_resource_observation,
                "issue_one_independent_readback": current_issue,
                "journal_unchanged": True,
            },
            "correction": {
                "action": "owner_created_issue_after_observed_404",
                "created_issue_readback": created_issue,
                "native_webhook_task": created_issue_task,
                "native_webhook_event": created_issue_event,
            },
            "recovery": {
                "exit_status": mixed_recovery_status,
                "same_delivery_ids": True,
                "batch_result": mixed_recovery_batch,
                "validated_result_count": len(checked_mixed_recovery_results),
                "api_observation": api_mixed_resource_recovery_observation,
                "issue_readbacks": {
                    str(issue_number): refreshed_issue_one,
                    str(missing_issue_number): refreshed_issue_two,
                },
                "journal_unchanged": True,
            },
        }

        proxy_records_before_inactive = proxy.snapshot()
        proxy_delivery_binding = validate_proxy_delivery_attempts(
            proxy_records_before_inactive,
            [original_task, *replay_tasks, created_issue_task],
        )

        if overload_mode:
            inactive_replay_summary = {
                "skipped": True,
                "reason": "this run isolates recovery from an actual 503; inactive-hook behavior is tested separately",
            }
            task_rows = hook_task_readback(database, hook_id)
            journal_records = emitted.read_journal_records(journal_path)
            proxy_records = proxy.snapshot()
        else:
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
            tasks_before_inactive_replay = hook_task_readback(database, hook_id)
            tasks_after_inactive_replay = wait_for_task_count(
                database, hook_id, len(tasks_before_inactive_replay) + 1, 0
            )
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
                "queue_pressure_used": overload_mode,
                "retry_after": initial_proxy_records[0].get("forwarded_retry_after"),
            },
            "overload_recovery": overload_recovery_binding,
            "task_count_observation": {
                "observation_seconds": observation_seconds,
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
            "journal_records_after_mixed_recovery_and_inactive_replay": (
                len(journal_records) if not overload_mode else None
            ),
            "journal_records_after_overload_native_replay": (
                len(journal_records) if overload_mode else None
            ),
            "replay_reconciliation": replay_reconciliation,
            "private_404_reconciliation": private_404_reconciliation,
            "private_404_recovery": private_404_recovery,
            "mixed_resource_reconciliation": mixed_resource_reconciliation,
            "limitations": [
                "Forgejo, receiver, and the trusted local reconciliation CLI share the host UID; no agent process is run in this scenario.",
                "The Forgejo webhook task database is inspected read-only as local test evidence and is not a supported public API.",
                "When receiver-overload mode is enabled, Forgejo receives one actual HTTP 503 with Retry-After and its owner manually invokes native replay, which creates a new delivery UUID; automatic retries are not established.",
                "No production Forgejo, external service, model, AgentOS, token-cost measurement or production credential is used.",
            ],
        }

    except Exception as error:
        failure = f"{type(error).__name__}: {error}"
    finally:
        if receiver_queue_pressure is not None:
            receiver_queue_pressure.close()
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
                forgejo_listeners = helpers.listener_addresses_for_port(3131)
                if not server_stop.get("stopped") or forgejo_listeners:
                    cleanup_failure = cleanup_failure or "Forgejo process/listener remained after cleanup"
            except Exception:
                cleanup_failure = cleanup_failure or "Forgejo cleanup readback failed"
        if api_observation_proxy is not None:
            try:
                api_observation_proxy_stop = api_observation_proxy.stop()
                api_observation_proxy_listeners = helpers.listener_addresses_for_port(3130)
                if not api_observation_proxy_stop.get("stopped") or api_observation_proxy_listeners:
                    cleanup_failure = cleanup_failure or "API observer proxy thread/listener remained after cleanup"
            except Exception:
                cleanup_failure = cleanup_failure or "API observer proxy cleanup readback failed"

        secrets_to_redact = [
            value
            for value in (owner_token, reader_token, owner_password, webhook_secret, invalid_secret)
            if value
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
            "api_observation_proxy_stop": api_observation_proxy_stop,
            "api_observation_proxy_listener_addresses_after_stop": api_observation_proxy_listeners,
            "api_observation_proxy_thread_exited": api_observation_proxy is None
            or api_observation_proxy_stop is not None
            and api_observation_proxy_stop.get("stopped") is True,
            "tracked_processes_exited": bool(server is None or server.poll() is not None)
            and bool(receiver is None or receiver.poll() is not None)
            and (proxy is None or (proxy_stop is not None and proxy_stop.get("stopped") is True))
            and (
                api_observation_proxy is None
                or api_observation_proxy_stop is not None
                and api_observation_proxy_stop.get("stopped") is True
            ),
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
            if api_observation_proxy is not None:
                helpers.write_safe_json(
                    evidence_dir / "api-observation-proxy.json",
                    {
                        "schema_version": 1,
                        "records_for_reconciliation_window": (
                            api_requests_during_reconciliation
                        ),
                        "records_for_private_404_window": api_requests_during_private_404,
                        "records_for_private_404_recovery_window": (
                            api_requests_during_private_404_recovery
                        ),
                        "records_for_mixed_resource_window": api_requests_during_mixed_resource,
                        "records_for_mixed_resource_recovery_window": (
                            api_requests_during_mixed_resource_recovery
                        ),
                        "records_for_native_deleted_resource_window": (
                            api_requests_during_native_deleted_reconciliation
                        ),
                        "validated": api_reconciliation_observation is not None,
                        "private_404_validated": api_private_404_observation is not None,
                        "private_404_recovery_validated": (
                            api_private_404_recovery_observation is not None
                        ),
                        "mixed_resource_validated": api_mixed_resource_observation is not None,
                        "mixed_resource_recovery_validated": (
                            api_mixed_resource_recovery_observation is not None
                        ),
                        "native_deleted_resource_validated": (
                            api_native_deleted_observation is not None
                        ),
                        "measurement": "forwarded_requests_between_adapter_and_local_forgejo",
                        "credential_values_recorded": False,
                        "cleanup": {
                            "stop": api_observation_proxy_stop,
                            "listener_addresses_after_stop": api_observation_proxy_listeners,
                        },
                    },
                    secrets_to_redact,
                )
            if replay_chain_summary is not None:
                helpers.write_safe_json(
                    evidence_dir / "native-replay.json",
                    {
                        "schema_version": 1,
                        "original_task": task_rows[0],
                        "replay_tasks": [
                            task
                            for task in task_rows
                            if task.get("uuid") in set(replay_chain_summary["replay_uuids"])
                        ],
                        "replay_binding": replay_chain_summary,
                        "no_session_refusal_status": result_record.get("no_session_replay_attempt", {}).get("http_status"),
                        "cross_origin_refusal_status": result_record.get("cross_origin_replay_attempt", {}).get("http_status"),
                        "unknown_uuid_refusal_status": result_record.get("unknown_uuid_replay_attempt", {}).get("http_status"),
                        "inactive_hook_replay": inactive_replay_summary,
                        "inactive_replay_task": task_rows[-1] if inactive_replay_summary is not None else None,
                    },
                    secrets_to_redact,
                )
            if replay_reconciliation is not None:
                helpers.write_safe_json(
                    evidence_dir / "reconciliation.json",
                    replay_reconciliation,
                    secrets_to_redact,
                )
            if private_404_reconciliation is not None:
                helpers.write_safe_json(
                    evidence_dir / "private-404-reconciliation.json",
                    private_404_reconciliation,
                    secrets_to_redact,
                )
            if private_404_recovery is not None:
                helpers.write_safe_json(
                    evidence_dir / "private-404-recovery.json",
                    private_404_recovery,
                    secrets_to_redact,
                )
            if mixed_resource_reconciliation is not None:
                helpers.write_safe_json(
                    evidence_dir / "mixed-resource-reconciliation.json",
                    mixed_resource_reconciliation,
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
    parser.add_argument("--native-deleted-resource-only", action="store_true")
    parser.add_argument(
        "--receiver-overload-first-delivery",
        action="store_true",
        help=(
            "fill the real webhook receiver before the initial Forgejo delivery, verify its 503/Retry-After, "
            "then recover via native manual replay"
        ),
    )
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
