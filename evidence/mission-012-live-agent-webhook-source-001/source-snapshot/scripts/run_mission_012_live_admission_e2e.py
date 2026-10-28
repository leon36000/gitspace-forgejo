#!/usr/bin/env python3
"""Exercise the real admission CLI against a private disposable Forgejo 17.

The test instance is loopback-only. Provisioning credentials remain in this
process; the read-only host credential is written under the private copied
project so the CLI can enforce its file checks. The script retains its state.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import io
import json
import os
from pathlib import Path
import shutil
import socket
import stat
import subprocess
import sys
import tarfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

ORIGIN = "http://127.0.0.1:3132"
OWNER = "forge-admin"
REPOSITORY = "mission-012-clamp"
FORGEJO_SHA256 = "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7"
READ_SCOPES = "read:repository,read:issue"


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def private_file(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def admin(binary: Path, state: Path, *args: str) -> bytes:
    result = subprocess.run(
        [str(binary), "--work-path", str(state), "--config",
         str(state / "custom/conf/app.ini"), *args],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=20, check=False,
    )
    if result.returncode:
        raise RuntimeError(f"Forgejo admin command refused (exit {result.returncode})")
    return result.stdout


def api(token: str | None):
    opener = build_opener(ProxyHandler({}), NoRedirect)

    def request(method: str, path: str, data: object | None = None):
        if not path.startswith("/") or path.startswith("//"):
            raise RuntimeError("API path was outside the pinned Forgejo instance")
        headers = {"Accept": "application/json"}
        if token is not None:
            headers["Authorization"] = f"token {token}"
        body = None
        if data is not None:
            headers["Content-Type"] = "application/json"
            body = json.dumps(data, separators=(",", ":")).encode()
        req = Request(ORIGIN + "/api/v1" + path, data=body, headers=headers, method=method)
        try:
            with opener.open(req, timeout=5) as response:
                raw = response.read(1024 * 1024 + 1)
                if len(raw) > 1024 * 1024:
                    raise RuntimeError("Forgejo API response exceeded 1 MiB")
                result = json.loads(raw) if raw else None
                response_headers = {key.lower(): value for key, value in response.headers.items()}
                return response.status, result, response_headers
        except HTTPError as error:
            return error.code, None, {key.lower(): value for key, value in error.headers.items()}
        except URLError:
            raise RuntimeError(f"Forgejo API transport failed for {path}") from None

    return request


def create_app_ini(state: Path) -> None:
    for name in ("data", "repositories", "custom/conf", "log", "tmp", "home"):
        (state / name).mkdir(parents=True, exist_ok=True, mode=0o700)
    values = [
        "[database]", "DB_TYPE = sqlite3", f"PATH = {state / 'data/forgejo.db'}", "",
        "[server]", f"APP_DATA_PATH = {state / 'data'}", "DOMAIN = 127.0.0.1",
        "HTTP_ADDR = 127.0.0.1", "HTTP_PORT = 3132", f"ROOT_URL = {ORIGIN}/",
        "PROTOCOL = http", "DISABLE_SSH = true", "START_SSH_SERVER = false",
        "LFS_START_SERVER = false", "", "[repository]",
        f"ROOT = {state / 'repositories'}", "DEFAULT_BRANCH = main", "",
        "[service]", "DISABLE_REGISTRATION = true", "REQUIRE_SIGNIN_VIEW = true", "",
        "[security]", "INSTALL_LOCK = true", f"SECRET_KEY = {os.urandom(48).hex()}",
        f"INTERNAL_TOKEN = {os.urandom(64).hex()}",
        f"LFS_JWT_SECRET = {os.urandom(32).hex()}", "", "[log]", "MODE = file",
        "LEVEL = Warn", f"ROOT_PATH = {state / 'log'}", "", "[actions]",
        "ENABLED = false", "", "[session]", "PROVIDER = memory", "",
        "[cache]", "ADAPTER = memory", "",
    ]
    private_file(state / "custom/conf/app.ini", "\n".join(values).encode())


def wait_for_health(process: subprocess.Popen[bytes]) -> None:
    opener = build_opener(ProxyHandler({}), NoRedirect)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Forgejo exited during startup ({process.returncode})")
        try:
            with opener.open(ORIGIN + "/api/healthz", timeout=1) as response:
                raw = response.read(16385)
                if len(raw) <= 16384 and json.loads(raw).get("status") == "pass":
                    return
        except Exception:
            pass
        time.sleep(0.2)
    raise RuntimeError("Forgejo health endpoint did not become ready in 30 seconds")


def ensure_owned_listener(process: subprocess.Popen[bytes]) -> None:
    result = subprocess.run(
        ["/usr/bin/ss", "-ltnp", "( sport = :3132 )"],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=5, check=False,
    )
    if result.returncode or str(process.pid) not in result.stdout.decode("utf-8", "replace"):
        raise RuntimeError("cannot attribute 127.0.0.1:3132 to this Forgejo process")


BUILD_INPUT_FILES = (
    "Cargo.toml", "Cargo.lock", "README.md", "build.rs",
    "rust-toolchain", "rust-toolchain.toml", "LICENSE",
)
BUILD_INPUT_DIRS = ("src", "tests", "policy")


_DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
_FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC
_DEST_FILE_FLAGS = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC


def _stat_identity(metadata: os.stat_result) -> tuple[int, int, int]:
    return metadata.st_dev, metadata.st_ino, stat.S_IFMT(metadata.st_mode)


def _stat_snapshot(metadata: os.stat_result) -> tuple[int, ...]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        stat.S_IFMT(metadata.st_mode),
        stat.S_IMODE(metadata.st_mode),
        metadata.st_nlink,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _snapshot_input_tree(source_fd: int) -> dict[str, tuple[int, ...]]:
    entries: dict[str, tuple[int, ...]] = {}

    def visit_directory(directory_fd: int, prefix: str) -> None:
        before = os.fstat(directory_fd)
        if not stat.S_ISDIR(before.st_mode):
            raise RuntimeError("build input directory must be a real directory")
        names = sorted(os.listdir(directory_fd))
        entries[prefix] = _stat_snapshot(before)
        for name in names:
            if name in (".", "..") or "/" in name or "\x00" in name:
                raise RuntimeError("build input tree contains an invalid path")
            relative = f"{prefix}/{name}" if prefix else name
            metadata = os.stat(name, dir_fd=directory_fd, follow_symlinks=False)
            entries[relative] = _stat_snapshot(metadata)
            if stat.S_ISDIR(metadata.st_mode):
                child_fd = os.open(name, _DIR_FLAGS, dir_fd=directory_fd)
                try:
                    if _stat_identity(os.fstat(child_fd)) != _stat_identity(metadata):
                        raise RuntimeError("build input directory changed during snapshot creation")
                    visit_directory(child_fd, relative)
                finally:
                    os.close(child_fd)
            elif stat.S_ISREG(metadata.st_mode):
                if metadata.st_nlink != 1:
                    raise RuntimeError("build input tree contains a linked or special file")
            else:
                raise RuntimeError("build input tree contains a linked or special file")
        after = os.fstat(directory_fd)
        if names != sorted(os.listdir(directory_fd)) or _stat_snapshot(before) != _stat_snapshot(after):
            raise RuntimeError("build input tree changed while creating the snapshot")

    for name in BUILD_INPUT_FILES:
        try:
            metadata = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
        except FileNotFoundError:
            continue
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            raise RuntimeError("build input file must be an unlinked regular file")
        entries[name] = _stat_snapshot(metadata)
    for name in BUILD_INPUT_DIRS:
        try:
            metadata = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
        except FileNotFoundError:
            continue
        if not stat.S_ISDIR(metadata.st_mode):
            raise RuntimeError("build input directory must be a real directory")
        child_fd = os.open(name, _DIR_FLAGS, dir_fd=source_fd)
        try:
            if _stat_identity(os.fstat(child_fd)) != _stat_identity(metadata):
                raise RuntimeError("build input directory changed during snapshot creation")
            visit_directory(child_fd, name)
        finally:
            os.close(child_fd)
    return entries


def _verify_destination_entry(parent_fd: int, name: str, child_fd: int, *, directory: bool) -> None:
    expected = os.fstat(child_fd)
    try:
        actual = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    except FileNotFoundError:
        raise RuntimeError("build snapshot destination changed during copy") from None
    if (
        _stat_identity(actual) != _stat_identity(expected)
        or (directory and not stat.S_ISDIR(actual.st_mode))
        or (not directory and not stat.S_ISREG(actual.st_mode))
    ):
        raise RuntimeError("build snapshot destination changed during copy")


def _copy_open_file(source_fd: int, destination_directory_fd: int, name: str) -> None:
    before = os.fstat(source_fd)
    if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
        raise RuntimeError("build input tree contains a linked or special file")
    destination_fd = os.open(
        name,
        _DEST_FILE_FLAGS,
        0o700 if before.st_mode & 0o111 else 0o600,
        dir_fd=destination_directory_fd,
    )
    try:
        while chunk := os.read(source_fd, 65536):
            view = memoryview(chunk)
            while view:
                written = os.write(destination_fd, view)
                if written <= 0:
                    raise OSError("short write copying bounded build input")
                view = view[written:]
        after = os.fstat(source_fd)
        before_state = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
        after_state = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
        if after_state != before_state:
            raise RuntimeError("build input file changed while creating the snapshot")
        os.fchmod(destination_fd, 0o700 if before.st_mode & 0o111 else 0o600)
        _verify_destination_entry(destination_directory_fd, name, destination_fd, directory=False)
    finally:
        os.close(destination_fd)


def _copy_directory_contents(source_fd: int, destination_directory_fd: int) -> None:
    before = os.fstat(source_fd)
    if not stat.S_ISDIR(before.st_mode):
        raise RuntimeError("build input directory must be a real directory")
    names = sorted(os.listdir(source_fd))
    for name in names:
        if name in (".", "..") or "/" in name or "\x00" in name:
            raise RuntimeError("build input tree contains an invalid path")
        entry = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
        if stat.S_ISDIR(entry.st_mode):
            _copy_unlinked_directory(source_fd, name, destination_directory_fd, entry)
        elif stat.S_ISREG(entry.st_mode):
            child_fd = os.open(name, _FILE_FLAGS, dir_fd=source_fd)
            try:
                opened = os.fstat(child_fd)
                if _stat_identity(opened) != _stat_identity(entry):
                    raise RuntimeError("build input file changed during snapshot creation")
                _copy_open_file(child_fd, destination_directory_fd, name)
            finally:
                os.close(child_fd)
        else:
            raise RuntimeError("build input tree contains a linked or special file")
    after = os.fstat(source_fd)
    if names != sorted(os.listdir(source_fd)) or (before.st_mtime_ns, before.st_ctime_ns) != (
        after.st_mtime_ns, after.st_ctime_ns
    ):
        raise RuntimeError("build input directory changed while creating the snapshot")


def _copy_unlinked_file(
    source_fd: int, name: str, destination_directory_fd: int, expected: os.stat_result
) -> None:
    source_file_fd = os.open(name, _FILE_FLAGS, dir_fd=source_fd)
    try:
        opened = os.fstat(source_file_fd)
        if _stat_identity(opened) != _stat_identity(expected):
            raise RuntimeError("build input file changed during snapshot creation")
        _copy_open_file(source_file_fd, destination_directory_fd, name)
    finally:
        os.close(source_file_fd)


def _copy_unlinked_directory(
    source_fd: int, name: str, destination_directory_fd: int, expected: os.stat_result
) -> None:
    child_fd = os.open(name, _DIR_FLAGS, dir_fd=source_fd)
    try:
        opened = os.fstat(child_fd)
        if _stat_identity(opened) != _stat_identity(expected):
            raise RuntimeError("build input directory changed during snapshot creation")
        os.mkdir(name, mode=0o700, dir_fd=destination_directory_fd)
        destination_child_fd = os.open(name, _DIR_FLAGS, dir_fd=destination_directory_fd)
        try:
            _copy_directory_contents(child_fd, destination_child_fd)
            _verify_destination_entry(destination_directory_fd, name, destination_child_fd, directory=True)
        finally:
            os.close(destination_child_fd)
    finally:
        os.close(child_fd)


def _copy_build_inputs(source: Path, destination_directory_fd: int) -> None:
    source_root = source.resolve(strict=True)
    source_fd = os.open(source_root, _DIR_FLAGS)
    try:
        initial_tree = _snapshot_input_tree(source_fd)
        for name in BUILD_INPUT_FILES:
            try:
                entry = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
            except FileNotFoundError:
                continue
            if not stat.S_ISREG(entry.st_mode):
                raise RuntimeError("build input file must be an unlinked regular file")
            _copy_unlinked_file(source_fd, name, destination_directory_fd, entry)
        for name in BUILD_INPUT_DIRS:
            try:
                entry = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
            except FileNotFoundError:
                continue
            if not stat.S_ISDIR(entry.st_mode):
                raise RuntimeError("build input directory must be a real directory")
            _copy_unlinked_directory(source_fd, name, destination_directory_fd, entry)
        if _snapshot_input_tree(source_fd) != initial_tree:
            raise RuntimeError("build input tree changed while creating the snapshot")
    finally:
        os.close(source_fd)


def _check_private_directory(fd: int) -> None:
    metadata = os.fstat(fd)
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
        raise RuntimeError("build snapshot directory must be private and owned by the current user")


def _open_child_directory(parent_fd: int, name: str) -> int:
    if "/" in name or name in ("", ".", ".."):
        raise RuntimeError("build snapshot directory name is invalid")
    child_fd = os.open(name, _DIR_FLAGS, dir_fd=parent_fd)
    try:
        _check_private_directory(child_fd)
    except BaseException:
        os.close(child_fd)
        raise
    return child_fd


def _create_child_directory(parent_fd: int, name: str) -> int:
    if "/" in name or name in ("", ".", ".."):
        raise RuntimeError("build snapshot directory name is invalid")
    os.mkdir(name, mode=0o700, dir_fd=parent_fd)
    return _open_child_directory(parent_fd, name)


def copy_project(source: Path, destination: Path) -> None:
    parent_fd = os.open(destination.parent, _DIR_FLAGS)
    destination_fd: int | None = None
    try:
        destination_fd = _create_child_directory(parent_fd, destination.name)
        _copy_build_inputs(source, destination_fd)
        os.mkdir(".runtime", mode=0o700, dir_fd=destination_fd)
        _verify_destination_entry(parent_fd, destination.name, destination_fd, directory=True)
    finally:
        if destination_fd is not None:
            os.close(destination_fd)
        os.close(parent_fd)


def prepare_build_snapshot(project: Path, state: Path) -> Path:
    """Create a no-link Cargo input snapshot that never carries project runtime state."""
    state_fd = os.open(state, _DIR_FLAGS)
    build_fd: int | None = None
    snapshot_fd: int | None = None
    try:
        _check_private_directory(state_fd)
        try:
            os.mkdir("build", mode=0o700, dir_fd=state_fd)
        except FileExistsError:
            pass
        build_fd = _open_child_directory(state_fd, "build")
        snapshot_fd = _create_child_directory(build_fd, "source-snapshot")
        _copy_build_inputs(project, snapshot_fd)
        _verify_destination_entry(build_fd, "source-snapshot", snapshot_fd, directory=True)
    finally:
        if snapshot_fd is not None:
            os.close(snapshot_fd)
        if build_fd is not None:
            os.close(build_fd)
        os.close(state_fd)
    return state / "build" / "source-snapshot"


def build_cli_isolated(
    project: Path, state: Path, source: Path, *, include_webhook_receiver: bool = False,
    include_mcp_host: bool = False,
) -> tuple[Path, bytes, Path | None]:
    # Reuse the repository's lockfile-verified registry staging and isolate
    # Cargo/build scripts from host home, credentials, and external network.
    import run_forgejo_mission_delivery_process_e2e as build_support

    build_project = prepare_build_snapshot(project, state)
    registry = state / "build/registry"
    build_support.copy_locked_registry(Path.home() / ".cargo/registry", registry)
    sysroot_run = subprocess.run(
        ["rustc", "--print", "sysroot"], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10, check=False,
    )
    if sysroot_run.returncode:
        raise RuntimeError("could not locate the configured Rust toolchain")
    sysroot = Path(sysroot_run.stdout.decode().strip()).resolve(strict=True)
    artifact_dir = state / "build/artifacts"
    artifact_dir.mkdir(mode=0o700)
    binary_names = ["forgejo-mission-local"]
    if include_webhook_receiver:
        binary_names.append("forgejo-webhook-local")
    if include_mcp_host:
        binary_names.append("mission-012-mcp-host")
    helper = r'''import io, os, shutil, subprocess, sys, tarfile
os.makedirs("/tmp/cargo-home/registry", exist_ok=True)
shutil.copytree("/bootstrap/registry/index", "/tmp/cargo-home/registry/index")
shutil.copytree("/bootstrap/registry/cache", "/tmp/cargo-home/registry/cache")
binary_names = sys.argv[1:]
command = ["/toolchain/bin/cargo", "build", "--offline", "--locked"]
for binary_name in binary_names:
    command.extend(["--bin", binary_name])
process = subprocess.Popen(
    command,
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT
)
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
artifacts = []
for binary_name in binary_names:
    binary = f"/build/target/debug/{binary_name}"
    if not os.path.isfile(binary) or not os.access(binary, os.X_OK):
        raise SystemExit(f"Cargo binary target missing: {binary_name}")
    size = os.path.getsize(binary)
    if size > 67108864:
        raise SystemExit(f"Cargo binary exceeded 64 MiB: {binary_name}")
    with open(binary, "rb") as source:
        artifacts.append((binary_name, source.read()))
archive = io.BytesIO()
with tarfile.open(fileobj=archive, mode="w") as tar:
    for binary_name, payload in artifacts:
        info = tarfile.TarInfo(binary_name)
        info.size = len(payload)
        info.mode = 0o700
        tar.addfile(info, io.BytesIO(payload))
sys.stdout.buffer.write(f"ARTIFACTS {len(artifacts)}\n".encode())
sys.stdout.buffer.write(archive.getvalue())
sys.stdout.buffer.flush()
'''
    mount_state = f"/tmp/{state.name}"
    command = [
        "/usr/bin/bwrap", "--die-with-parent", "--unshare-net", "--unshare-pid",
        "--new-session", "--ro-bind", "/usr", "/usr", "--dir", "/etc",
        "--ro-bind", "/etc/alternatives", "/etc/alternatives", "--symlink",
        "usr/lib", "/lib", "--symlink", "usr/bin", "/bin", "--size",
        "67108864", "--tmpfs", "/home", "--size", "134217728", "--tmpfs",
        "/tmp", "--dir", mount_state, "--dir", f"{mount_state}/project",
        "--ro-bind", str(build_project), f"{mount_state}/project", "--dir",
        "/toolchain", "--ro-bind", str(sysroot), "/toolchain", "--dev",
        "/dev", "--proc", "/proc", "--dir", "/tmp/cargo-home", "--dir",
        "/bootstrap", "--ro-bind", str(registry), "/bootstrap/registry",
        "--size", "1073741824", "--tmpfs", "/build", "--dir",
        "/build/target", "--size", "67108864", "--tmpfs", "/artifacts",
        "--clearenv", "--setenv", "PATH", "/toolchain/bin:/usr/bin:/bin",
        "--setenv", "HOME", "/tmp", "--setenv", "TMPDIR", "/tmp",
        "--setenv", "CARGO_HOME", "/tmp/cargo-home", "--setenv",
        "CARGO_TARGET_DIR", "/build/target", "--setenv", "CARGO_NET_OFFLINE",
        "true", "--setenv", "CARGO_BUILD_JOBS", "2", "--setenv", "RUSTC",
        "/toolchain/bin/rustc", "--chdir", f"{mount_state}/project",
        "/usr/bin/prlimit", "--cpu=240", "--as=4294967296", "--nproc=384",
        "--nofile=128", "--fsize=67108864", "--", "/usr/bin/python3",
        "-c", helper, *binary_names,
    ]
    result = subprocess.run(
        command, cwd=state, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, timeout=300, check=False,
    )
    if result.returncode:
        raise RuntimeError(f"sandboxed offline CLI build failed (exit {result.returncode})")
    end = result.stdout.find(b"\n")
    if end < 0:
        raise RuntimeError("sandboxed build omitted its artifact header")
    header = result.stdout[:end].decode("ascii", "strict").split()
    if header != ["ARTIFACTS", str(len(binary_names))]:
        raise RuntimeError("sandboxed build returned an unexpected artifact count")
    archive_bytes = result.stdout[end + 1 :]
    if len(archive_bytes) > 128 * 1024 * 1024:
        raise RuntimeError("sandboxed build artifact archive exceeded 128 MiB")
    artifacts: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:") as tar:
        members = tar.getmembers()
        if len(members) != len(binary_names):
            raise RuntimeError("sandboxed build artifact archive is incomplete")
        for member in members:
            if (
                member.name not in binary_names or not member.isfile()
                or member.size > 64 * 1024 * 1024 or member.name in artifacts
            ):
                raise RuntimeError("sandboxed build artifact archive contains an invalid member")
            stream = tar.extractfile(member)
            if stream is None:
                raise RuntimeError("sandboxed build artifact could not be read")
            payload = stream.read(64 * 1024 * 1024 + 1)
            if len(payload) != member.size:
                raise RuntimeError("sandboxed build artifact size did not match its archive entry")
            artifacts[member.name] = payload
    if set(artifacts) != set(binary_names):
        raise RuntimeError("sandboxed build artifact names did not match requested targets")
    paths: dict[str, Path] = {}
    for binary_name, payload in artifacts.items():
        artifact = artifact_dir / binary_name
        private_file(artifact, payload)
        artifact.chmod(0o700)
        paths[binary_name] = artifact
    if include_mcp_host:
        host_artifact_dir = state / "build/host-artifacts"
        host_artifact_dir.mkdir(mode=0o700)
        host_artifact = host_artifact_dir / "mission-012-mcp-host"
        private_file(host_artifact, paths["mission-012-mcp-host"].read_bytes())
        host_artifact.chmod(0o700)
    return paths["forgejo-mission-local"], result.stderr, paths.get("forgejo-webhook-local")


def signed_webhook_request(
    secret: bytes, delivery_id: str, body: bytes, *, signature_override: str | None = None
) -> bytes:
    signature = signature_override or hmac.new(secret, body, hashlib.sha256).hexdigest()
    headers = (
        "POST /forgejo/webhook HTTP/1.1\r\n"
        "Host: 127.0.0.1\r\n"
        f"Content-Length: {len(body)}\r\n"
        "Content-Type: application/json\r\n"
        f"X-Forgejo-Delivery: {delivery_id}\r\n"
        "X-Forgejo-Event: issues\r\n"
        f"X-Forgejo-Signature: {signature}\r\n\r\n"
    )
    return headers.encode("ascii") + body


def send_webhook(address: tuple[str, int], request: bytes) -> tuple[int, dict[str, object]]:
    with socket.create_connection(address, timeout=3) as stream:
        stream.settimeout(3)
        stream.sendall(request)
        stream.shutdown(socket.SHUT_WR)
        response = bytearray()
        while chunk := stream.recv(8192):
            response.extend(chunk)
            if len(response) > 64 * 1024:
                raise RuntimeError("webhook receiver response exceeded 64 KiB")
    header_bytes, body = bytes(response).split(b"\r\n\r\n", 1)
    status_line = header_bytes.split(b"\r\n", 1)[0].split()
    if len(status_line) < 2:
        raise RuntimeError("webhook receiver returned an invalid HTTP status line")
    status = int(status_line[1])
    decoded = json.loads(body) if body else {}
    if not isinstance(decoded, dict):
        raise RuntimeError("webhook receiver response body must be a JSON object")
    return status, decoded


def start_webhook_receiver(
    binary: Path, secret: Path, journal: Path, log_path: Path
) -> tuple[subprocess.Popen[bytes], tuple[str, int], object]:
    log_fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    log = os.fdopen(log_fd, "wb")
    for _ in range(5):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            address = reservation.getsockname()
        process = subprocess.Popen(
            [str(binary), f"{address[0]}:{address[1]}", str(secret), str(journal)],
            cwd=log_path.parent,
            env={"PATH": "/usr/bin:/bin", "HOME": str(log_path.parent / "home")},
            stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and process.poll() is None:
            try:
                status, _ = send_webhook(
                    (str(address[0]), int(address[1])),
                    b"GET /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n",
                )
                if status == 405:
                    return process, (str(address[0]), int(address[1])), log
            except (OSError, RuntimeError, ValueError, json.JSONDecodeError):
                time.sleep(0.05)
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    log.close()
    raise RuntimeError("actual webhook receiver did not pass loopback readiness after five attempts")


def stop_process(process: subprocess.Popen[bytes], log: object) -> int:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    log.close()
    if process.returncode is None:
        raise RuntimeError("receiver process exit status is unavailable after shutdown")
    return process.returncode


def run(args: argparse.Namespace) -> dict[str, object]:
    state = Path(args.state_dir)
    source = Path(args.repo_root).resolve(strict=True)
    binary_arg = Path(args.forgejo_binary)
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("--state-dir must be a new direct child of /tmp")
    if binary_arg.is_symlink() or not binary_arg.is_file():
        raise RuntimeError("--forgejo-binary must be a regular non-symlink file")
    binary = binary_arg.resolve(strict=True)
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    if binary_hash != FORGEJO_SHA256:
        raise RuntimeError("Forgejo binary hash differs from the previously identified local build")
    version = subprocess.run(
        [str(binary), "--version"], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10, check=False,
    )
    if version.returncode or b"17.0.0+gitea-1.22.0" not in version.stdout:
        raise RuntimeError("expected the locally identified Forgejo 17.0.0 binary")
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3132)) == 0:
            raise RuntimeError("127.0.0.1:3132 is occupied; no server was started")

    signed_delivery = bool(getattr(args, "signed_webhook_delivery", False))
    state.mkdir(mode=0o700)
    create_app_ini(state)
    project = state / "project"
    copy_project(source, project)
    server_log = (state / "forgejo-server.log").open("xb")
    process = subprocess.Popen(
        [str(binary), "--work-path", str(state), "--config",
         str(state / "custom/conf/app.ini"), "web"],
        cwd=state, env={"PATH": "/usr/bin:/bin", "HOME": str(state / "home"),
                        "TMPDIR": str(state / "tmp")},
        stdin=subprocess.DEVNULL, stdout=server_log, stderr=subprocess.STDOUT,
    )
    receiver_process = None
    receiver_log = None
    webhook_result: dict[str, object] | None = None
    try:
        wait_for_health(process)
        ensure_owned_listener(process)
        admin(binary, state, "admin", "user", "create", "--username", OWNER,
              "--random-password", "--email", f"{OWNER}@local.invalid", "--admin",
              "--must-change-password=false")
        token = admin(
            binary, state, "admin", "user", "generate-access-token", "--username", OWNER,
            "--token-name", "mission-012-admission-provisioner", "--raw", "--scopes",
            "write:user,write:repository,write:issue",
        ).decode().strip()
        if not token or any(character.isspace() for character in token):
            raise RuntimeError("Forgejo did not return a valid temporary provisioning token")
        provision = api(token)
        status, _, _ = provision("POST", "/user/repos", {
            "name": REPOSITORY, "description": "Isolated MISSION-012 admission test",
            "private": True, "auto_init": True, "default_branch": "main",
        })
        if status not in (200, 201):
            raise RuntimeError(f"private repository creation returned HTTP {status}")
        token = ""

        broken_source = (
            "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n"
            "    if value < minimum {\n        minimum\n"
            "    } else if value > maximum {\n"
            "        minimum // BUG: values above the range should clamp to maximum.\n"
            "    } else {\n        value\n    }\n}\n"
        ).encode()
        requirement = (source / "tests/fixtures/mission-012/requirements/"
                       "MISSION-012-CLAMP-001.json").read_bytes()
        files = {
            "Cargo.toml": b'[package]\nname = "mission012-clamp"\nversion = "0.1.0"\nedition = "2024"\n',
            "Cargo.lock": b'# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = "mission012-clamp"\nversion = "0.1.0"\n',
            "src/lib.rs": broken_source,
            "tests/acceptance.rs": (
                b'use mission012_clamp::clamp_to_range;\n\n'
                b'#[test]\nfn values_outside_the_range_clamp_to_the_nearest_boundary() {\n'
                b'    assert_eq!(clamp_to_range(-5, -2, 10), -2);\n'
                b'    assert_eq!(clamp_to_range(15, -2, 10), 10);\n}\n\n'
                b'#[test]\nfn values_inside_the_range_are_unchanged() {\n'
                b'    assert_eq!(clamp_to_range(4, -2, 10), 4);\n}\n'
            ),
            "requirements/MISSION-012-CLAMP-001.json": requirement,
        }
        for path, contents in files.items():
            status, _, _ = provision("POST", f"/repos/{OWNER}/{REPOSITORY}/contents/{path}", {
                "message": f"Add isolated mission fixture {path}",
                "content": base64.b64encode(contents).decode("ascii"), "branch": "main",
            })
            if status not in (200, 201):
                raise RuntimeError(f"fixture commit for {path} returned HTTP {status}")
        issue_body = (
            "Correct clamp_to_range so values above maximum return maximum. Preserve "
            "lower-bound and inclusive in-range behavior. Edit only src/lib.rs and run "
            "the protected acceptance tests before and after the fix."
        )
        status, issue, _ = provision(
            "POST", f"/repos/{OWNER}/{REPOSITORY}/issues",
            {"title": "MISSION-012: clamp upper boundary", "body": issue_body},
        )
        if status not in (200, 201) or not isinstance(issue, dict) or issue.get("number") != 1:
            raise RuntimeError("Forgejo did not create the expected open issue #1")
        admin(
            binary, state, "admin", "user", "create", "--username", "verifier-bot",
            "--random-password", "--email", "verifier-bot@local.invalid",
            "--must-change-password=false",
        )
        status, _, _ = provision(
            "PUT", f"/repos/{OWNER}/{REPOSITORY}/collaborators/verifier-bot",
            {"permission": "write"},
        )
        if status not in (200, 201, 204):
            raise RuntimeError(f"reviewer collaborator setup returned HTTP {status}")
        protection = {
            "rule_name": "main", "branch_name": "main", "enable_push": False,
            "required_approvals": 1, "dismiss_stale_approvals": True,
            "block_on_outdated_branch": True, "apply_to_admins": True,
            "enable_approvals_whitelist": True, "approvals_whitelist_username": ["verifier-bot"],
            "enable_merge_whitelist": True, "merge_whitelist_usernames": [OWNER],
            "enable_status_check": False,
        }
        status, _, _ = provision("POST", f"/repos/{OWNER}/{REPOSITORY}/branch_protections", protection)
        if status not in (200, 201):
            raise RuntimeError(f"main protection creation returned HTTP {status}")
        status, branch, _ = provision("GET", f"/repos/{OWNER}/{REPOSITORY}/branches/main")
        if status != 200 or not isinstance(branch, dict) or branch.get("protected") is not True:
            raise RuntimeError("Forgejo read-back did not mark main protected")
        base_sha = branch["commit"]["id"]
        provision = None

        read_token = admin(
            binary, state, "admin", "user", "generate-access-token", "--username", OWNER,
            "--token-name", "mission-012-read-only-admission", "--raw", "--scopes", READ_SCOPES,
        ).decode().strip()
        if not read_token or any(character.isspace() for character in read_token):
            raise RuntimeError("Forgejo did not return the bounded read-only token")
        credential = project / ".runtime/forgejo-read.token"
        private_file(credential, read_token.encode("ascii") + b"\n")
        admission = project / ".runtime/host-admission.json"

        cli, build_log, webhook_binary = build_cli_isolated(
            project, state, source, include_webhook_receiver=signed_delivery,
            include_mcp_host=bool(getattr(args, "build_mcp_host", False)),
        )
        delivery_id = None
        if signed_delivery:
            if webhook_binary is None:
                raise RuntimeError("isolated build did not return the requested webhook receiver")
            secret_path = project / ".runtime/forgejo-webhook.secret"
            secret = os.urandom(32)
            private_file(secret_path, secret)
            inbox = project / ".runtime/forgejo-webhook-inbox.jsonl"
            receiver_process, receiver_address, receiver_log = start_webhook_receiver(
                webhook_binary, secret_path, inbox, state / "forgejo-webhook.log"
            )
            delivery_id = f"mission012-live-{base_sha[:12]}"
            event_body = json.dumps({
                "action": "opened",
                "repository": {"full_name": f"{OWNER}/{REPOSITORY}"},
                "issue": {"number": 1, "state": "open"},
            }, sort_keys=True, separators=(",", ":")).encode()
            correct_signature = hmac.new(secret, event_body, hashlib.sha256).hexdigest()
            wrong_signature = "0" * 64 if correct_signature != "0" * 64 else "1" * 64
            refused_status, refused = send_webhook(
                receiver_address,
                signed_webhook_request(
                    secret, delivery_id, event_body, signature_override=wrong_signature
                ),
            )
            if (
                refused_status != 401
                or refused.get("error", {}).get("code") != "invalid_signature"
                or (inbox.exists() and inbox.read_bytes() != b"")
            ):
                raise RuntimeError("invalid webhook signature was not refused without journaling")
            accepted_status, accepted = send_webhook(
                receiver_address, signed_webhook_request(secret, delivery_id, event_body)
            )
            if (
                accepted_status != 202 or accepted.get("accepted") is not True
                or accepted.get("duplicate") is not False or accepted.get("sequence") != 1
            ):
                raise RuntimeError("valid signed issue webhook was not journaled as the first delivery")
            duplicate_status, duplicate = send_webhook(
                receiver_address, signed_webhook_request(secret, delivery_id, event_body)
            )
            if (
                duplicate_status != 202 or duplicate.get("accepted") is not True
                or duplicate.get("duplicate") is not True or duplicate.get("sequence") != 1
            ):
                raise RuntimeError("identical signed webhook retry was not accepted idempotently")
            receiver_exit = stop_process(receiver_process, receiver_log)
            if receiver_exit not in (0, -15):
                raise RuntimeError("webhook receiver did not stop cleanly")
            receiver_process = None
            receiver_log = None
            if len(inbox.read_bytes().splitlines()) != 1:
                raise RuntimeError("webhook retries created more than one inbox record")
            webhook_result = {
                "receiver_binary_sha256": hashlib.sha256(webhook_binary.read_bytes()).hexdigest(),
                "invalid_signature_http": refused_status,
                "invalid_signature_error": refused["error"]["code"],
                "invalid_signature_journal_bytes": 0,
                "accepted_http": accepted_status,
                "accepted_delivery_id": delivery_id,
                "accepted_sequence": accepted["sequence"],
                "duplicate_http": duplicate_status,
                "duplicate_delivery": duplicate["duplicate"],
                "inbox_records": 1,
                "secret_file_mode": oct(secret_path.stat().st_mode & 0o777),
            }
        common = [
            "--profile", "mission-012-clamp", "--issue", "1", "--requirement",
            "MISSION-012-CLAMP-001", "--revision", "1", "--allocation-id", args.allocation_id,
            "--workspace-id", "mission-012-clamp-issue-1",
        ]
        admission_arguments = list(common)
        if delivery_id is not None:
            admission_arguments.extend([
                "--delivery-id", delivery_id,
                "--requirement-sha256", hashlib.sha256(requirement).hexdigest(),
            ])
        journal = project / ".runtime/mission-execution.jsonl"
        anonymous = subprocess.run(
            [str(cli), *admission_arguments, "--host-admission-file", str(admission)],
            cwd=project, env={"PATH": "/usr/bin:/bin", "HOME": str(state / "home")},
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=20, check=False,
        )
        anonymous_result = json.loads(anonymous.stdout)
        anonymous_error = anonymous_result.get("error", {}).get("code")
        if anonymous.returncode == 0 or anonymous_error != "mission_source_forbidden" or journal.exists():
            raise RuntimeError("anonymous admission was not refused before claim state")
        authenticated = subprocess.run(
            [str(cli), *admission_arguments, "--host-admission-file", str(admission),
             "--host-credential-file", str(credential)],
            cwd=project, env={"PATH": "/usr/bin:/bin", "HOME": str(state / "home")},
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=20, check=False,
        )
        if authenticated.returncode != 0:
            raise RuntimeError(f"authenticated admission refused (exit {authenticated.returncode})")
        if read_token.encode("ascii") in authenticated.stdout:
            raise RuntimeError("host credential appeared in CLI output")
        if not admission.is_file() or admission.stat().st_mode & 0o777 != 0o600:
            raise RuntimeError("host admission DTO is missing or not owner-only")
        if read_token.encode("ascii") in admission.read_bytes() or read_token.encode("ascii") in journal.read_bytes():
            raise RuntimeError("host credential appeared in admission or journal state")
        result = json.loads(authenticated.stdout)
        expected_mode = "webhook_delivery" if signed_delivery else "direct_local"
        if result.get("profile_id") != "mission-012-clamp" or result.get("admission_mode") != expected_mode:
            raise RuntimeError("live CLI returned an unexpected profile or admission mode")
        dto = result["host_admission"]
        if (
            dto.get("scope") != {
                "origin": ORIGIN, "owner": OWNER, "repository": REPOSITORY, "branch": "main"
            }
            or dto.get("issue_number") != 1
            or dto.get("base_commit") != base_sha
            or dto.get("requirement_path") != "requirements/MISSION-012-CLAMP-001.json"
            or dto.get("allocation", {}).get("allocation_id") != args.allocation_id
            or dto.get("allocation", {}).get("execution_generation") != 1
        ):
            raise RuntimeError("live admission DTO did not preserve the pinned profile and claim")
        requirement_hash = dto["requirement_sha256"]
        if requirement_hash != hashlib.sha256(requirement).hexdigest():
            raise RuntimeError("admitted requirement digest differs from protected commit bytes")
        admission_replay = None
        if delivery_id is not None:
            replayed = subprocess.run(
                [str(cli), *admission_arguments, "--host-admission-file", str(admission),
                 "--host-credential-file", str(credential)],
                cwd=project, env={"PATH": "/usr/bin:/bin", "HOME": str(state / "home")},
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                timeout=20, check=False,
            )
            if replayed.returncode != 0:
                raise RuntimeError("idempotent signed-delivery admission replay failed")
            replay_result = json.loads(replayed.stdout)
            if (
                replay_result.get("verdict") != "already_claimed"
                or replay_result.get("host_admission") != dto
                or replay_result.get("allocation_replayed") is not True
                or len(inbox.read_bytes().splitlines()) != 1
            ):
                raise RuntimeError("delivery replay changed the durable admission or inbox")
            admission_replay = {
                "verdict": replay_result["verdict"],
                "allocation_replayed": replay_result["allocation_replayed"],
                "host_admission_sha256": hashlib.sha256(admission.read_bytes()).hexdigest(),
                "inbox_records_after_replay": 1,
            }
        token_mode = oct(credential.stat().st_mode & 0o777)
        if token_mode != "0o600":
            raise RuntimeError("host credential file is not owner-only")
        return {
            "forgejo_version": "17.0.0+gitea-1.22.0", "forgejo_binary_sha256": binary_hash,
            "origin": ORIGIN, "repository_private": True,
            "anonymous_admission": anonymous_error,
            "host_token_scopes": READ_SCOPES, "host_token_in_output_or_admission_state": False,
            "issue_number": 1, "issue_state": "open", "protected_branch": "main",
            "base_commit": base_sha, "requirement_sha256": requirement_hash,
            "allocation_id": args.allocation_id, "admission_mode": result["admission_mode"],
            "webhook": webhook_result,
            "admission_replay": admission_replay,
            "admission_file_mode": oct(admission.stat().st_mode & 0o777),
            "credential_file_mode": token_mode,
            "admitted_context_paths": [item["path"] for item in dto["request"]["context"]["manifest"]["sources"]],
            "context_estimated_tokens": dto["request"]["context"]["manifest"]["estimated_tokens"],
            "host_admission_sha256": hashlib.sha256(admission.read_bytes()).hexdigest(),
            "private_state_dir": str(state),
            "isolated_build_log_sha256": hashlib.sha256(build_log).hexdigest(),
        }
    finally:
        if receiver_process is not None and receiver_log is not None:
            stop_process(receiver_process, receiver_log)
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        server_log.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(Path(__file__).resolve().parents[1]))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--allocation-id", default="mission012-live-admission-001")
    parser.add_argument(
        "--signed-webhook-delivery", action="store_true",
        help="admit from one HMAC-signed issue delivery through the real local receiver",
    )
    parser.add_argument(
        "--build-mcp-host", action="store_true",
        help="also build the exact MCP host consumed by the following live-agent stage",
    )
    args = parser.parse_args()
    try:
        result = run(args)
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    print(f"Private Forgejo state retained at {args.state_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
