#!/usr/bin/env python3
"""Probe a host-attested systemd scope containing an isolated Bubblewrap child."""

from __future__ import annotations

import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import selectors
import socket
import subprocess
import sys
import tempfile
import threading
import time
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import systemd_process_scope as process_scope  # noqa: E402

GATE_ROUTE = "/api/v1/repos/forge-admin/agent-forge-core/pulls/1"
AGENT_ROUTES = [
    ("GET", GATE_ROUTE),
    ("GET", "/forge-admin/agent-forge-core/pulls/1"),
    ("POST", "/forge-admin/agent-forge-core.git/git-receive-pack"),
]
SYNTHETIC_TOKEN = b"CONT220-SYNTHETIC-MERGE-CREDENTIAL-DO-NOT-USE"


class FixtureHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self) -> None:
        self.server.requests.append(("GET", self.path))  # type: ignore[attr-defined]
        body = b"ok"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:
        self.server.requests.append(("POST", self.path))  # type: ignore[attr-defined]
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.send_header("Connection", "close")
        self.end_headers()

    def log_message(self, _format: str, *_args: object) -> None:
        return


def cgroup_path() -> str:
    entries = [
        line[3:]
        for line in Path("/proc/self/cgroup").read_text(encoding="ascii").splitlines()
        if line.startswith("0::/")
    ]
    if len(entries) != 1:
        raise RuntimeError("expected one cgroup-v2 membership")
    return entries[0]


def read_agent_line(process: subprocess.Popen[bytes], timeout: float) -> bytes:
    if process.stdout is None:
        raise RuntimeError("scoped process stdout is unavailable")
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        if not selector.select(timeout):
            raise TimeoutError("isolated child did not publish its probe result")
        line = process.stdout.readline(64 * 1024 + 1)
        if not line or len(line) > 64 * 1024 or not line.endswith(b"\n"):
            raise RuntimeError("isolated child result is empty, oversized, or unterminated")
        return line
    finally:
        selector.close()


def main() -> int:
    if os.geteuid() == 0:
        raise RuntimeError("run as the ordinary local user; root is outside this probe's model")
    host_netns = os.readlink("/proc/self/ns/net")
    host_pidns = os.readlink("/proc/self/ns/pid")
    fixture = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    fixture.daemon_threads = True
    fixture.requests = []  # type: ignore[attr-defined]
    fixture_thread = threading.Thread(target=fixture.serve_forever, daemon=True)
    fixture_thread.start()
    port = fixture.server_address[1]

    with tempfile.TemporaryDirectory(prefix="continuation-220-canary-") as scratch_name:
        scratch = Path(scratch_name)
        secret_file = scratch / "merge-credential-canary"
        secret_file.write_bytes(SYNTHETIC_TOKEN)
        secret_file.chmod(0o600)
        token_hash = hashlib.sha256(SYNTHETIC_TOKEN).hexdigest()
        read_fd, write_fd = os.pipe()
        os.set_inheritable(read_fd, True)
        try:
            os.write(write_fd, SYNTHETIC_TOKEN)
        finally:
            os.close(write_fd)
        canary_pipe_link = os.readlink(f"/proc/self/fd/{read_fd}")

        child_code = r'''import hashlib, json, os, socket, sys, time
port = int(sys.argv[1])
file_path = sys.argv[2]
pipe_link = sys.argv[3]
token_hash = sys.argv[4]
routes = [
    ("GET", f"/api/v1/repos/forge-admin/agent-forge-core/pulls/1"),
    ("GET", "/forge-admin/agent-forge-core/pulls/1"),
    ("POST", "/forge-admin/agent-forge-core.git/git-receive-pack"),
]
attempts = []
for method, path in routes:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1.0) as connection:
            request = f"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
            connection.sendall(request.encode("ascii"))
            attempts.append({"method": method, "path": path, "connected": True})
    except OSError as error:
        attempts.append({"method": method, "path": path, "connected": False, "errno": error.errno, "error_type": type(error).__name__})
visible = False
pipe_read = False
for entry in os.listdir("/proc/self/fd"):
    try:
        descriptor = os.path.join("/proc/self/fd", entry)
        if os.readlink(descriptor) == pipe_link:
            visible = True
            try:
                pipe_read = hashlib.sha256(os.read(int(entry), 4096)).hexdigest() == token_hash
            except OSError:
                pass
    except OSError:
        pass
file_canary_read = False
try:
    file_canary_read = hashlib.sha256(open(file_path, "rb").read()).hexdigest() == token_hash
except OSError:
    pass
environment_canary = any(hashlib.sha256(value.encode()).hexdigest() == token_hash for value in os.environ.values())
print(json.dumps({
    "route_attempts": attempts,
    "credential_pipe_visible": visible,
    "credential_pipe_read": pipe_read,
    "filesystem_canary_read": file_canary_read,
    "environment_canary": environment_canary,
    "network_namespace": os.readlink("/proc/self/ns/net"),
    "pid_namespace": os.readlink("/proc/self/ns/pid"),
    "cgroup": next(line[3:] for line in open("/proc/self/cgroup") if line.startswith("0::/")),
}, sort_keys=True), flush=True)
time.sleep(1.5)
'''
        bubblewrap_command = [
            "/usr/bin/bwrap",
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
            "--dev", "/dev",
            "--proc", "/proc",
            "--chdir", "/tmp",
            "--clearenv",
            "--setenv", "PATH", "/usr/bin:/bin",
            "--setenv", "HOME", "/tmp",
            "--setenv", "LANG", "C.UTF-8",
            "--setenv", "LC_ALL", "C.UTF-8",
            "--setenv", "PYTHONDONTWRITEBYTECODE", "1",
            "/usr/bin/prlimit",
            "--cpu=5", "--as=1073741824", "--nproc=16", "--nofile=64", "--fsize=1048576",
            "--", "/usr/bin/python3", "-c", child_code,
            str(port), str(secret_file), canary_pipe_link, token_hash,
        ]
        environment = {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"}
        scope = process_scope.prepare_process_scope(bubblewrap_command, environment, 12)
        started = process_scope.launch_scoped_process(
            scope,
            bubblewrap_command,
            environment,
            cwd=ROOT,
            timeout=12,
            merge_stderr=True,
        )
        if started.process is None or started.cgroup_path is None:
            fixture.shutdown()
            fixture.server_close()
            raise RuntimeError("host systemd scope did not attest the Bubblewrap command")
        process = started.process
        process.stdin = None  # launch_scoped_process closed its command-envelope pipe after attestation.
        cleanup = "not_finished"
        try:
            line = read_agent_line(process, 8)
            agent = json.loads(line.decode("utf-8"))
            with urlopen(f"http://127.0.0.1:{port}{GATE_ROUTE}", timeout=3) as response:
                gate_status = response.status
                gate_body = response.read(16).decode("ascii")
            remaining, _ = process.communicate(timeout=5)
            cleanup = process_scope.finish_process_scope(scope, started.cgroup_path)
        except BaseException:
            process_scope._abort_scope_process(process, scope)
            raise
        host_token = os.read(read_fd, 4096)
        os.close(read_fd)
        read_fd = -1
        observed_requests = list(fixture.requests)  # type: ignore[attr-defined]
        observed = {
            "host_network_namespace": host_netns,
            "host_pid_namespace": host_pidns,
            "scope_unit": scope.unit,
            "attested_cgroup_path": started.cgroup_path,
            "scope_exit_code": process.returncode,
            "scope_cleanup": cleanup,
            "agent": agent,
            "gate": {"status": gate_status, "body": gate_body, "route": GATE_ROUTE},
            "fixture_requests": observed_requests,
            "host_retained_synthetic_credential": host_token == SYNTHETIC_TOKEN,
            "host_filesystem_canary_path": str(secret_file),
        }
        errors = []
        if process.returncode != 0 or remaining:
            errors.append("scoped command did not exit cleanly with bounded output")
        if cleanup != "clean":
            errors.append(f"scope cleanup was {cleanup!r}")
        if agent["network_namespace"] == host_netns or agent["pid_namespace"] == host_pidns:
            errors.append("Bubblewrap did not create distinct network and PID namespaces")
        if agent["cgroup"].strip() != started.cgroup_path:
            errors.append("Bubblewrap child did not remain in the attested cgroup")
        if any(item["connected"] for item in agent["route_attempts"]):
            errors.append("agent child connected to a loopback Forgejo-like route")
        if agent["credential_pipe_visible"] or agent["credential_pipe_read"] or agent["filesystem_canary_read"] or agent["environment_canary"]:
            errors.append("agent child could observe a synthetic credential canary")
        if gate_status != 200 or gate_body != "ok":
            errors.append("host-side gate request was not served while the child was active")
        if observed_requests != [("GET", GATE_ROUTE)]:
            errors.append(f"fixture observed an unexpected request set: {observed_requests!r}")
        if not observed["host_retained_synthetic_credential"]:
            errors.append("host no longer retained its synthetic credential pipe")
        fixture.shutdown()
        fixture.server_close()
        fixture_thread.join(timeout=3)
        if fixture_thread.is_alive():
            errors.append("loopback fixture thread did not stop")
        if errors:
            raise RuntimeError("; ".join(errors) + "\n" + json.dumps(observed, sort_keys=True))
        print(json.dumps(observed, sort_keys=True))
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
