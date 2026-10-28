#!/usr/bin/env python3
"""Run the native Forgejo PR-sync webhook flow in a private user/network namespace.

The outer process observes the actual Forgejo listener through /proc, probes
the host loopback while it is live, and records whether this unprivileged host
can join its network namespace. This is local test evidence, not a production
attestation or a proof against root/privileged writers.
"""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import threading
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
FORGEJO_SHA256 = "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7"
FORGEJO_PORT = 3130
MAX_CAPTURE_BYTES = 8 * 1024 * 1024
DEFAULT_TIMEOUT_SECONDS = 360
NETNS_RE = re.compile(r"^net:\[(?P<inode>[1-9][0-9]*)\]$")
SOCKET_FD_RE = re.compile(r"^socket:\[(?P<inode>[1-9][0-9]*)\]$")


def parse_network_namespace_link(value: str) -> int:
    match = NETNS_RE.fullmatch(value)
    if match is None:
        raise ValueError("network namespace link is malformed")
    return int(match.group("inode"))


def proc_tcp_listener_inodes(contents: str, port: int) -> set[int]:
    """Return inodes for IPv4 loopback LISTEN sockets on the requested port."""
    if not 1 <= port <= 65535:
        raise ValueError("TCP port is out of range")
    inodes: set[int] = set()
    for line in contents.splitlines()[1:]:
        fields = line.split()
        if len(fields) < 10:
            continue
        local_address, state, inode_text = fields[1], fields[3], fields[9]
        address_hex, separator, port_hex = local_address.partition(":")
        if not separator or len(address_hex) != 8 or state != "0A":
            continue
        try:
            address = ipaddress.IPv4Address(bytes.fromhex(address_hex)[::-1])
            local_port = int(port_hex, 16)
            inode = int(inode_text)
        except (ValueError, OverflowError):
            continue
        if address == ipaddress.IPv4Address("127.0.0.1") and local_port == port and inode > 0:
            inodes.add(inode)
    return inodes


def process_open_socket_inodes(pid: int) -> set[int]:
    inodes: set[int] = set()
    try:
        fds = Path(f"/proc/{pid}/fd").iterdir()
        for fd in fds:
            try:
                target = os.readlink(fd)
            except OSError:
                continue
            match = SOCKET_FD_RE.fullmatch(target)
            if match is not None:
                inodes.add(int(match.group("inode")))
    except OSError:
        return set()
    return inodes


def loopback_listener_inode(pid: int, port: int) -> int | None:
    """Find a listening loopback socket owned by this exact process."""
    try:
        contents = Path(f"/proc/{pid}/net/tcp").read_text(encoding="ascii")
    except OSError:
        return None
    owned = process_open_socket_inodes(pid)
    matching = proc_tcp_listener_inodes(contents, port) & owned
    return min(matching) if matching else None


def _children(pid: int) -> list[int]:
    children: list[int] = []
    try:
        task_ids = os.listdir(f"/proc/{pid}/task")
    except OSError:
        return children
    for task_id in task_ids:
        try:
            raw = Path(f"/proc/{pid}/task/{task_id}/children").read_text(encoding="ascii")
        except OSError:
            continue
        for value in raw.split():
            try:
                children.append(int(value))
            except ValueError:
                continue
    return children


def process_tree(root_pid: int) -> list[int]:
    pending = [root_pid]
    seen: set[int] = set()
    ordered: list[int] = []
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        ordered.append(pid)
        pending.extend(_children(pid))
    return ordered


def is_emitted_runner(executable: str, argv: list[str]) -> bool:
    if not Path(executable).name.startswith("python3") or not argv:
        return False
    return (
        len(argv) >= 2
        and Path(argv[0]).name.startswith("python3")
        and argv[1] == "/run/workspace/scripts/run_forgejo_live_webhook_reconcile_emitted.py"
    )


def _network_namespace_inode(pid: int) -> int | None:
    try:
        return parse_network_namespace_link(os.readlink(f"/proc/{pid}/ns/net"))
    except (OSError, ValueError):
        return None


def _same_binary(pid: int, binary: Path) -> bool:
    try:
        actual = os.stat(f"/proc/{pid}/exe")
        expected = binary.stat()
        return (actual.st_dev, actual.st_ino) == (expected.st_dev, expected.st_ino)
    except OSError:
        return False


def _runner_pid(pids: list[int]) -> int | None:
    for pid in pids:
        try:
            executable = os.readlink(f"/proc/{pid}/exe")
            raw_cmdline = Path(f"/proc/{pid}/cmdline").read_bytes()
        except OSError:
            continue
        argv = [os.fsdecode(argument) for argument in raw_cmdline.split(b"\0") if argument]
        if is_emitted_runner(executable, argv):
            return pid
    return None


def _host_connect_errno(port: int) -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.settimeout(0.25)
        return probe.connect_ex(("127.0.0.1", port))


def _git_metadata_paths(repo: Path) -> tuple[Path, Path]:
    dot_git = repo / ".git"
    if dot_git.is_dir():
        git_dir = dot_git.resolve(strict=True)
    else:
        lines = dot_git.read_text(encoding="utf-8").splitlines()
        if len(lines) != 1 or not lines[0].startswith("gitdir: "):
            raise RuntimeError("source worktree .git pointer is malformed")
        raw_git_dir = Path(lines[0][len("gitdir: ") :])
        git_dir = (raw_git_dir if raw_git_dir.is_absolute() else repo / raw_git_dir).resolve(
            strict=True
        )
    common_file = git_dir / "commondir"
    if common_file.is_file():
        raw_common = Path(common_file.read_text(encoding="utf-8").strip())
        common_dir = (raw_common if raw_common.is_absolute() else git_dir / raw_common).resolve(
            strict=True
        )
    else:
        common_dir = git_dir
    return common_dir, git_dir.relative_to(common_dir)


def _join_probe(pid: int) -> dict[str, Any]:
    nsenter = shutil.which("nsenter")
    if nsenter is None:
        return {"status": "unavailable"}
    result = subprocess.run(
        [nsenter, f"--net=/proc/{pid}/ns/net", "/usr/bin/true"],
        env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        timeout=3,
        check=False,
    )
    if result.returncode == 0:
        return {"status": "joined"}
    error = result.stderr.decode("utf-8", "replace").lower()
    if "operation not permitted" in error:
        return {"status": "denied_operation_not_permitted"}
    return {"status": "inconclusive", "exit_code": result.returncode}


def _write_exclusive_json(path: Path, value: dict[str, Any]) -> None:
    data = json.dumps(value, indent=2, sort_keys=True).encode("utf-8") + b"\n"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def hash_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def create_checksum_manifest(evidence_dir: Path, filename: str) -> None:
    if Path(filename).name != filename or filename in {"", ".", "..", "SHA256SUMS"}:
        raise ValueError("checksum filename must be a single ordinary file name")
    target = evidence_dir / filename
    manifest = evidence_dir / "SHA256SUMS"
    if target.is_symlink() or not target.is_file() or manifest.exists() or manifest.is_symlink():
        raise RuntimeError("cannot create checksum manifest for a missing artifact or existing manifest")
    entry = f"{hash_file(target)}  {filename}\n".encode("ascii")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(manifest, flags, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(entry)
        stream.flush()
        os.fsync(stream.fileno())


def read_inner_result(evidence_dir: Path) -> dict[str, Any] | None:
    path = evidence_dir / "result.json"
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_CAPTURE_BYTES:
        return None
    try:
        result = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return None
    if not isinstance(result, dict) or not isinstance(result.get("verdict"), str):
        return None
    return result


def private_run_checks(
    observation: dict[str, Any], inner_result: dict[str, Any] | None
) -> dict[str, bool]:
    service = observation.get("private_service_observation")
    service = service if isinstance(service, dict) else {}
    setns_probe = service.get("host_setns_probe")
    setns_probe = setns_probe if isinstance(setns_probe, dict) else {}
    host_netns = service.get("host_network_namespace_inode")
    forgejo_netns = service.get("forgejo_network_namespace_inode")
    runner_netns = service.get("runner_network_namespace_inode")
    return {
        "inner_result_passed": inner_result is not None and inner_result.get("verdict") == "passed",
        "runner_exited_successfully": observation.get("inner_runner_exit_code") == 0,
        "runner_not_timed_out": observation.get("inner_runner_timed_out") is False,
        "capture_within_limit": observation.get("captured_output_exceeded") is False,
        "no_inner_failure_stage": observation.get("inner_failure_stage") is None,
        "forgejo_network_namespace_differs_from_host": (
            isinstance(host_netns, int) and isinstance(forgejo_netns, int) and host_netns != forgejo_netns
        ),
        "runner_shares_private_service_namespace": (
            isinstance(runner_netns, int) and isinstance(forgejo_netns, int) and runner_netns == forgejo_netns
        ),
        "host_loopback_connection_refused": service.get("host_loopback_probe_errno") == 111,
        "unprivileged_namespace_join_denied": (
            setns_probe.get("status") == "denied_operation_not_permitted"
        ),
        "service_observed_alive": service.get("observed_while_forgejo_process_alive") is True,
    }


def reassess_saved_run(evidence_dir: Path) -> dict[str, Any]:
    observation_path = evidence_dir / "private-netns-observation.json"
    if (
        observation_path.is_symlink()
        or not observation_path.is_file()
        or observation_path.stat().st_size > MAX_CAPTURE_BYTES
    ):
        raise RuntimeError("saved private namespace observation is missing or invalid")
    observation = json.loads(observation_path.read_text(encoding="utf-8"))
    if not isinstance(observation, dict):
        raise RuntimeError("saved private namespace observation is not an object")
    inner_result = read_inner_result(evidence_dir)
    service = observation.get("private_service_observation")
    service = service if isinstance(service, dict) else {}
    setns_probe = service.get("host_setns_probe")
    setns_probe = setns_probe if isinstance(setns_probe, dict) else {}
    host_netns = service.get("host_network_namespace_inode")
    forgejo_netns = service.get("forgejo_network_namespace_inode")
    runner_netns = service.get("runner_network_namespace_inode")
    checks = private_run_checks(observation, inner_result)
    passed = all(checks.values())
    result_path = evidence_dir / "result.json"
    return {
        "schema_version": 1,
        "verdict": "passed" if passed else "inconclusive",
        "prior_outer_run_status": observation.get("run_status"),
        "inner_verdict": inner_result.get("verdict") if inner_result is not None else None,
        "forgejo_version": (
            inner_result.get("forgejo", {}).get("version") if inner_result is not None else None
        ),
        "checks": checks,
        "network_namespace_inodes": {
            "host": host_netns,
            "runner": runner_netns,
            "forgejo": forgejo_netns,
        },
        "host_loopback_probe_errno": service.get("host_loopback_probe_errno"),
        "host_setns_probe": setns_probe or None,
        "source_sha256": {
            "private-netns-observation.json": hash_file(observation_path),
            "result.json": hash_file(result_path) if result_path.is_file() and not result_path.is_symlink() else None,
        },
        "limits": [
            "local disposable Forgejo and test runner only",
            "does not establish resistance to root, privileged writers, or kernel compromise",
            "does not exercise merge authority or Forgejo server-side compare-and-swap",
            "does not involve an LLM or production instance",
        ],
    }


def _append_local_checksum(evidence_dir: Path, filename: str) -> None:
    target = evidence_dir / filename
    manifest = evidence_dir / "SHA256SUMS"
    if target.is_symlink() or not target.is_file() or manifest.is_symlink() or not manifest.is_file():
        raise RuntimeError("private network evidence or run manifest is missing")
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    entry = f"{digest}  {filename}\n".encode("ascii")
    fd = os.open(manifest, os.O_WRONLY | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "ab") as stream:
        stream.write(entry)
        stream.flush()
        os.fsync(stream.fileno())


def _workspace_mounts(repo: Path, forgejo_binary: Path) -> list[str]:
    cargo_home = Path.home() / ".cargo"
    rustup_home = Path.home() / ".rustup"
    for required in (cargo_home / "bin", cargo_home / "registry", rustup_home):
        if not required.is_dir():
            raise RuntimeError(f"offline Rust toolchain cache is unavailable: {required.name}")
    common_git_dir, relative_git_dir = _git_metadata_paths(repo)
    mounted_git_dir = "/run/source-git"
    if relative_git_dir.parts:
        mounted_git_dir += "/" + relative_git_dir.as_posix()
    args = [
        "--unshare-user",
        "--unshare-net",
        "--die-with-parent",
        "--ro-bind",
        "/",
        "/",
        "--bind",
        "/tmp",
        "/tmp",
        "--tmpfs",
        "/opt",
        "--dir",
        "/opt/cargo",
        "--dir",
        "/opt/cargo/bin",
        "--ro-bind",
        str(cargo_home / "bin"),
        "/opt/cargo/bin",
        "--dir",
        "/opt/cargo/registry",
        "--ro-bind",
        str(cargo_home / "registry"),
        "/opt/cargo/registry",
        "--dir",
        "/opt/rustup",
        "--ro-bind",
        str(rustup_home),
        "/opt/rustup",
        "--dir",
        "/opt/forgejo",
        "--ro-bind",
        str(forgejo_binary),
        "/opt/forgejo/forgejo",
        "--tmpfs",
        "/home",
        "--tmpfs",
        "/run",
        "--dir",
        "/run/workspace",
        "--dir",
        "/run/source-git",
        "--ro-bind",
        str(common_git_dir),
        "/run/source-git",
        "--ro-bind",
        str(repo),
        "/run/workspace",
        "--bind",
        str(repo / "evidence"),
        "/run/workspace/evidence",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--chdir",
        "/run/workspace",
        "--setenv",
        "PATH",
        "/opt/cargo/bin:/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/nonexistent",
        "--setenv",
        "CARGO_HOME",
        "/opt/cargo",
        "--setenv",
        "RUSTUP_HOME",
        "/opt/rustup",
        "--setenv",
        "CARGO_NET_OFFLINE",
        "true",
        "--setenv",
        "GIT_CONFIG_NOSYSTEM",
        "1",
        "--setenv",
        "GIT_CONFIG_GLOBAL",
        "/dev/null",
        "--setenv",
        "GIT_DIR",
        mounted_git_dir,
        "--setenv",
        "GIT_WORK_TREE",
        "/run/workspace",
        "--setenv",
        "GIT_OPTIONAL_LOCKS",
        "0",
    ]
    claude_dir = repo / ".claude"
    if claude_dir.is_dir():
        args.extend(["--tmpfs", "/run/workspace/.claude"])
    return args


def _validate_args(args: argparse.Namespace) -> tuple[Path, Path, Path]:
    binary = Path(args.forgejo_binary)
    state = Path(args.state_dir)
    evidence = Path(args.evidence_dir)
    if binary.is_symlink() or not binary.is_file():
        raise RuntimeError("Forgejo binary must be a local regular file")
    if hashlib.sha256(binary.read_bytes()).hexdigest() != FORGEJO_SHA256:
        raise RuntimeError("Forgejo binary SHA-256 differs from the identified v17 fixture")
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists() or state.is_symlink():
        raise RuntimeError("state path must be a new direct child of /tmp")
    expected_parent = (ROOT / "evidence").resolve(strict=True)
    if (
        not evidence.is_absolute()
        or evidence.parent.resolve(strict=True) != expected_parent
        or evidence.exists()
        or evidence.is_symlink()
    ):
        raise RuntimeError("evidence path must be a new direct child of this repository's evidence/")
    return binary.resolve(strict=True), state, evidence


def run(args: argparse.Namespace) -> dict[str, Any]:
    forgejo_binary, state_dir, evidence_dir = _validate_args(args)
    if _host_connect_errno(FORGEJO_PORT) != 111:
        raise RuntimeError("host loopback port 3130 must be unused before the private run")
    host_netns = _network_namespace_inode(os.getpid())
    if host_netns is None:
        raise RuntimeError("cannot identify the host runner network namespace")
    bwrap = "/usr/bin/bwrap"
    version = subprocess.run(
        [bwrap, "--version"], capture_output=True, text=True, timeout=5, check=True
    ).stdout.strip()
    command = [
        bwrap,
        *_workspace_mounts(ROOT, forgejo_binary),
        "--",
        "/usr/bin/python3",
        "/run/workspace/scripts/run_forgejo_live_webhook_reconcile_emitted.py",
        "--forgejo-binary",
        "/opt/forgejo/forgejo",
        "--repo-root",
        "/run/workspace",
        "--state-dir",
        str(state_dir),
        "--evidence-dir",
        "/run/workspace/evidence/" + evidence_dir.name,
        "--synchronize-pull-request",
        "--batch-pull-request-events",
    ]
    process = subprocess.Popen(
        command,
        env={"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8"},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        close_fds=True,
    )
    captured = bytearray()
    overflow = threading.Event()

    def drain() -> None:
        assert process.stdout is not None
        while chunk := process.stdout.read(65536):
            if len(captured) + len(chunk) > MAX_CAPTURE_BYTES:
                overflow.set()
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
            elif not overflow.is_set():
                captured.extend(chunk)

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()
    observation: dict[str, Any] | None = None
    deadline = time.monotonic() + args.timeout_seconds
    while process.poll() is None and time.monotonic() < deadline:
        if observation is None:
            pids = process_tree(process.pid)
            forgejo_pid = next((pid for pid in pids if _same_binary(pid, forgejo_binary)), None)
            if forgejo_pid is not None:
                server_netns = _network_namespace_inode(forgejo_pid)
                owned_listener = loopback_listener_inode(forgejo_pid, FORGEJO_PORT)
                runner_pid = _runner_pid(pids)
                runner_netns = _network_namespace_inode(runner_pid) if runner_pid is not None else None
                if server_netns is not None and owned_listener is not None:
                    host_probe_errno = _host_connect_errno(FORGEJO_PORT)
                    join_probe = _join_probe(forgejo_pid)
                    observation = {
                        "host_network_namespace_inode": host_netns,
                        "runner_pid": runner_pid,
                        "runner_network_namespace_inode": runner_netns,
                        "forgejo_pid": forgejo_pid,
                        "forgejo_network_namespace_inode": server_netns,
                        "forgejo_loopback_port": FORGEJO_PORT,
                        "forgejo_listener_owned_socket_inode": owned_listener,
                        "host_loopback_probe_errno": host_probe_errno,
                        "host_setns_probe": join_probe,
                        "observed_while_forgejo_process_alive": True,
                    }
        time.sleep(0.05)

    timed_out = process.poll() is None and time.monotonic() >= deadline
    if timed_out or overflow.is_set():
        try:
            process.kill()
        except ProcessLookupError:
            pass
    try:
        status = process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        status = process.wait()
    reader.join(timeout=5)

    inner_result = read_inner_result(evidence_dir)
    failure_path = evidence_dir / "failure.json"
    failure_stage = None
    if failure_path.is_file() and not failure_path.is_symlink():
        try:
            failure = json.loads(failure_path.read_text(encoding="utf-8"))
            failure_stage = failure.get("stage") if isinstance(failure, dict) else None
        except (OSError, json.JSONDecodeError):
            failure_stage = "failure_evidence_unreadable"

    report = {
        "schema_version": 1,
        "scenario": "Forgejo native pull_request synchronized webhook plus batch reconciliation",
        "bwrap_version": version,
        "host_network_namespace_inode": host_netns,
        "private_service_observation": observation,
        "inner_runner_exit_code": status,
        "inner_runner_timed_out": timed_out,
        "captured_output_exceeded": overflow.is_set(),
        "inner_verdict": inner_result.get("verdict") if inner_result is not None else None,
        "inner_failure_stage": failure_stage,
        "run_status": "failed",
        "limits": [
            "local disposable Forgejo v17 fixture and webhook receiver only",
            "the namespace observation covers this unprivileged host process and does not exclude root or privileged writers",
            "no merge authority or CAS behavior is exercised by this webhook scenario",
            "no LLM, AgentOS, external service, or model cost is involved",
        ],
    }
    if observation is not None:
        report["run_status"] = (
            "passed" if all(private_run_checks(report, inner_result).values()) else "failed"
        )
    if evidence_dir.exists():
        if not evidence_dir.is_dir() or evidence_dir.is_symlink():
            raise RuntimeError("inner runner evidence destination is not a regular directory")
    else:
        evidence_dir.mkdir(mode=0o700)
    _write_exclusive_json(evidence_dir / "private-netns-observation.json", report)
    manifest = evidence_dir / "SHA256SUMS"
    if manifest.is_file() and not manifest.is_symlink():
        _append_local_checksum(evidence_dir, "private-netns-observation.json")
    else:
        create_checksum_manifest(evidence_dir, "private-netns-observation.json")
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument("--timeout-seconds", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    args = parser.parse_args()
    if not 30 <= args.timeout_seconds <= 600:
        parser.error("--timeout-seconds must be between 30 and 600")
    try:
        report = run(args)
    except Exception as error:
        print(f"FAIL-CLOSED: {type(error).__name__}: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0 if report["run_status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
