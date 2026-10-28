#!/usr/bin/env python3
"""Exercise MERGE-019's policy re-read against a real disposable Forgejo API.

The integration driver uses Forgejo on 127.0.0.1:3131. A loopback-only proxy
occupies the CLI's fixed test profile port 3130 and changes the branch rule
between the gate's first and final policy reads. Credentials remain in memory.
"""

from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import http.client
import importlib.util
import json
from pathlib import Path
import socket
import sys
import threading
from types import SimpleNamespace
from typing import Any
from urllib.parse import urlsplit


REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "scripts"))
DRIVER_PATH = REPO_ROOT / "scripts/run_forgejo_live_pr_merge_e2e.py"
SPEC = importlib.util.spec_from_file_location("forgejo_live_driver", DRIVER_PATH)
if SPEC is None or SPEC.loader is None:
    raise SystemExit("could not load the existing Forgejo live integration driver")
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)

SERVER_ORIGIN = "http://127.0.0.1:3131"

RACE_LOCK = threading.Lock()
RACE: dict[str, Any] = {
    "armed": False,
    "policy_gets": 0,
    "mutation_status": None,
    "merge_posts": 0,
    "blocked_non_get_requests": 0,
    "forwarded_non_get_requests": 0,
    "gate_non_get_upstream_attempts": 0,
    "write_guard_probes": [],
    "events": [],
}
PROXY: ThreadingHTTPServer | None = None
PROXY_THREAD: threading.Thread | None = None


def upstream(
    method: str,
    path: str,
    headers: dict[str, str],
    body: bytes | None,
    *,
    gate_request: bool = False,
) -> tuple[int, bytes, list[tuple[str, str]]]:
    if gate_request and method != "GET":
        with RACE_LOCK:
            RACE["gate_non_get_upstream_attempts"] += 1
    connection = http.client.HTTPConnection("127.0.0.1", 3131, timeout=10)
    try:
        forwarded = dict(headers)
        forwarded["Host"] = "127.0.0.1:3131"
        forwarded["Connection"] = "close"
        connection.request(method, path, body=body, headers=forwarded)
        response = connection.getresponse()
        return response.status, response.read(1024 * 1024 + 1), response.getheaders()
    finally:
        connection.close()


class ProxyHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def _respond(self, status: int, body: bytes, headers: list[tuple[str, str]]) -> None:
        self.send_response(status)
        excluded = {"connection", "transfer-encoding", "content-length", "date", "server"}
        for key, value in headers:
            if key.lower() not in excluded:
                self.send_header(key, value)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        if body:
            self.wfile.write(body)
        self.close_connection = True

    def _handle(self) -> None:
        path = self.path
        if self.command != "GET":
            with RACE_LOCK:
                if RACE["armed"]:
                    RACE["blocked_non_get_requests"] += 1
                    if (
                        self.command == "POST"
                        and urlsplit(path).path
                        == "/api/v1/repos/forge-admin/agent-forge-core/pulls/2/merge"
                    ):
                        RACE["merge_posts"] += 1
                    RACE["events"].append(
                        {
                            "event": "blocked_non_get_request",
                            "method": self.command,
                            "path": path,
                            "http_status": 409,
                        }
                    )
            self._respond(409, b"local race harness is read-only for gate requests", [])
            return

        if self.headers.get("Transfer-Encoding") or self.headers.get(
            "Content-Length", "0"
        ) not in {"0", ""}:
            self._respond(400, b"local race harness accepts bodyless GET requests only", [])
            return

        auth = self.headers.get("Authorization", "")
        safe_headers = {
            key: value
            for key, value in self.headers.items()
            if key.lower() not in {"host", "connection", "content-length"}
        }
        safe_headers["Host"] = "127.0.0.1:3131"
        safe_headers["Connection"] = "close"
        change_policy = False
        with RACE_LOCK:
            if (
                RACE["armed"]
                and self.command == "GET"
                and urlsplit(path).path
                == "/api/v1/repos/forge-admin/agent-forge-core/branch_protections"
            ):
                RACE["policy_gets"] += 1
                RACE["events"].append(
                    {
                        "event": "branch_protection_read_request",
                        "ordinal": RACE["policy_gets"],
                    }
                )
                change_policy = RACE["policy_gets"] == 2

        if change_policy:
            mutation = json.dumps(
                {"merge_whitelist_usernames": ["forge-admin", "forge-reviewer"]}
            ).encode()
            mutation_status, _mutation_body, _ = upstream(
                "PATCH",
                "/api/v1/repos/forge-admin/agent-forge-core/branch_protections/main",
                {
                    "Authorization": auth,
                    "Accept": "application/json",
                    "Content-Type": "application/json",
                },
                mutation,
            )
            with RACE_LOCK:
                RACE["mutation_status"] = mutation_status
                RACE["events"].append(
                    {"event": "policy_patch", "http_status": mutation_status}
                )
            if mutation_status != 200:
                self._respond(503, b"local race harness could not change the branch policy", [])
                return

        try:
            status, response_body, response_headers = upstream(
                self.command, path, safe_headers, None, gate_request=True
            )
        except (OSError, http.client.HTTPException):
            self._respond(502, b"local Forgejo upstream unavailable", [])
            return

        with RACE_LOCK:
            if RACE["armed"]:
                if self.command != "GET":
                    RACE["forwarded_non_get_requests"] += 1
                RACE["events"].append(
                    {
                        "event": "forward",
                        "method": self.command,
                        "path": path,
                        "http_status": status,
                    }
                )
        self._respond(status, response_body, response_headers)

    do_GET = _handle
    do_HEAD = _handle
    do_POST = _handle
    do_PATCH = _handle
    do_PUT = _handle
    do_DELETE = _handle
    do_OPTIONS = _handle
    do_CONNECT = _handle
    do_TRACE = _handle


def start_proxy() -> None:
    global PROXY, PROXY_THREAD
    if PROXY is not None:
        return
    PROXY = ThreadingHTTPServer(("127.0.0.1", 3130), ProxyHandler)
    PROXY.daemon_threads = True
    PROXY_THREAD = threading.Thread(target=PROXY.serve_forever, daemon=True)
    PROXY_THREAD.start()


def _begin_write_guard_probe() -> int:
    with RACE_LOCK:
        RACE["armed"] = True
        RACE["events"] = []
        RACE["blocked_non_get_requests"] = 0
        RACE["forwarded_non_get_requests"] = 0
        RACE["merge_posts"] = 0
        return RACE["gate_non_get_upstream_attempts"]


def _finish_write_guard_probe(
    path: str, status: int | None, upstream_attempts_before: int
) -> dict[str, Any]:
    with RACE_LOCK:
        forwarded = any(
            event.get("event") == "forward" and event.get("path") == path
            for event in RACE["events"]
        )
        blocked_count = RACE["blocked_non_get_requests"]
        forwarded_count = RACE["forwarded_non_get_requests"]
        upstream_attempts = (
            RACE["gate_non_get_upstream_attempts"] - upstream_attempts_before
        )
        probe = {
            "method": "POST",
            "path": path,
            "http_status": status,
            "blocked_non_get_count": blocked_count,
            "forwarded_non_get_count": forwarded_count,
            "upstream_attempt_count": upstream_attempts,
            "forwarded_to_forgejo": forwarded,
        }
        RACE["write_guard_probes"].append(probe)
        RACE["armed"] = False
        RACE["events"] = []
        RACE["blocked_non_get_requests"] = 0
        RACE["forwarded_non_get_requests"] = 0
        RACE["merge_posts"] = 0
    if (
        status != 409
        or blocked_count != 1
        or forwarded_count != 0
        or upstream_attempts != 0
        or forwarded
    ):
        raise RuntimeError("local race proxy did not block a write probe before forwarding")
    return probe


def verify_proxy_write_guard() -> None:
    """Probe query bypasses and slow request bodies against the read-only proxy."""
    query_path = "/api/v1/repos/forge-admin/agent-forge-core/pulls/999999/merge?probe=guard"
    upstream_attempts_before = _begin_write_guard_probe()
    connection = http.client.HTTPConnection("127.0.0.1", 3130, timeout=5)
    try:
        connection.request(
            "POST",
            query_path,
            body=b"{}",
            headers={"Content-Type": "application/json"},
        )
        response = connection.getresponse()
        status = response.status
        response.read()
    finally:
        connection.close()
        with RACE_LOCK:
            RACE["armed"] = False
    _finish_write_guard_probe(query_path, status, upstream_attempts_before)

    partial_path = "/api/v1/repos/forge-admin/agent-forge-core/pulls/888888/merge?probe=partial"
    upstream_attempts_before = _begin_write_guard_probe()
    response_bytes = bytearray()
    client = socket.create_connection(("127.0.0.1", 3130), timeout=5)
    try:
        client.settimeout(5)
        request = (
            f"POST {partial_path} HTTP/1.1\r\n"
            "Host: 127.0.0.1:3130\r\n"
            "Content-Type: application/json\r\n"
            "Content-Length: 128\r\n"
            "Connection: close\r\n\r\n"
        ).encode("ascii")
        client.sendall(request)
        response_bytes.extend(client.recv(4096))
    finally:
        client.close()
        with RACE_LOCK:
            RACE["armed"] = False
    status_line = response_bytes.split(b"\r\n", 1)[0].split()
    partial_status = int(status_line[1]) if len(status_line) >= 2 else None
    _finish_write_guard_probe(partial_path, partial_status, upstream_attempts_before)


def stop_proxy() -> None:
    global PROXY, PROXY_THREAD
    if PROXY is not None:
        PROXY.shutdown()
        PROXY.server_close()
        PROXY = None
    if PROXY_THREAD is not None:
        PROXY_THREAD.join(timeout=5)
        PROXY_THREAD = None


def main() -> int:
    global STATE_DIR
    if len(sys.argv) != 4:
        print(
            "usage: test_forgejo_live_branch_policy_race.py <forgejo-binary> <repo-root> <new-/tmp-state-dir>",
            file=sys.stderr,
        )
        return 64
    binary, repo_root, state_dir = map(Path, sys.argv[1:])
    args = SimpleNamespace(
        forgejo_binary=str(binary), repo_root=str(repo_root), state_dir=str(state_dir)
    )

    original_create_app_ini = driver.create_app_ini

    def create_app_ini_on_3131(state: Path) -> None:
        original_create_app_ini(state)
        config = state / "custom/conf/app.ini"
        text = config.read_text()
        if text.count("HTTP_PORT = 3130") != 1:
            raise RuntimeError("test driver config did not contain one expected HTTP port")
        config.write_text(text.replace("HTTP_PORT = 3130", "HTTP_PORT = 3131"))

    original_make_api = driver.make_api

    def make_api_on_server(token: str | None):
        api_call = original_make_api(token)

        def call(method: str, path: str, data: Any = None):
            previous_origin = driver.ORIGIN
            try:
                driver.ORIGIN = SERVER_ORIGIN
                return api_call(method, path, data)
            finally:
                driver.ORIGIN = previous_origin

        return call

    original_wait_for_health = driver.wait_for_health

    def wait_for_server_health(process) -> None:
        previous_origin = driver.ORIGIN
        try:
            driver.ORIGIN = SERVER_ORIGIN
            original_wait_for_health(process)
        finally:
            driver.ORIGIN = previous_origin

    original_listener_check = driver.assert_forgejo_listener

    def listener_and_proxy(process, port: int = 3130) -> None:
        original_listener_check(process, port=3131)
        start_proxy()
        verify_proxy_write_guard()

    original_merge_call = driver.run_local_test_merge_authority
    merge_calls = 0

    def merge_with_policy_race(*call_args, **call_kwargs):
        nonlocal merge_calls
        merge_calls += 1
        final_call = merge_calls == 3
        if final_call:
            with RACE_LOCK:
                RACE["armed"] = True
                RACE["policy_gets"] = 0
                RACE["mutation_status"] = None
                RACE["merge_posts"] = 0
                RACE["blocked_non_get_requests"] = 0
                RACE["forwarded_non_get_requests"] = 0
                RACE["events"] = []
        try:
            status, result = original_merge_call(*call_args, **call_kwargs)
        finally:
            if final_call:
                with RACE_LOCK:
                    RACE["armed"] = False
        if final_call:
            number = call_kwargs.get("number")
            host_token = call_kwargs.get("host_token")
            api = driver.make_api(host_token)
            _, pr, _ = api("GET", f"/repos/forge-admin/agent-forge-core/pulls/{number}")
            _, protections, _ = api(
                "GET", "/repos/forge-admin/agent-forge-core/branch_protections"
            )
            matching = [item for item in protections if item.get("rule_name") == "main"]
            readback = matching[0] if len(matching) == 1 else None
            with RACE_LOCK:
                race_snapshot = {key: value for key, value in RACE.items() if key != "armed"}
            summary = {
                "schema_version": 1,
                "scenario": "forgejo_live_branch_policy_changed_between_gate_reads",
                "gate_exit_status": status,
                "gate_result": result,
                "policy_readback_after_gate": readback,
                "pull_request_state_after_gate": pr.get("state"),
                "pull_request_merged_after_gate": pr.get("merged"),
                "race": race_snapshot,
                "credential_logged": False,
            }
            driver.write_json(
                Path(args.state_dir) / "evidence/live-policy-race-summary.json", summary
            )
        return status, result

    driver.create_app_ini = create_app_ini_on_3131
    driver.make_api = make_api_on_server
    driver.wait_for_health = wait_for_server_health
    driver.assert_forgejo_listener = listener_and_proxy
    driver.run_local_test_merge_authority = merge_with_policy_race

    run_error = None
    try:
        try:
            driver.run(args)
        except Exception as error:
            run_error = str(error)
    finally:
        stop_proxy()

    evidence = state_dir / "evidence/live-policy-race-summary.json"
    if not evidence.is_file():
        print(f"FAIL-CLOSED: live race evidence absent; run error={run_error!r}", file=sys.stderr)
        return 1
    summary = json.loads(evidence.read_text())
    race = summary["race"]
    rule = summary.get("policy_readback_after_gate") or {}
    result = summary["gate_result"]
    error = result.get("error") or {}
    expected_write_guard_probes = [
        {
            "method": "POST",
            "path": "/api/v1/repos/forge-admin/agent-forge-core/pulls/999999/merge?probe=guard",
            "http_status": 409,
            "blocked_non_get_count": 1,
            "forwarded_non_get_count": 0,
            "upstream_attempt_count": 0,
            "forwarded_to_forgejo": False,
        },
        {
            "method": "POST",
            "path": "/api/v1/repos/forge-admin/agent-forge-core/pulls/888888/merge?probe=partial",
            "http_status": 409,
            "blocked_non_get_count": 1,
            "forwarded_non_get_count": 0,
            "upstream_attempt_count": 0,
            "forwarded_to_forgejo": False,
        },
    ]
    good = (
        run_error is not None
        and "Rust local merge authority did not confirm the protected merge" in run_error
        and race.get("policy_gets") == 2
        and race.get("mutation_status") == 200
        and race.get("merge_posts") == 0
        and race.get("blocked_non_get_requests") == 0
        and race.get("forwarded_non_get_requests") == 0
        and race.get("gate_non_get_upstream_attempts") == 0
        and race.get("write_guard_probes") == expected_write_guard_probes
        and sorted(rule.get("merge_whitelist_usernames", []))
        == ["forge-admin", "forge-reviewer"]
        and result.get("verdict") == "rejected"
        and result.get("merge_authorized") is False
        and result.get("merge_attempted") is False
        and error.get("code") == "branch_protection_changed_during_gate"
        and summary.get("pull_request_state_after_gate") == "open"
        and summary.get("pull_request_merged_after_gate") is False
    )
    if not good:
        print(
            json.dumps({"run_error": run_error, "summary": summary}, sort_keys=True),
            file=sys.stderr,
        )
        return 1
    final = {
        "scenario": summary["scenario"],
        "forgejo_api_port": 3131,
        "gate_proxy_port": 3130,
        "gate_policy_reads": race["policy_gets"],
        "policy_mutation_http_status": race["mutation_status"],
        "merge_posts_from_gate": race["merge_posts"],
        "non_get_gate_requests_blocked": race["blocked_non_get_requests"],
        "non_get_gate_requests_forwarded": race["forwarded_non_get_requests"],
        "gate_non_get_upstream_attempts": race["gate_non_get_upstream_attempts"],
        "write_guard_probes": race["write_guard_probes"],
        "gate_verdict": result["verdict"],
        "gate_error_code": error["code"],
        "pull_request_state": summary["pull_request_state_after_gate"],
        "pull_request_merged": summary["pull_request_merged_after_gate"],
    }
    evidence = state_dir / "evidence"
    driver.write_json(evidence / "live-policy-race-result.json", final)
    print(json.dumps(final, sort_keys=True))
    print(f"Forgejo isolated state retained at {state_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
