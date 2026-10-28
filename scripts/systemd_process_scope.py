"""Attest and clean one transient user-systemd cgroup for a local process tree."""

from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import stat
import subprocess
import sys
import time

SYSTEMD_RUN = Path("/usr/bin/systemd-run")
SYSTEMCTL = Path("/usr/bin/systemctl")
SCOPE_EXEC = Path(__file__).with_name("systemd_scope_exec.py")
STARTUP_GRACE_SECONDS = 3.0
STOP_TIMEOUT_SECONDS = 4.0


class ProcessScopeUnavailable(RuntimeError):
    pass


@dataclass(frozen=True)
class ProcessScope:
    unit: str
    nonce: str
    manager_environment: dict[str, str]
    expected_prefix: str
    command: list[str]
    owner_unit: str | None = None


@dataclass(frozen=True)
class ProcessScopeSnapshot:
    """Attested scope identity and the active members present at one instant."""

    unit: str
    cgroup_path: str
    active_pids: tuple[int, ...]


@dataclass(frozen=True)
class ScopedProcessStart:
    process: subprocess.Popen[bytes] | None
    cgroup_path: str | None
    startup_stdout: bytes = b""
    startup_stderr: bytes = b""


def _runtime_and_bus() -> tuple[Path, Path, int]:
    runtime_value = os.environ.get("XDG_RUNTIME_DIR")
    if runtime_value is None:
        try:
            cgroup = Path("/proc/self/cgroup").read_text(encoding="ascii")
        except OSError:
            cgroup = ""
        match = re.search(r"/user\.slice/user-([0-9]+)\.slice/user@\1\.service/", cgroup)
        runtime_value = f"/run/user/{match.group(1) if match else os.getuid()}"
    match = re.fullmatch(r"/run/user/([0-9]+)", runtime_value)
    if match is None:
        raise ProcessScopeUnavailable("systemd runtime directory is not a direct /run/user child")
    uid = int(match.group(1))
    runtime = Path(runtime_value)
    bus = runtime / "bus"
    try:
        runtime_metadata = runtime.lstat()
        bus_metadata = bus.lstat()
    except OSError as error:
        raise ProcessScopeUnavailable("systemd user bus is unavailable") from error
    if (
        not stat.S_ISDIR(runtime_metadata.st_mode)
        or runtime_metadata.st_uid != os.getuid()
        or stat.S_IMODE(runtime_metadata.st_mode) & 0o077
        or not stat.S_ISSOCK(bus_metadata.st_mode)
        or bus_metadata.st_uid != runtime_metadata.st_uid
    ):
        raise ProcessScopeUnavailable("systemd user bus ownership or permissions are not attestable")
    configured_bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS")
    expected_bus = f"unix:path={bus}"
    if configured_bus is not None and configured_bus != expected_bus:
        raise ProcessScopeUnavailable("systemd user bus address is not the checked local socket")
    return runtime, bus, uid


def _trusted_systemd_binary(path: Path) -> bool:
    try:
        metadata = path.lstat()
    except OSError:
        return False
    return (
        stat.S_ISREG(metadata.st_mode)
        and metadata.st_uid == 0
        and stat.S_IMODE(metadata.st_mode) & 0o022 == 0
    )


def prepare_process_scope(
    command: list[str],
    environment: dict[str, str],
    timeout: float,
    *,
    owner_unit: str | None = None,
) -> ProcessScope:
    if (
        not command
        or any(not isinstance(part, str) or "\0" in part for part in command)
        or not isinstance(environment, dict)
        or set(environment) - {"PATH", "HOME", "LANG", "LC_ALL", "TERM", "NO_COLOR", "TMPDIR"}
        or any(not isinstance(value, str) or "\0" in value for value in environment.values())
    ):
        raise ProcessScopeUnavailable("scoped command or environment is invalid")
    if timeout <= 0 or not _trusted_systemd_binary(SYSTEMD_RUN) or not _trusted_systemd_binary(SYSTEMCTL):
        raise ProcessScopeUnavailable("trusted systemd-run/systemctl binaries are unavailable")
    if owner_unit is not None and re.fullmatch(
        r"run-mission012-stage-[0-9]+-[a-f0-9]{16}\.scope", owner_unit
    ) is None:
        raise ProcessScopeUnavailable("parent process scope unit is invalid")
    if SCOPE_EXEC.is_symlink() or not SCOPE_EXEC.is_file():
        raise ProcessScopeUnavailable("scoped exec helper is missing or redirected")
    runtime, bus, uid = _runtime_and_bus()
    unit = f"run-mission012-stage-{os.getpid()}-{secrets.token_hex(8)}.scope"
    prefix = f"/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/"
    nonce = secrets.token_hex(16)
    manager_environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": "/nonexistent",
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "XDG_RUNTIME_DIR": str(runtime),
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path={bus}",
    }
    maximum_runtime = max(2, int(timeout + STARTUP_GRACE_SECONDS + 1))
    scoped_exec = [
        sys.executable,
        str(SCOPE_EXEC),
        nonce,
    ]
    launch_command = [
        str(SYSTEMD_RUN),
        "--user",
        "--scope",
        "--quiet",
        f"--unit={unit}",
        "--property=KillMode=control-group",
        "--property=TimeoutStopSec=1s",
        f"--property=RuntimeMaxSec={maximum_runtime}s",
    ]
    if owner_unit is not None:
        launch_command.append(f"--property=BindsTo={owner_unit}")
    launch_command.extend(["--", *scoped_exec])
    return ProcessScope(unit, nonce, manager_environment, prefix, launch_command, owner_unit)


def _show(unit: str, manager_environment: dict[str, str]) -> dict[str, str] | None:
    try:
        result = subprocess.run(
            [
                str(SYSTEMCTL), "--user", "show", unit,
                "-p", "ControlGroup", "-p", "ActiveState", "-p", "KillMode", "-p", "BindsTo",
            ],
            env=manager_environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=2,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode:
        return None
    fields: dict[str, str] = {}
    try:
        lines = result.stdout.decode("ascii", errors="strict").splitlines()
    except UnicodeDecodeError:
        return None
    for line in lines:
        key, separator, value = line.partition("=")
        if not separator or key not in {"ControlGroup", "ActiveState", "KillMode", "BindsTo"}:
            return None
        fields[key] = value
    if set(fields) != {"ControlGroup", "ActiveState", "KillMode", "BindsTo"}:
        return None
    return fields


def _proc_cgroup(pid: int) -> str | None:
    try:
        entries = Path(f"/proc/{pid}/cgroup").read_text(encoding="ascii").splitlines()
    except OSError:
        return None
    unified = [line[3:] for line in entries if line.startswith("0::/")]
    return unified[0] if len(unified) == 1 else None


def _scope_cgroup_path(scope: ProcessScope, fields: dict[str, str]) -> str | None:
    path = fields.get("ControlGroup", "")
    if (
        not path.startswith(scope.expected_prefix)
        or path[len(scope.expected_prefix) :] != scope.unit
        or fields.get("KillMode") != "control-group"
        or (scope.owner_unit is not None and fields.get("BindsTo") != scope.owner_unit)
    ):
        return None
    return path


def attest_process_scope(scope: ProcessScope, pid: int) -> str | None:
    fields = _show(scope.unit, scope.manager_environment)
    if fields is None or fields.get("ActiveState") != "active":
        return None
    cgroup_path = _scope_cgroup_path(scope, fields)
    if cgroup_path is None or _proc_cgroup(pid) != cgroup_path:
        return None
    cgroup_procs = Path("/sys/fs/cgroup") / cgroup_path.lstrip("/") / "cgroup.procs"
    try:
        members = {int(item) for item in cgroup_procs.read_text(encoding="ascii").splitlines()}
    except (OSError, ValueError):
        return None
    return cgroup_path if pid in members else None


def _abort_scope_process(process: subprocess.Popen[bytes], scope: ProcessScope) -> None:
    try:
        os.killpg(process.pid, 15)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + 0.5
    while time.monotonic() < deadline and process.poll() is None:
        time.sleep(0.02)
    if process.poll() is None:
        try:
            os.killpg(process.pid, 9)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        pass
    abort_unstarted_process_scope(scope)
    for stream in (process.stdin, process.stdout, process.stderr):
        if stream is not None:
            try:
                stream.close()
            except OSError:
                pass


def launch_scoped_process(
    scope: ProcessScope,
    command: list[str],
    environment: dict[str, str],
    *,
    cwd: Path,
    timeout: float,
    merge_stderr: bool,
) -> ScopedProcessStart:
    """Start only a scope-attested command; the exec helper waits before launch."""
    try:
        process = subprocess.Popen(
            scope.command,
            cwd=cwd,
            env=scope.manager_environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT if merge_stderr else subprocess.PIPE,
            bufsize=0,
            close_fds=True,
            start_new_session=True,
        )
    except OSError:
        return ScopedProcessStart(None, None)
    assert process.stdin is not None and process.stdout is not None
    assert merge_stderr or process.stderr is not None
    expected_ready = f"PROCESS_SCOPE_READY:{scope.nonce}".encode("ascii")
    startup_stdout = bytearray()
    startup_stderr = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, "stdout")
    if not merge_stderr:
        assert process.stderr is not None
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
    startup_deadline = time.monotonic() + min(STARTUP_GRACE_SECONDS, max(0.0, timeout))
    cgroup_path: str | None = None
    try:
        while time.monotonic() < startup_deadline:
            events = selector.select(max(0.0, startup_deadline - time.monotonic()))
            if not events:
                if process.poll() is not None:
                    break
                continue
            for key, _ in events:
                chunk = os.read(key.fileobj.fileno(), 8192)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                if key.data == "stderr":
                    startup_stderr.extend(chunk)
                    if len(startup_stderr) > 8192:
                        raise ValueError("scope startup diagnostics exceeded bound")
                    continue
                startup_stdout.extend(chunk)
                if len(startup_stdout) > 8192:
                    raise ValueError("scope startup output exceeded bound")
                if b"\n" not in startup_stdout:
                    continue
                first_line, _, remainder = startup_stdout.partition(b"\n")
                if first_line != expected_ready or remainder:
                    raise ValueError("scope ready handshake did not match")
                cgroup_path = attest_process_scope(scope, process.pid)
                if cgroup_path is None:
                    raise ValueError("systemd did not attest the waiting process")
                payload = json.dumps(
                    {"command": command, "environment": environment},
                    sort_keys=True,
                    separators=(",", ":"),
                    ensure_ascii=True,
                ).encode("ascii") + b"\n"
                if len(payload) > 256 * 1024:
                    raise ValueError("scoped command envelope exceeds its bound")
                view = memoryview(payload)
                while view:
                    written = os.write(process.stdin.fileno(), view)
                    if written <= 0:
                        raise OSError("short write to scoped command envelope")
                    view = view[written:]
                process.stdin.close()
                return ScopedProcessStart(
                    process,
                    cgroup_path,
                    b"",
                    bytes(startup_stderr),
                )
    except (OSError, ValueError, subprocess.SubprocessError):
        pass
    finally:
        selector.close()
    _abort_scope_process(process, scope)
    return ScopedProcessStart(
        None,
        None,
        bytes(startup_stdout[:8192]),
        bytes(startup_stderr[:8192]),
    )


def current_attested_process_scope() -> ProcessScope | None:
    try:
        runtime, bus, uid = _runtime_and_bus()
    except ProcessScopeUnavailable:
        return None
    cgroup_path = _proc_cgroup(os.getpid())
    prefix = f"/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/"
    if cgroup_path is None or not cgroup_path.startswith(prefix):
        return None
    unit = cgroup_path[len(prefix) :]
    if (
        "/" in unit
        or not unit.startswith("run-mission012-stage-")
        or not unit.endswith(".scope")
        or not all(char.isascii() and (char.isalnum() or char in "-_.") for char in unit)
    ):
        return None
    manager_environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": "/nonexistent",
        "XDG_RUNTIME_DIR": str(runtime),
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path={bus}",
    }
    scope = ProcessScope(unit, "", manager_environment, prefix, [])
    return scope if attest_process_scope(scope, os.getpid()) == cgroup_path else None


def current_process_scope_is_attested() -> bool:
    return current_attested_process_scope() is not None


def _active_scope_members(cgroup_path: str) -> list[int] | None:
    cgroup_procs = Path("/sys/fs/cgroup") / cgroup_path.lstrip("/") / "cgroup.procs"
    try:
        pids = [int(item) for item in cgroup_procs.read_text(encoding="ascii").splitlines()]
    except FileNotFoundError:
        return []
    except (OSError, ValueError):
        return None
    active: list[int] = []
    for pid in pids:
        try:
            fields = Path(f"/proc/{pid}/stat").read_bytes().rsplit(b")", 1)[1].split()
        except FileNotFoundError:
            continue
        except OSError:
            return None
        if not fields or fields[0] not in {b"Z", b"X"}:
            active.append(pid)
    return active


def current_attested_process_scope_snapshot() -> ProcessScopeSnapshot | None:
    """Return a read-only snapshot of this process's verified parent scope."""
    scope = current_attested_process_scope()
    cgroup_path = _proc_cgroup(os.getpid())
    if scope is None or cgroup_path is None:
        return None
    members = _active_scope_members(cgroup_path)
    if members is None or os.getpid() not in members:
        return None
    return ProcessScopeSnapshot(scope.unit, cgroup_path, tuple(sorted(members)))


def process_scope_snapshot_matches(snapshot: ProcessScopeSnapshot) -> bool:
    """Confirm the caller still occupies the exact scope and active PID set."""
    current = current_attested_process_scope_snapshot()
    return current == snapshot


def finish_process_scope(scope: ProcessScope, cgroup_path: str) -> str:
    """Stop the scope and confirm no active process remains; flag escaped sessions."""
    before_stop = _active_scope_members(cgroup_path)
    if before_stop is None:
        leaked = True
    else:
        leaked = bool(before_stop)
    try:
        stop = subprocess.run(
            [str(SYSTEMCTL), "--user", "stop", scope.unit],
            env=scope.manager_environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=STOP_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return "unconfirmed_stop_request"
    if stop.returncode:
        fields = _show(scope.unit, scope.manager_environment)
        members = _active_scope_members(cgroup_path)
        if (
            fields is not None
            and fields.get("ActiveState") == "inactive"
            and members == []
        ):
            return "descendants_terminated" if leaked else "clean"
        return f"unconfirmed_stop_request_{stop.returncode}"
    deadline = time.monotonic() + STOP_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        fields = _show(scope.unit, scope.manager_environment)
        if fields is None:
            return "unconfirmed_state_read"
        active = fields.get("ActiveState")
        members = _active_scope_members(cgroup_path)
        if members is None:
            return "unconfirmed_cgroup_read"
        if active == "inactive" and not members:
            return "descendants_terminated" if leaked else "clean"
        time.sleep(0.025)
    return "unconfirmed_timeout"


def abort_unstarted_process_scope(scope: ProcessScope) -> bool:
    """Stop a generated unit after startup attestation failed, before command payload."""
    try:
        stop = subprocess.run(
            [str(SYSTEMCTL), "--user", "stop", scope.unit],
            env=scope.manager_environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=STOP_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return False
    if stop.returncode:
        return False
    fields = _show(scope.unit, scope.manager_environment)
    if fields is None:
        return False
    cgroup_path = _scope_cgroup_path(scope, fields)
    if cgroup_path is None:
        return fields.get("ActiveState") == "inactive" and not fields.get("ControlGroup")
    members = _active_scope_members(cgroup_path)
    return fields.get("ActiveState") == "inactive" and members == []
