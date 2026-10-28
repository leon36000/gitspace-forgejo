#!/usr/bin/env python3
"""Exercise the existing no-LLM adapter probe against a disposable Forgejo."""

from __future__ import annotations

import argparse
import errno
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_pr_merge_e2e as forgejo_helpers  # noqa: E402

OWNER = "forge-admin"
REPOSITORY = "mission-012-clamp"
FORGEJO_PORT = 3130
ORIGIN = f"http://127.0.0.1:{FORGEJO_PORT}"
NETWORK_DENIAL_ERRNOS = frozenset(
    {errno.ECONNREFUSED, errno.ENETUNREACH, errno.EHOSTUNREACH}
)
ROUTES = (
    {
        "name": "api",
        "method": "GET",
        "path": f"/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/1",
        "host_statuses": (404,),
    },
    {
        "name": "web",
        "method": "GET",
        "path": f"/{OWNER}/{REPOSITORY}/pulls/1",
        "host_statuses": (404,),
    },
    {
        "name": "git_receive_pack",
        "method": "POST",
        "path": f"/{OWNER}/{REPOSITORY}.git/git-receive-pack",
        "host_statuses": (401, 403),
    },
)
POSITIVE_CONTROLS = (
    {"name": "api_repository", "method": "GET", "path": f"/api/v1/repos/{OWNER}/{REPOSITORY}"},
    {"name": "web_repository", "method": "GET", "path": f"/{OWNER}/{REPOSITORY}"},
)
SHA_RE = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
HASH_RE = re.compile(r"^[0-9a-f]{64}$")
EXPECTED_POSITIVES = {
    "health": ("GET", "/api/healthz"),
    "api_repository": ("GET", f"/api/v1/repos/{OWNER}/{REPOSITORY}"),
    "web_repository": ("GET", f"/{OWNER}/{REPOSITORY}"),
}


def validate_live_egress_result(result: dict[str, Any]) -> bool:
    """Reject evidence unless host controls work and the isolated probes fail closed."""
    version = result.get("schema_version") if isinstance(result, dict) else None
    if (
        not isinstance(result, dict)
        or isinstance(version, bool)
        or version not in {1, 2, 3}
    ):
        raise ValueError("unsupported live egress evidence schema")
    if result.get("verdict") != "passed":
        raise ValueError("live egress result is not passed")

    forgejo = result.get("forgejo")
    if (
        not isinstance(forgejo, dict)
        or "17.0.0+gitea-1.22.0" not in str(forgejo.get("version", ""))
        or not isinstance(forgejo.get("binary_sha256"), str)
        or not HASH_RE.fullmatch(forgejo["binary_sha256"])
        or forgejo.get("listener") != f"127.0.0.1:{FORGEJO_PORT}"
    ):
        raise ValueError("Forgejo identity or listener evidence is invalid")

    positives = result.get("positive_controls")
    if not isinstance(positives, list):
        raise ValueError("host positive controls are missing")
    positive_by_name = {entry.get("name"): entry for entry in positives if isinstance(entry, dict)}
    expected_positive_names = {"health", "api_repository", "web_repository"}
    if set(positive_by_name) != expected_positive_names or any(
        positive_by_name[name].get("status") != 200
        or positive_by_name[name].get("method") != EXPECTED_POSITIVES[name][0]
        or positive_by_name[name].get("path") != EXPECTED_POSITIVES[name][1]
        for name in expected_positive_names
    ):
        raise ValueError("host route control failed")

    routes = result.get("routes")
    if not isinstance(routes, list):
        raise ValueError("route observations are missing")
    route_by_name = {entry.get("name"): entry for entry in routes if isinstance(entry, dict)}
    if set(route_by_name) != {route["name"] for route in ROUTES} or len(routes) != len(ROUTES):
        raise ValueError("route observations are incomplete or duplicated")
    connect_errors = result.get("sandbox_connect_errors")
    if version >= 2 and (
        not isinstance(connect_errors, list) or len(connect_errors) != len(ROUTES)
    ):
        raise ValueError("sandbox connection error observations are incomplete")
    if connect_errors is not None and (
        not isinstance(connect_errors, list) or len(connect_errors) != len(ROUTES)
    ):
        raise ValueError("sandbox connection error observations are malformed")
    for index, expected in enumerate(ROUTES):
        observed = route_by_name[expected["name"]]
        if observed.get("method") != expected["method"] or observed.get("path") != expected["path"]:
            raise ValueError("route observation does not match the Forgejo route probe")
        status = observed.get("host_status")
        if not isinstance(status, int) or isinstance(status, bool) or status not in expected["host_statuses"]:
            raise ValueError(f"host route control failed: {expected['name']}")
        observed_errno = observed.get("sandbox_errno")
        if observed_errno not in NETWORK_DENIAL_ERRNOS:
            raise ValueError(
                f"sandbox route failure does not confirm network isolation: {expected['name']}"
            )
        if connect_errors is not None:
            connect_error = connect_errors[index]
            if (
                not isinstance(connect_error, dict)
                or connect_error.get("port") != FORGEJO_PORT
                or connect_error.get("errno") not in NETWORK_DENIAL_ERRNOS
                or connect_error.get("error_type") != observed.get("sandbox_error_type")
                or observed_errno != connect_error.get("errno")
            ):
                raise ValueError(
                    f"sandbox route failure does not confirm network isolation: {expected['name']}"
                )
        if (
            observed.get("sandbox_blocked") is not True
            or observed.get("sandbox_status") is not None
            or observed_errno not in NETWORK_DENIAL_ERRNOS
            or observed.get("sandbox_target_port") != FORGEJO_PORT
        ):
            if observed.get("sandbox_target_port") != FORGEJO_PORT:
                raise ValueError("sandbox probe did not target the Forgejo listener")
            raise ValueError(f"sandbox route was not blocked: {expected['name']}")

    trusted = result.get("trusted_read_control")
    if (
        not isinstance(trusted, dict)
        or trusted.get("status") != 200
        or trusted.get("authorized") is not True
        or trusted.get("credential_passed_to_sandbox") is not False
    ):
        raise ValueError("trusted read control did not succeed with isolated credentials")
    if result.get("credential_visible") is not False:
        raise ValueError("sandbox credential visibility was not refused")
    if version >= 3 and result.get("host_systemd_bus_visible") is not False:
        raise ValueError("sandbox systemd bus is visible or unverified")
    if result.get("network_connected") is not False:
        raise ValueError("sandbox network access was not refused")

    namespaces = result.get("network_namespaces")
    if (
        not isinstance(namespaces, dict)
        or not isinstance(namespaces.get("host"), str)
        or not isinstance(namespaces.get("sandbox"), str)
        or namespaces["host"] == namespaces["sandbox"]
    ):
        raise ValueError("host and sandbox network namespaces are not distinct")

    before = result.get("branch_sha_before")
    after = result.get("branch_sha_after")
    if not isinstance(before, str) or not SHA_RE.fullmatch(before) or after != before:
        raise ValueError("read-only branch state changed during the probe")
    cleanup = result.get("cleanup")
    if (
        not isinstance(cleanup, dict)
        or cleanup.get("forgejo_exited") is not True
        or cleanup.get("listener_absent") is not True
    ):
        raise ValueError("Forgejo cleanup was not confirmed")
    return True


def write_json(path: Path, value: dict[str, Any]) -> None:
    data = json.dumps(value, sort_keys=True, indent=2).encode() + b"\n"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def validate_evidence_directory(evidence: Path) -> None:
    """Preserve the initial red probe while refusing unrelated evidence files."""
    if not evidence.is_dir():
        raise RuntimeError("--evidence-dir must be an existing directory")
    existing_evidence = {path.name for path in evidence.iterdir()}
    if existing_evidence - {"red-route-probe.log"}:
        raise RuntimeError("--evidence-dir may contain only the preserved red-route-probe.log")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def request_status(method: str, path: str, body: bytes | None = None) -> tuple[int, bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", FORGEJO_PORT, timeout=5)
    headers = {"Host": f"127.0.0.1:{FORGEJO_PORT}", "Connection": "close"}
    if method == "POST":
        headers["Content-Type"] = "application/x-git-receive-pack-request"
    try:
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        payload = response.read(128 * 1024 + 1)
        if len(payload) > 128 * 1024:
            raise RuntimeError("Forgejo route response exceeded 128 KiB")
        return response.status, payload
    finally:
        connection.close()


def listener_lines(port: int) -> list[str]:
    ss = Path("/usr/bin/ss")
    if not ss.is_file() or ss.is_symlink():
        raise RuntimeError("ss is required for listener cleanup readback")
    process = subprocess.run(
        [str(ss), "-H", "-ltnp", f"sport = :{port}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        timeout=5,
        check=True,
    )
    return process.stdout.splitlines()


def run_probe_in_sandbox(adapter_path: Path, credential_path: Path) -> dict[str, Any]:
    wrapper = r'''import json, os, socket, sys
sys.path.insert(0, "/opt")
connect_errors = []
original = socket.create_connection
def record_connection(address, *args, **kwargs):
    try:
        return original(address, *args, **kwargs)
    except OSError as error:
        connect_errors.append({"errno": error.errno, "error_type": type(error).__name__, "port": address[1]})
        raise
socket.create_connection = record_connection
from mission_012_test_agent_adapter import probe_sandbox_capabilities
checks = probe_sandbox_capabilities(__import__("pathlib").Path(sys.argv[1]), int(sys.argv[2]))
print(json.dumps({"checks": checks, "connect_errors": connect_errors, "network_namespace": os.readlink("/proc/self/ns/net")}, sort_keys=True))
'''
    bubblewrap = Path("/usr/bin/bwrap")
    prlimit = Path("/usr/bin/prlimit")
    if not bubblewrap.is_file() or not prlimit.is_file():
        raise RuntimeError("Bubblewrap and prlimit are required for this local isolation proof")
    command = [
        str(bubblewrap),
        "--die-with-parent",
        "--unshare-net",
        "--unshare-pid",
        "--new-session",
        "--ro-bind", "/usr", "/usr",
        "--dir", "/etc",
        "--ro-bind", "/etc/alternatives", "/etc/alternatives",
        "--symlink", "usr/lib", "/lib",
        "--symlink", "usr/bin", "/bin",
        "--tmpfs", "/tmp",
        "--tmpfs", "/home",
        "--tmpfs", "/run",
        "--dev", "/dev",
        "--proc", "/proc",
        "--dir", "/opt",
        "--ro-bind", str(adapter_path), "/opt/mission_012_test_agent_adapter.py",
        "--chdir", "/tmp",
        "--clearenv",
        "--setenv", "PATH", "/usr/bin:/bin",
        "--setenv", "HOME", "/tmp",
        "--setenv", "LANG", "C.UTF-8",
        "--setenv", "LC_ALL", "C.UTF-8",
        "--setenv", "PYTHONDONTWRITEBYTECODE", "1",
        str(prlimit),
        "--cpu=5",
        "--as=1073741824",
        "--nproc=16",
        "--nofile=64",
        "--fsize=1048576",
        "--",
        "/usr/bin/python3",
        "-c",
        wrapper,
        str(credential_path),
        str(FORGEJO_PORT),
    ]
    process = subprocess.run(
        command,
        cwd="/",
        env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=15,
        close_fds=True,
        check=False,
    )
    if process.returncode != 0:
        raise RuntimeError(f"isolated route probe failed with status {process.returncode}")
    if len(process.stdout) > 64 * 1024:
        raise RuntimeError("isolated route probe output exceeded 64 KiB")
    try:
        result = json.loads(process.stdout)
    except json.JSONDecodeError:
        raise RuntimeError("isolated route probe did not return a JSON observation") from None
    if not isinstance(result, dict):
        raise RuntimeError("isolated route probe returned an invalid JSON observation")
    return result


def repository_token(binary: Path, state: Path, *, name: str, scopes: str) -> str:
    raw = forgejo_helpers.call_admin(
        binary,
        state,
        [
            "admin", "user", "generate-access-token",
            "--username", OWNER,
            "--token-name", name,
            "--raw",
            "--scopes", scopes,
        ],
    ).decode("ascii").strip()
    if not raw or any(character.isspace() for character in raw):
        raise RuntimeError("Forgejo returned an invalid temporary token")
    return raw


def provision_repository(binary: Path, state: Path) -> tuple[str, str]:
    forgejo_helpers.call_admin(
        binary,
        state,
        [
            "admin", "user", "create",
            "--username", OWNER,
            "--random-password",
            "--email", f"{OWNER}@local.invalid",
            "--admin",
            "--must-change-password=false",
        ],
    )
    bootstrap_token = repository_token(
        binary, state, name="agent-egress-provision", scopes="write:user,write:repository"
    )
    bootstrap_api = forgejo_helpers.make_api(bootstrap_token)
    status, repository, _ = bootstrap_api(
        "POST",
        "/user/repos",
        {
            "name": REPOSITORY,
            "description": "Disposable direct-route egress fixture",
            "private": False,
            "auto_init": True,
            "default_branch": "main",
        },
    )
    bootstrap_api = None
    bootstrap_token = ""
    if status != 201 or not isinstance(repository, dict) or repository.get("name") != REPOSITORY:
        raise RuntimeError("Forgejo did not create the disposable route-probe repository")

    read_token = repository_token(
        binary, state, name="agent-egress-trusted-read", scopes="read:user,read:repository"
    )
    return read_token, str(repository.get("default_branch", "main"))


def host_route_statuses() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    health_status, _ = request_status("GET", "/api/healthz")
    api_status, api_body = request_status(
        "GET", f"/api/v1/repos/{OWNER}/{REPOSITORY}"
    )
    web_status, _ = request_status("GET", f"/{OWNER}/{REPOSITORY}")
    try:
        repository = json.loads(api_body)
    except json.JSONDecodeError:
        repository = None
    if (
        health_status != 200
        or api_status != 200
        or not isinstance(repository, dict)
        or repository.get("name") != REPOSITORY
        or web_status != 200
    ):
        raise RuntimeError("Forgejo host positive controls did not reach the expected routes")
    positives = [
        {"name": "health", "method": "GET", "path": "/api/healthz", "status": health_status},
        {"name": "api_repository", "method": "GET", "path": f"/api/v1/repos/{OWNER}/{REPOSITORY}", "status": api_status},
        {"name": "web_repository", "method": "GET", "path": f"/{OWNER}/{REPOSITORY}", "status": web_status},
    ]

    routes: list[dict[str, Any]] = []
    for route in ROUTES:
        body = b"0000" if route["name"] == "git_receive_pack" else None
        status, _ = request_status(route["method"], route["path"], body)
        routes.append({**route, "host_status": status})
    return positives, routes


def read_branch(api, branch: str) -> str:
    status, result, _ = api("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{branch}")
    if status != 200 or not isinstance(result, dict):
        raise RuntimeError("trusted read control could not read the fixture branch")
    commit = result.get("commit")
    sha = commit.get("id") if isinstance(commit, dict) else None
    if not isinstance(sha, str) or not SHA_RE.fullmatch(sha):
        raise RuntimeError("trusted read control returned an invalid branch SHA")
    return sha


def forgejo_version(binary: Path) -> str:
    result = subprocess.run(
        [str(binary), "--version"],
        cwd="/",
        env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=10,
        check=True,
    )
    if "17.0.0+gitea-1.22.0" not in result.stdout:
        raise RuntimeError("this local route probe requires Forgejo v17.0.0")
    return result.stdout.strip()


def stop_forgejo(process: subprocess.Popen[bytes] | None) -> dict[str, Any]:
    if process is None:
        return {"forgejo_exited": True, "returncode": None, "listener_absent": not listener_lines(FORGEJO_PORT)}
    if process.poll() is None:
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    absent = not listener_lines(FORGEJO_PORT)
    return {
        "forgejo_exited": process.poll() is not None,
        "returncode": process.returncode,
        "listener_absent": absent,
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    binary_argument = Path(args.forgejo_binary)
    if binary_argument.is_symlink():
        raise RuntimeError("--forgejo-binary must not be a symlink")
    binary = binary_argument.resolve(strict=True)
    state = Path(args.state_dir)
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("--state-dir must be a new direct child of /tmp")
    evidence = Path(args.evidence_dir).resolve(strict=True)
    validate_evidence_directory(evidence)
    if not binary.is_file():
        raise RuntimeError("--forgejo-binary must resolve to a regular file")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", FORGEJO_PORT)) == 0:
            raise RuntimeError("Forgejo probe port is already occupied")

    version = forgejo_version(binary)
    state.mkdir(mode=0o700)
    for name in ("data", "repositories", "log", "home", "tmp"):
        (state / name).mkdir(mode=0o700)
    forgejo_helpers.create_app_ini(state)
    credential_path = state / "merge-credential-canary"
    forgejo_helpers.write_private(
        credential_path, b"SYNTHETIC-NOT-A-FORGEJO-CREDENTIAL\n"
    )
    server_log = (state / "log/server.log").open("wb")
    process: subprocess.Popen[bytes] | None = None
    payload: dict[str, Any] | None = None
    stage = "start_forgejo"
    bootstrap_token = ""
    read_token = ""
    try:
        environment = {
            "PATH": "/usr/bin:/bin",
            "HOME": str(state / "home"),
            "TMPDIR": str(state / "tmp"),
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
        }
        process = subprocess.Popen(
            [
                str(binary), "--work-path", str(state),
                "--config", str(state / "custom/conf/app.ini"), "web",
            ],
            cwd=state,
            env=environment,
            stdout=server_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
            start_new_session=True,
        )
        forgejo_helpers.wait_for_health(process)
        forgejo_helpers.assert_forgejo_listener(process, FORGEJO_PORT)

        stage = "provision_disposable_repository"
        read_token, branch = provision_repository(binary, state)
        trusted_api = forgejo_helpers.make_api(read_token)
        branch_before = read_branch(trusted_api, branch)

        stage = "host_positive_route_controls"
        positives, host_routes = host_route_statuses()

        stage = "sandbox_route_probes"
        adapter_path = SCRIPT_DIR / "mission_012_test_agent_adapter.py"
        sandbox = run_probe_in_sandbox(adapter_path, credential_path)
        adapter_checks = sandbox.get("checks")
        connection_errors = sandbox.get("connect_errors")
        if not isinstance(adapter_checks, dict) or not isinstance(connection_errors, list):
            raise RuntimeError("sandbox did not return the adapter route probe observations")
        if len(connection_errors) != 3:
            raise RuntimeError("sandbox connection refusal was not observed for every route")

        stage = "trusted_readback"
        branch_after = read_branch(trusted_api, branch)
        read_token = ""
        trusted_api = None
        routes: list[dict[str, Any]] = []
        errors_by_order = connection_errors
        for index, (expected, observed) in enumerate(zip(ROUTES, host_routes, strict=True)):
            error = errors_by_order[index]
            routes.append(
                {
                    "name": expected["name"],
                    "method": expected["method"],
                    "path": expected["path"],
                    "host_status": observed["host_status"],
                    "sandbox_blocked": expected["name"] in adapter_checks.get("direct_forgejo_routes_blocked", []),
                    "sandbox_status": None,
                    "sandbox_errno": error.get("errno"),
                    "sandbox_error_type": error.get("error_type"),
                    "sandbox_target_port": error.get("port"),
                }
            )
        payload = {
            "schema_version": 3,
            "verdict": "passed",
            "forgejo": {
                "version": version,
                "binary_sha256": sha256(binary),
                "listener": f"127.0.0.1:{FORGEJO_PORT}",
            },
            "positive_controls": positives,
            "routes": routes,
            "trusted_read_control": {
                "status": 200,
                "authorized": True,
                "credential_passed_to_sandbox": False,
                "credential_scope": "read:user,read:repository",
                "route": f"GET /api/v1/repos/{OWNER}/{REPOSITORY}/branches/{branch}",
            },
            "credential_visible": adapter_checks.get("credential_visible"),
            "host_systemd_bus_visible": adapter_checks.get("host_systemd_bus_visible"),
            "network_connected": adapter_checks.get("network_connected"),
            "network_namespaces": {
                "host": os.readlink("/proc/self/ns/net"),
                "sandbox": sandbox.get("network_namespace"),
            },
            "branch_sha_before": branch_before,
            "branch_sha_after": branch_after,
            "cleanup": {},
            "limitations": [
                "The sandbox child invoked the existing local-test-adapter route probe only; no model was called.",
                "The trusted control was a read-only API call, not a merge-authority operation.",
                "The isolated Forgejo server and sandbox used the same host UID; this is not multi-tenant isolation.",
            ],
        }
        if adapter_checks.get("network_connected") is not False:
            raise RuntimeError("sandbox reported a network connection")
        if adapter_checks.get("credential_visible") is not False:
            raise RuntimeError("sandbox could read the synthetic credential canary")
        if adapter_checks.get("host_systemd_bus_visible") is not False:
            raise RuntimeError("sandbox exposed the host systemd bus path")
        if any(error.get("port") != FORGEJO_PORT for error in connection_errors):
            raise RuntimeError("sandbox attempts did not target the real Forgejo listener")
        stage = "validate_live_result"
    except BaseException as error:
        failure = {"stage": stage, "error_type": type(error).__name__}
        try:
            write_json(evidence / "failure.json", failure)
        except FileExistsError:
            pass
        raise
    finally:
        bootstrap_token = ""
        read_token = ""
        cleanup = stop_forgejo(process)
        server_log.close()
        if payload is not None:
            payload["cleanup"] = cleanup
            validate_live_egress_result(payload)
            write_json(evidence / "result.json", payload)
            probe_copy = evidence / "sandbox-probe.py"
            forgejo_helpers.write_private(probe_copy, adapter_path.read_bytes())
            write_json(
                evidence / "provenance.json",
                {
                    "head_commit": subprocess.run(
                        ["git", "rev-parse", "HEAD"],
                        cwd=forgejo_helpers.process_e2e.REPO,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        text=True,
                        timeout=5,
                        check=True,
                    ).stdout.strip(),
                    "driver_sha256": sha256(Path(__file__).resolve()),
                    "adapter_probe_sha256": sha256(adapter_path),
                    "forgejo_binary_sha256": payload["forgejo"]["binary_sha256"],
                    "bubblewrap_version": subprocess.run(
                        ["/usr/bin/bwrap", "--version"],
                        stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT,
                        text=True,
                        timeout=5,
                        check=True,
                    ).stdout.strip(),
                    "python_version": sys.version.split()[0],
                    "sandbox_network_policy": "Bubblewrap --unshare-net and --unshare-pid; adapter probe only; no credentials mounted",
                },
            )

    return payload


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    args = parser.parse_args()
    try:
        result = run(args)
    except Exception as error:
        print(json.dumps({"verdict": "failed", "error_type": type(error).__name__}, sort_keys=True))
        return 1
    print(json.dumps({"verdict": result["verdict"], "forgejo": result["forgejo"], "route_count": len(result["routes"]), "cleanup": result["cleanup"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
