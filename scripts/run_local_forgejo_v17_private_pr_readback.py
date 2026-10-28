#!/usr/bin/env python3
"""Run a local-only Forgejo v17 PR, repair, and webhook readback exercise.

The runner fails closed unless its named container is the pinned image on an
internal Docker network with no published ports. It creates a private test
repository and leaves its issue/branch/PR in that isolated instance. Tokens are
held in memory, passed to the reconciler over a pipe, and revoked on exit.
Candidate Python code is evaluated only in a networkless Bubblewrap sandbox.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import http.client
import http.server
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import uuid


CONTAINER = "agent-forge-forgejo-v17-live-poc-20261025"
NETWORK = "agent-forge-forgejo-v17-20261025"
IMAGE_ID = "sha256:06d38310de496a10186d952bdad5ca764c153c8023ede8dfa0944c79a1dc5412"
OWNER = "forge-admin"
REPOSITORY = "agent-forge-core"
CONTAINER_PORT = 3000
LOOPBACK_ORIGIN = "http://127.0.0.1:3130"
WORKSPACE = Path(__file__).resolve().parents[1]
WEBHOOK_BINARY = WORKSPACE / "target/debug/forgejo-webhook-local"
RECONCILE_BINARY = WORKSPACE / "target/debug/forgejo-reconcile-local"
MAX_API_BODY = 1_048_576

INITIAL_SOURCE = '''def parse_repository_coordinate(value):
    if not isinstance(value, str):
        raise ValueError("coordinate must be text")
    parts = value.split("/", 1)
    if len(parts) != 2 or not all(parts):
        raise ValueError("coordinate must be owner/repository")
    return parts[0], parts[1]
'''

FIXED_SOURCE = '''_ALLOWED = frozenset("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-")


def _valid_component(value):
    return (
        1 <= len(value) <= 100
        and value not in {".", ".."}
        and all(character in _ALLOWED for character in value)
    )


def parse_repository_coordinate(value):
    if not isinstance(value, str):
        raise ValueError("coordinate must be text")
    parts = value.split("/")
    if len(parts) != 2 or not all(_valid_component(part) for part in parts):
        raise ValueError("coordinate must be owner/repository with valid components")
    return parts[0], parts[1]
'''

TEST_SOURCE = '''import unittest

from src.repository_identity import parse_repository_coordinate


class RepositoryCoordinateTests(unittest.TestCase):
    def test_valid_coordinate(self):
        self.assertEqual(parse_repository_coordinate("forge-admin/agent-forge-core"),
                         ("forge-admin", "agent-forge-core"))

    def test_whitespace_is_rejected(self):
        for value in ("alice /repo", "alice/repo "):
            with self.subTest(value=value), self.assertRaises(ValueError):
                parse_repository_coordinate(value)

    def test_extra_path_component_is_rejected(self):
        with self.assertRaises(ValueError):
            parse_repository_coordinate("alice/repo/extra")
'''

VERIFIER_SOURCE = '''from repository_identity import parse_repository_coordinate

valid = {
    "forge-admin/agent-forge-core": ("forge-admin", "agent-forge-core"),
    "A_1/repo.v2": ("A_1", "repo.v2"),
}
invalid = ["alice /repo", "alice/repo ", "alice/repo/extra", "/repo", "alice/"]
failures = []
for coordinate, expected in valid.items():
    try:
        actual = parse_repository_coordinate(coordinate)
        if actual != expected:
            failures.append(f"valid coordinate changed: {coordinate!r}")
    except Exception as error:
        failures.append(f"valid coordinate rejected: {coordinate!r} ({type(error).__name__})")
for coordinate in invalid:
    try:
        parse_repository_coordinate(coordinate)
    except ValueError:
        continue
    except Exception as error:
        failures.append(f"invalid coordinate raised wrong error: {coordinate!r} ({type(error).__name__})")
    else:
        failures.append(f"invalid coordinate accepted: {coordinate!r}")
if failures:
    for failure in failures:
        print("FAIL: " + failure)
    raise SystemExit(1)
print("PASS: 2 valid and 5 invalid repository coordinates")
'''


class LocalRunError(RuntimeError):
    pass


def run(command: list[str], *, timeout: int = 20) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise LocalRunError(f"local command could not complete: {Path(command[0]).name}") from error


def docker_json(*arguments: str) -> object:
    result = run(["docker", *arguments])
    if result.returncode != 0:
        raise LocalRunError(f"Docker metadata command failed: {arguments[0]}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise LocalRunError("Docker returned invalid metadata JSON") from error


def inspect_isolated_container() -> tuple[str, str]:
    containers = docker_json("inspect", CONTAINER)
    if not isinstance(containers, list) or len(containers) != 1:
        raise LocalRunError("the dedicated Forgejo test container was not found uniquely")
    container = containers[0]
    if container.get("Image") != IMAGE_ID or not container.get("State", {}).get("Running"):
        raise LocalRunError("the named test container is not running the pinned Forgejo image")
    networks = container.get("NetworkSettings", {}).get("Networks", {})
    if set(networks) != {NETWORK}:
        raise LocalRunError("the test container is attached to an unexpected network")
    network_info = networks[NETWORK]
    container_ip = network_info.get("IPAddress", "")
    if not container_ip:
        raise LocalRunError("the isolated Forgejo container has no internal address")
    network_metadata = docker_json("network", "inspect", NETWORK)
    if (
        not isinstance(network_metadata, list)
        or len(network_metadata) != 1
        or network_metadata[0].get("Internal") is not True
    ):
        raise LocalRunError("the Forgejo test network is not marked internal")
    ports = container.get("NetworkSettings", {}).get("Ports", {})
    if any(bindings for bindings in ports.values()):
        raise LocalRunError("the Forgejo test container has a host-published port")
    env = dict(
        entry.split("=", 1)
        for entry in container.get("Config", {}).get("Env", [])
        if "=" in entry
    )
    if env.get("FORGEJO__server__ROOT_URL") != f"{LOOPBACK_ORIGIN}/":
        raise LocalRunError("Forgejo ROOT_URL does not match the local adapter origin")
    return container_ip, network_info["NetworkID"]


def forgejo_version() -> str:
    result = run(
        ["docker", "exec", CONTAINER, "/usr/local/bin/gitea", "--version"]
    )
    version = result.stdout.decode("utf-8", "replace").strip()
    if result.returncode != 0 or "17.0.0" not in version:
        raise LocalRunError("the isolated Forgejo binary is not version 17.0.0")
    return version


class ForgejoApi:
    def __init__(self, host: str, token: str, password: str, *, default_basic_auth: bool = False):
        self.host = host
        self.token = token
        self.password = password
        self.default_basic_auth = default_basic_auth

    def request(
        self,
        method: str,
        path: str,
        payload: dict[str, object] | None = None,
        expected: tuple[int, ...] = (200,),
        basic_auth: bool | None = None,
    ) -> tuple[int, object | None]:
        body = None if payload is None else json.dumps(payload).encode()
        if basic_auth is None:
            basic_auth = self.default_basic_auth
        credential = (
            "Basic " + base64.b64encode(f"{OWNER}:{self.password}".encode()).decode("ascii")
            if basic_auth
            else f"token {self.token}"
        )
        headers = {
            "Accept": "application/json",
            "Authorization": credential,
        }
        if body is not None:
            headers["Content-Type"] = "application/json"
        connection = http.client.HTTPConnection(self.host, CONTAINER_PORT, timeout=8)
        try:
            connection.request(method, "/api/v1" + path, body=body, headers=headers)
            response = connection.getresponse()
            raw = response.read(MAX_API_BODY + 1)
            if len(raw) > MAX_API_BODY:
                raise LocalRunError("Forgejo API response exceeded the local test limit")
            if response.status not in expected:
                detail = ""
                try:
                    error_body = json.loads(raw)
                    if isinstance(error_body, dict) and isinstance(error_body.get("message"), str):
                        detail = error_body["message"][:200]
                except json.JSONDecodeError:
                    pass
                detail = detail.replace(self.token, "[redacted]").replace(self.password, "[redacted]")
                suffix = f": {detail}" if detail else ""
                raise LocalRunError(
                    f"Forgejo API {method} {path} returned HTTP {response.status}{suffix}"
                )
            if not raw:
                return response.status, None
            try:
                return response.status, json.loads(raw)
            except json.JSONDecodeError as error:
                raise LocalRunError(f"Forgejo API {method} {path} returned non-JSON data") from error
        except OSError as error:
            raise LocalRunError("the internal Forgejo API connection failed") from error
        finally:
            connection.close()


class ProxyState:
    def __init__(self, upstream_ip: str):
        self.upstream_ip = upstream_ip
        self.requests: list[tuple[str, bool, int]] = []
        self.lock = threading.Lock()


class PrivateReadbackProxy(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, _format: str, *_arguments: object) -> None:
        return

    def do_GET(self) -> None:
        state: ProxyState = self.server.proxy_state  # type: ignore[attr-defined]
        authorization = self.headers.get("Authorization")
        match = re.fullmatch(
            rf"/api/v1/repos/{re.escape(OWNER)}/{re.escape(REPOSITORY)}/pulls/([1-9][0-9]*)",
            self.path,
        )
        if match is None:
            self.send_error(404)
            return
        connection = http.client.HTTPConnection(state.upstream_ip, CONTAINER_PORT, timeout=8)
        headers = {"Accept": "application/json"}
        if authorization is not None:
            headers["Authorization"] = authorization
        try:
            connection.request("GET", self.path, headers=headers)
            upstream = connection.getresponse()
            body = upstream.read(MAX_API_BODY + 1)
            if len(body) > MAX_API_BODY:
                self.send_error(502)
                return
            with state.lock:
                state.requests.append((self.path, authorization is not None, upstream.status))
            self.send_response(upstream.status)
            self.send_header("Content-Type", upstream.getheader("Content-Type", "application/json"))
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
        except OSError:
            self.send_error(502)
        finally:
            connection.close()


def api_file_path(relative_path: str) -> str:
    return f"/repos/{OWNER}/{REPOSITORY}/contents/{urllib.parse.quote(relative_path, safe='/')}"


def add_file(
    api: ForgejoApi,
    relative_path: str,
    contents: str,
    branch: str,
    message: str,
) -> dict[str, object]:
    _, result = api.request(
        "POST",
        api_file_path(relative_path),
        {
            "branch": branch,
            "content": base64.b64encode(contents.encode()).decode("ascii"),
            "message": message,
        },
        expected=(201,),
    )
    assert isinstance(result, dict)
    return result


def update_file(
    api: ForgejoApi,
    relative_path: str,
    contents: str,
    branch: str,
    message: str,
    sha: str,
) -> dict[str, object]:
    _, result = api.request(
        "PUT",
        api_file_path(relative_path),
        {
            "branch": branch,
            "content": base64.b64encode(contents.encode()).decode("ascii"),
            "message": message,
            "sha": sha,
        },
        expected=(200,),
    )
    assert isinstance(result, dict)
    return result


def read_file(api: ForgejoApi, relative_path: str, ref: str) -> tuple[dict[str, object], str]:
    query = urllib.parse.urlencode({"ref": ref})
    _, result = api.request("GET", api_file_path(relative_path) + "?" + query)
    if not isinstance(result, dict):
        raise LocalRunError(f"Forgejo did not return the requested file: {relative_path}")
    try:
        contents = base64.b64decode(result["content"], validate=False).decode("utf-8")
    except (KeyError, ValueError, UnicodeDecodeError) as error:
        raise LocalRunError(f"Forgejo returned invalid text content for {relative_path}") from error
    return result, contents


def returned_file_sha(result: dict[str, object]) -> str:
    direct_sha = result.get("sha")
    if isinstance(direct_sha, str):
        return direct_sha
    content = result.get("content")
    if isinstance(content, dict) and isinstance(content.get("sha"), str):
        return content["sha"]
    raise LocalRunError("Forgejo file response did not contain a commit object SHA")


def isolated_verifier(source: str) -> subprocess.CompletedProcess[bytes]:
    if not Path("/usr/bin/bwrap").is_file():
        raise LocalRunError("Bubblewrap is required to run candidate code without network access")
    with tempfile.TemporaryDirectory(prefix="forgejo-live-candidate-") as temp:
        root = Path(temp)
        (root / "src").mkdir()
        (root / "src/repository_identity.py").write_text(source)
        (root / "verify.py").write_text(VERIFIER_SOURCE)
        command = [
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
            "16777216",
            "--tmpfs",
            "/tmp",
            "--ro-bind",
            str(root),
            "/workspace",
            "--chdir",
            "/workspace",
            "--clearenv",
            "--setenv",
            "PYTHONDONTWRITEBYTECODE",
            "1",
            "--setenv",
            "PYTHONPATH",
            "/workspace/src",
            "--",
            "/usr/bin/python3",
            "/workspace/verify.py",
        ]
        return run(command, timeout=15)


def run_cli(journal_path: Path, delivery_id: str, token: str | None) -> tuple[int, bytes, bytes]:
    command = [str(RECONCILE_BINARY)]
    pass_fds: tuple[int, ...] = ()
    read_fd = -1
    write_fd = -1
    if token is not None:
        read_fd, write_fd = os.pipe()
        command.extend(["--host-credential-fd", str(read_fd)])
        pass_fds = (read_fd,)
    command.extend([str(journal_path), delivery_id])
    try:
        if token is not None:
            os.write(write_fd, token.encode("ascii"))
            os.close(write_fd)
            write_fd = -1
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={},
            pass_fds=pass_fds,
            start_new_session=True,
            close_fds=True,
        )
        if read_fd >= 0:
            os.close(read_fd)
            read_fd = -1
        try:
            stdout, stderr = process.communicate(timeout=20)
        except subprocess.TimeoutExpired as error:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
            raise LocalRunError("local reconciliation CLI exceeded its 20-second bound") from error
        return process.returncode, stdout, stderr
    except OSError as error:
        raise LocalRunError("local reconciliation CLI could not be launched") from error
    finally:
        for descriptor in (read_fd, write_fd):
            if descriptor >= 0:
                os.close(descriptor)


def run_receiver_event(pr_number: int, head_sha: str, base_sha: str, token: str, api_host: str) -> dict[str, object]:
    if not WEBHOOK_BINARY.is_file() or not RECONCILE_BINARY.is_file():
        raise LocalRunError("build forgejo-webhook-local and forgejo-reconcile-local before running")
    if not re.fullmatch(r"[0-9a-fA-F:.]+", api_host):
        raise LocalRunError("the Forgejo internal address is not a numeric IP literal")
    proxy_state = ProxyState(api_host)
    proxy = http.server.ThreadingHTTPServer(("127.0.0.1", 3130), PrivateReadbackProxy)
    proxy.proxy_state = proxy_state  # type: ignore[attr-defined]
    proxy_thread = threading.Thread(target=proxy.serve_forever, daemon=True)
    proxy_thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="forgejo-live-webhook-") as temp:
            scratch = Path(temp)
            secret = secrets.token_bytes(32)
            secret_path = scratch / "webhook.secret"
            descriptor = os.open(secret_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "wb") as secret_file:
                secret_file.write(secret)
            journal = scratch / "inbox.jsonl"
            reservation = socket.socket()
            reservation.bind(("127.0.0.1", 0))
            address, port = reservation.getsockname()
            reservation.close()
            receiver = subprocess.Popen(
                [str(WEBHOOK_BINARY), f"{address}:{port}", str(secret_path), str(journal)],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                env={},
                start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 5
                ready = False
                while time.monotonic() < deadline:
                    if receiver.poll() is not None:
                        raise LocalRunError("webhook receiver exited before readiness")
                    try:
                        connection = http.client.HTTPConnection(address, port, timeout=0.3)
                        connection.request(
                            "GET", "/forgejo/webhook", headers={"Content-Length": "0"}
                        )
                        response = connection.getresponse()
                        response.read()
                        connection.close()
                        if response.status == 405:
                            ready = True
                            break
                    except OSError:
                        pass
                    time.sleep(0.05)
                if not ready:
                    raise LocalRunError("webhook receiver did not become ready")

                delivery_id = f"live-forgejo-pr-{pr_number}-{uuid.uuid4().hex[:8]}"
                stale_head = hashlib.sha256((delivery_id + ":head").encode()).hexdigest()
                stale_base = hashlib.sha256((delivery_id + ":base").encode()).hexdigest()
                if stale_head == head_sha:
                    stale_head = "0" * 40
                if stale_base == base_sha:
                    stale_base = "f" * 40
                event = {
                    "action": "synchronize",
                    "repository": {
                        "full_name": f"{OWNER}/{REPOSITORY}",
                        "name": REPOSITORY,
                        "owner": {"login": OWNER},
                    },
                    "pull_request": {
                        "number": pr_number,
                        "state": "open",
                        "merged": False,
                        "head": {
                            "ref": "codex/local-readback",
                            "sha": stale_head,
                            "repo": {
                                "full_name": f"{OWNER}/{REPOSITORY}",
                                "name": REPOSITORY,
                                "owner": {"login": OWNER},
                            },
                        },
                        "base": {
                            "ref": "main",
                            "sha": stale_base,
                            "repo": {
                                "full_name": f"{OWNER}/{REPOSITORY}",
                                "name": REPOSITORY,
                                "owner": {"login": OWNER},
                            },
                        },
                    },
                }
                body = json.dumps(event, separators=(",", ":")).encode()
                signature = hmac.new(secret, body, hashlib.sha256).hexdigest()
                connection = http.client.HTTPConnection(address, port, timeout=5)
                connection.request(
                    "POST",
                    "/forgejo/webhook",
                    body=body,
                    headers={
                        "Content-Type": "application/json",
                        "X-Forgejo-Delivery": delivery_id,
                        "X-Forgejo-Event": "pull_request",
                        "X-Forgejo-Signature": signature,
                    },
                )
                accepted_response = connection.getresponse()
                accepted_body = json.loads(accepted_response.read())
                connection.close()
                if accepted_response.status != 202 or accepted_body.get("accepted") is not True:
                    raise LocalRunError("the live Forgejo PR webhook fixture was not accepted")
                receiver.send_signal(signal.SIGTERM)
                receiver.wait(timeout=5)
                if receiver.returncode != 0:
                    raise LocalRunError("webhook receiver did not stop cleanly")
                journal_before = journal.read_bytes()

                before_authenticated = len(proxy_state.requests)
                auth_code, auth_stdout, auth_stderr = run_cli(journal, delivery_id, token)
                authenticated_requests = proxy_state.requests[before_authenticated:]
                if token.encode() in auth_stdout or token.encode() in auth_stderr:
                    raise LocalRunError("host token leaked into reconciliation output")
                if auth_code != 0:
                    raise LocalRunError("credentialed reconciliation against live Forgejo failed")
                auth_result = json.loads(auth_stdout)
                if (
                    auth_result.get("verdict") != "current_state_read"
                    or auth_result.get("canonical_state", {}).get("head_sha") != head_sha
                    or auth_result.get("canonical_state", {}).get("base_sha") != base_sha
                    or auth_result.get("upstream_read_attempts") != 1
                    or auth_result.get("canonical_state", {}).get("html_url", "").startswith(LOOPBACK_ORIGIN + "/") is False
                ):
                    raise LocalRunError("live Forgejo current-state readback did not match the PR")
                if (
                    len(authenticated_requests) != 1
                    or authenticated_requests[0][0]
                    != f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{pr_number}"
                    or not authenticated_requests[0][1]
                    or authenticated_requests[0][2] != 200
                ):
                    raise LocalRunError("the loopback adapter did not observe one authenticated PR GET")
                if journal.read_bytes() != journal_before:
                    raise LocalRunError("credentialed readback modified the durable webhook inbox")

                before_anonymous = len(proxy_state.requests)
                anonymous_code, anonymous_stdout, anonymous_stderr = run_cli(
                    journal, delivery_id, None
                )
                anonymous_requests = proxy_state.requests[before_anonymous:]
                if anonymous_code != 1:
                    raise LocalRunError("anonymous private-repository read unexpectedly succeeded")
                anonymous_result = json.loads(anonymous_stdout)
                if (
                    anonymous_result.get("verdict") != "inconclusive"
                    or anonymous_result.get("canonical_state") is not None
                    or anonymous_result.get("error", {}).get("code") not in {"resource_not_found", "upstream_unauthorized"}
                ):
                    raise LocalRunError("anonymous read did not fail closed without canonical state")
                if (
                    len(anonymous_requests) != 1
                    or anonymous_requests[0][0]
                    != f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{pr_number}"
                    or anonymous_requests[0][1]
                    or anonymous_requests[0][2] not in {401, 404}
                ):
                    raise LocalRunError("Forgejo did not reject the one anonymous private PR GET")
                if token.encode() in anonymous_stdout or token.encode() in anonymous_stderr:
                    raise LocalRunError("host token leaked into anonymous reconciliation output")
                if journal.read_bytes() != journal_before:
                    raise LocalRunError("anonymous read modified the durable webhook inbox")

                return {
                    "delivery_id": delivery_id,
                    "webhook_status": accepted_response.status,
                    "authenticated": {
                        "exit_code": auth_code,
                        "verdict": auth_result["verdict"],
                        "upstream_read_attempts": auth_result["upstream_read_attempts"],
                        "head_sha_matches_live_api": auth_result["canonical_state"]["head_sha"] == head_sha,
                        "base_sha_matches_live_api": auth_result["canonical_state"]["base_sha"] == base_sha,
                        "one_target_get": len(authenticated_requests) == 1,
                        "authorization_present": authenticated_requests[0][1],
                        "token_output_leak": False,
                    },
                    "anonymous": {
                        "exit_code": anonymous_code,
                        "verdict": anonymous_result["verdict"],
                        "error": anonymous_result["error"]["code"],
                        "canonical_state_absent": anonymous_result.get("canonical_state") is None,
                        "one_target_get": len(anonymous_requests) == 1,
                        "authorization_absent": not anonymous_requests[0][1],
                        "http_status": anonymous_requests[0][2],
                    },
                    "inbox_unchanged_after_both_reads": journal.read_bytes() == journal_before,
                }
            finally:
                if receiver.poll() is None:
                    receiver.send_signal(signal.SIGTERM)
                    try:
                        receiver.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        receiver.kill()
                        receiver.wait()
    finally:
        proxy.shutdown()
        proxy.server_close()
        proxy_thread.join(timeout=2)


def main() -> int:
    container_ip, network_id = inspect_isolated_container()
    version = forgejo_version()
    health = http.client.HTTPConnection(container_ip, CONTAINER_PORT, timeout=5)
    health.request("GET", "/api/healthz")
    health_response = health.getresponse()
    health_response.read()
    health_status = health_response.status
    health.close()
    if health_status != 200:
        raise LocalRunError("the isolated Forgejo health endpoint did not return HTTP 200")

    created = run(
        [
            "docker", "exec", "--user", "1000:1000", CONTAINER,
            "/usr/local/bin/gitea", "admin", "user", "create",
            "--username", OWNER, "--email", "forge-admin@local.invalid",
            "--random-password", "--must-change-password=false",
        ]
    )
    if created.returncode != 0:
        stderr = created.stderr.decode("utf-8", "replace").lower()
        if "already exists" not in stderr:
            raise LocalRunError("could not create the local non-admin test owner")

    password = secrets.token_urlsafe(32)
    password_changed = run(
        [
            "docker", "exec", "--user", "1000:1000", CONTAINER,
            "/usr/local/bin/gitea", "admin", "user", "change-password",
            "--username", OWNER, "--password", password,
            "--must-change-password=false",
        ]
    )
    if password_changed.returncode != 0:
        raise LocalRunError("could not set a temporary local test password")

    # Password auth is used only for local fixture setup and token cleanup.
    # The read-only token is exercised through the real reconciler process.
    token_name = "codex-live-e2e-" + uuid.uuid4().hex[:12]
    api = ForgejoApi(container_ip, "", password, default_basic_auth=True)
    token_revoked = False
    repo_path = f"/repos/{OWNER}/{REPOSITORY}"
    try:
        generated = run(
            [
                "docker", "exec", "--user", "1000:1000", CONTAINER,
                "/usr/local/bin/gitea", "admin", "user", "generate-access-token",
                "--username", OWNER, "--token-name", token_name,
                "--scopes", "read:repository,read:user",
                "--raw",
            ]
        )
        token = generated.stdout.decode("ascii", "strict").strip()
        if generated.returncode != 0 or not token or len(token) > 4096:
            raise LocalRunError("could not create a bounded, scoped local API token")
        api.token = token

        status, existing_repo = api.request("GET", repo_path, expected=(200, 404))
        if status == 404:
            _, repo = api.request(
                "POST",
                "/user/repos",
                {
                    "name": REPOSITORY,
                    "description": "Local isolated Forgejo adapter integration fixture",
                    "private": True,
                    "auto_init": True,
                    "default_branch": "main",
                },
                expected=(201,),
            )
            if not isinstance(repo, dict) or repo.get("private") is not True:
                raise LocalRunError("Forgejo did not create the intended private test repository")
            _, issue = api.request(
                "POST",
                repo_path + "/issues",
                {
                    "title": "Reject whitespace and extra path components in repository coordinates",
                    "body": (
                        "REQ-IDENTITY-001 v1. Parse exactly `owner/repository`; reject whitespace, "
                        "empty components, and additional slash-separated components. Keep accepted "
                        "ASCII letters, digits, dot, underscore, and hyphen. Independent checks must "
                        "cover accepted and rejected coordinates."
                    ),
                },
                expected=(201,),
            )
            assert isinstance(issue, dict)
            issue_number = int(issue["number"])
            requirement = json.dumps(
                {
                    "id": "REQ-IDENTITY-001",
                    "version": 1,
                    "source_issue": issue_number,
                    "intent": "Parse exactly one owner/repository coordinate without whitespace.",
                    "invariants": ["two non-empty ASCII path components", "no extra slash"],
                    "acceptance": [
                        "forge-admin/agent-forge-core parses into its two components",
                        "whitespace in either component is rejected",
                        "empty or extra path components are rejected",
                    ],
                    "decision": "local-test-adapter fixture; no human arbitration claimed",
                },
                indent=2,
                sort_keys=True,
            ) + "\n"
            requirement_file = add_file(
                api,
                "requirements/REQ-IDENTITY-001.v1.json",
                requirement,
                "main",
                "docs: record versioned repository identity requirement",
            )
            branch = "codex/live-readback-" + uuid.uuid4().hex[:10]
            api.request(
                "POST",
                repo_path + "/branches",
                {"new_branch_name": branch, "old_branch_name": "main"},
                expected=(201,),
            )
            add_file(
                api, "src/repository_identity.py", INITIAL_SOURCE, branch,
                "feat: parse repository coordinates",
            )
            add_file(
                api, "tests/test_repository_identity.py", TEST_SOURCE, branch,
                "test: specify repository coordinate rules",
            )
            _, pr = api.request(
                "POST",
                repo_path + "/pulls",
                {
                    "head": branch,
                    "base": "main",
                    "title": "Reject malformed repository coordinates",
                    "body": f"Closes #{issue_number}. Implements REQ-IDENTITY-001.v1.",
                },
                expected=(201,),
            )
            assert isinstance(pr, dict)
            pr_number = int(pr["number"])
        else:
            if not isinstance(existing_repo, dict) or existing_repo.get("private") is not True:
                raise LocalRunError("the existing local repository is not a private test repository")
            _, pulls = api.request("GET", repo_path + "/pulls?state=open&limit=50")
            if not isinstance(pulls, list):
                raise LocalRunError("Forgejo did not list open pull requests")
            matching_pr = next(
                (
                    item for item in pulls
                    if isinstance(item, dict)
                    and item.get("title") == "Reject malformed repository coordinates"
                    and "REQ-IDENTITY-001.v1" in item.get("body", "")
                ),
                None,
            )
            if not isinstance(matching_pr, dict):
                raise LocalRunError("the existing repository has no resumable local test PR")
            pr_number = int(matching_pr["number"])
            branch = matching_pr["head"]["ref"]
            issue_match = re.search(r"Closes #(\d+)", matching_pr.get("body", ""))
            if issue_match is None:
                raise LocalRunError("the existing PR is not linked to its test issue")
            issue_number = int(issue_match.group(1))
            requirement_file, requirement_text = read_file(
                api, "requirements/REQ-IDENTITY-001.v1.json", "main"
            )
            requirement = json.loads(requirement_text)
            if requirement.get("id") != "REQ-IDENTITY-001" or requirement.get("version") != 1:
                raise LocalRunError("the existing versioned requirement does not match this run")

        _, current_pr = api.request("GET", repo_path + f"/pulls/{pr_number}")
        assert isinstance(current_pr, dict)
        head_sha = current_pr["head"]["sha"]
        base_sha = current_pr["base"]["sha"]
        if not current_pr.get("html_url", "").startswith(LOOPBACK_ORIGIN + "/"):
            raise LocalRunError("Forgejo API PR URL does not match the fixed adapter origin")

        _, commits = api.request(
            "GET",
            repo_path + "/commits?" + urllib.parse.urlencode({"sha": branch, "limit": 50}),
        )
        if not isinstance(commits, list):
            raise LocalRunError("Forgejo did not return the PR branch history")
        initial_commit = next(
            (
                item for item in commits
                if isinstance(item, dict)
                and item.get("commit", {}).get("message", "").splitlines()[:1]
                == ["feat: parse repository coordinates"]
            ),
            None,
        )
        if not isinstance(initial_commit, dict):
            raise LocalRunError("the initial candidate commit is absent from the PR history")
        initial_file, initial_source = read_file(
            api, "src/repository_identity.py", str(initial_commit["sha"])
        )
        if initial_source != INITIAL_SOURCE:
            raise LocalRunError("the initial candidate commit differs from the recorded fixture")
        first_run = isolated_verifier(initial_source)
        first_output = (first_run.stdout + first_run.stderr).decode("utf-8", "replace")
        if first_run.returncode == 0 or "invalid coordinate accepted" not in first_output:
            raise LocalRunError("the independent verifier did not expose an actionable initial defect")

        current_file, current_source = read_file(api, "src/repository_identity.py", branch)
        if current_source == INITIAL_SOURCE:
            updated = update_file(
                api,
                "src/repository_identity.py",
                FIXED_SOURCE,
                branch,
                "fix: reject whitespace and extra repository components",
                str(current_file["sha"]),
            )
        elif current_source == FIXED_SOURCE:
            updated = current_file
        else:
            raise LocalRunError("the current PR branch does not contain the known candidate")
        _, corrected_source = read_file(api, "src/repository_identity.py", branch)
        if corrected_source != FIXED_SOURCE:
            raise LocalRunError("the corrected source was not read back byte-for-byte from Forgejo")
        corrected_run = isolated_verifier(corrected_source)
        corrected_output = (corrected_run.stdout + corrected_run.stderr).decode("utf-8", "replace")
        if corrected_run.returncode != 0 or "PASS: 2 valid and 5 invalid" not in corrected_output:
            raise LocalRunError("the corrected code failed the independent networkless verifier")

        _, current_pr = api.request("GET", repo_path + f"/pulls/{pr_number}")
        if not isinstance(current_pr, dict):
            raise LocalRunError("Forgejo did not return the repaired pull request")
        head_sha = current_pr.get("head", {}).get("sha")
        base_sha = current_pr.get("base", {}).get("sha")
        if not isinstance(head_sha, str) or not isinstance(base_sha, str):
            raise LocalRunError("the repaired pull request has no current head or base SHA")

        live = run_receiver_event(pr_number, head_sha, base_sha, token, container_ip)

        _, tokens = api.request("GET", f"/users/{OWNER}/tokens?limit=50", expected=(200,))
        if not isinstance(tokens, list):
            raise LocalRunError("Forgejo did not return token metadata for revocation")
        local_tokens = [
            item
            for item in tokens
            if isinstance(item, dict)
            and isinstance(item.get("name"), str)
            and item["name"].startswith("codex-live-e2e-")
        ]
        if not any(item.get("name") == token_name for item in local_tokens):
            raise LocalRunError("the temporary scoped API token could not be located for revocation")
        for local_token in local_tokens:
            api.request(
                "DELETE",
                f"/users/{OWNER}/tokens/{local_token['id']}",
                expected=(204,),
                basic_auth=True,
            )
        _, remaining_tokens = api.request(
            "GET", f"/users/{OWNER}/tokens?limit=50", expected=(200,)
        )
        if not isinstance(remaining_tokens, list) or any(
            isinstance(item, dict)
            and isinstance(item.get("name"), str)
            and item["name"].startswith("codex-live-e2e-")
            for item in remaining_tokens
        ):
            raise LocalRunError("temporary local API tokens remained after cleanup")
        token_revoked = True

        summary = {
            "forgejo": {
                "version": version,
                "image_id": IMAGE_ID,
                "internal_network_id": network_id,
                "health_http": health_status,
                "host_published_ports": False,
            },
            "repository": {"full_name": f"{OWNER}/{REPOSITORY}", "private": True},
            "request": {
                "issue": issue_number,
                "requirement": "requirements/REQ-IDENTITY-001.v1.json",
                "requirement_sha": returned_file_sha(requirement_file),
                "pr": pr_number,
                "pr_url": current_pr["html_url"],
                "branch": branch,
                "head_sha": head_sha,
                "base_sha": base_sha,
                "initial_source_sha": initial_file["sha"],
                "corrected_source_sha": returned_file_sha(updated),
            },
            "worker": "local-test-adapter script; no LLM/AgentOS run",
            "verification": {
                "initial_failure_exit": first_run.returncode,
                "initial_failure_actionable": first_output.strip().splitlines(),
                "corrected_exit": corrected_run.returncode,
                "corrected_output": corrected_output.strip().splitlines(),
                "candidate_code_sandbox": "bubblewrap --unshare-net --unshare-pid, read-only source mount",
            },
            "live_reconciliation": live,
            "temporary_token_revoked": token_revoked,
            "merge": "not performed; this run does not establish a server-enforced CAS merge gate",
        }
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 0
    finally:
        if not token_revoked:
            # Best-effort credential revocation on every failure path; no token value is logged.
            try:
                _, tokens = api.request("GET", f"/users/{OWNER}/tokens?limit=50", expected=(200,))
                if isinstance(tokens, list):
                    matching = [
                        item
                        for item in tokens
                        if isinstance(item, dict)
                        and isinstance(item.get("name"), str)
                        and item["name"].startswith("codex-live-e2e-")
                    ]
                    for item in matching:
                        api.request(
                            "DELETE",
                            f"/users/{OWNER}/tokens/{item['id']}",
                            expected=(204,),
                            basic_auth=True,
                        )
                    _, remaining = api.request(
                        "GET", f"/users/{OWNER}/tokens?limit=50", expected=(200,)
                    )
                    token_revoked = isinstance(remaining, list) and not any(
                        isinstance(item, dict) and item.get("name") == token_name
                        for item in remaining
                    )
            except Exception:
                pass


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except LocalRunError as error:
        print(f"local Forgejo test failed: {error}", file=sys.stderr)
        raise SystemExit(1)
