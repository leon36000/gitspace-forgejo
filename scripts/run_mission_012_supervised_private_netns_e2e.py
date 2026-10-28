#!/usr/bin/env python3
"""Run the no-model mission pipeline with Forgejo and its merge gate in a private netns."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import stat
import subprocess
import sys
import threading
import time
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))
import run_forgejo_private_netns_webhook_e2e as netns  # noqa: E402
from systemd_process_scope import (  # noqa: E402
    ProcessScopeUnavailable,
    finish_process_scope,
    launch_scoped_process,
    prepare_process_scope,
)

FORGEJO_SHA256 = "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7"
FORGEJO_PORT = 3132
MAX_CAPTURE_BYTES = 12 * 1024 * 1024
MAX_RUNTIME_SECONDS = 1800


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _same_namespace_inode(left: Any, right: Any) -> bool:
    return (
        type(left) is int and left > 0
        and type(right) is int and right > 0
        and left == right
    )


def _postmerge_behavior_is_verified(
    forgejo_result: Any, proof_file: Any, output: Any,
) -> bool:
    if not isinstance(forgejo_result, dict) or not isinstance(proof_file, dict) or not isinstance(output, bytes):
        return False
    proof = forgejo_result.get("postmerge_verification")
    if not isinstance(proof, dict) or proof_file != proof:
        return False
    merge_commit = forgejo_result.get("merge_commit")
    output_sha256 = proof.get("output_sha256")
    return (
        proof.get("verdict") == "pass"
        and proof.get("test_summary") == "2 passed; 0 failed"
        and type(proof.get("exit_code")) is int and proof.get("exit_code") == 0
        and proof.get("timed_out") is False
        and type(proof.get("output_bytes")) is int and proof.get("output_bytes") > 0
        and isinstance(output_sha256, str) and re.fullmatch(r"[0-9a-f]{64}", output_sha256) is not None
        and proof.get("output_bytes") == len(output)
        and output_sha256 == sha256(output)
        and output.count(b"test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;") == 1
        and proof.get("commit") == merge_commit
        and proof.get("main_head_after_merge") == merge_commit
        and merge_commit == forgejo_result.get("main_head_after_merge")
        and proof.get("candidate_commit") == forgejo_result.get("candidate_commit")
        and proof.get("base_commit") == forgejo_result.get("base_commit")
        and proof.get("requirement_sha256") == forgejo_result.get("requirement_sha256")
        and proof.get("policy_sha256") == forgejo_result.get("policy_sha256")
        and proof.get("source_sha256") == forgejo_result.get("candidate_source_sha256")
        and proof.get("acceptance_sha256") == forgejo_result.get("acceptance_sha256")
        and proof.get("merge_tree_oid") == forgejo_result.get("merge_tree_oid")
        and proof.get("merge_tree_oid") == proof.get("candidate_tree_oid")
        and proof.get("changed_paths_from_base") == ["src/lib.rs"]
    )


def _host_restart_recovery_is_verified(result: Any) -> bool:
    if not isinstance(result, dict):
        return False
    recovery = result.get("host_restart_recovery")
    if not isinstance(recovery, dict):
        return False
    old_fence = recovery.get("old_fencing_token")
    new_fence = recovery.get("new_fencing_token")
    initial_lease = recovery.get("initial_lease_id")
    recovery_lease = recovery.get("recovery_lease_id")
    first_exit = recovery.get("first_attempt_exit_code")
    return (
        recovery.get("crash_point") == "after_effect"
        and type(first_exit) is int and first_exit != 0
        and recovery.get("first_attempt_timed_out") is False
        and recovery.get("first_attempt_output_exceeded") is False
        and recovery.get("unresolved_intents_before_restart") == 1
        and recovery.get("results_before_restart") == 0
        and recovery.get("results_after_restart") == 1
        and recovery.get("resume_checkpoint_status") == "unverified"
        and type(old_fence) is int and type(new_fence) is int and new_fence > old_fence
        and isinstance(initial_lease, str) and bool(initial_lease)
        and isinstance(recovery_lease, str) and bool(recovery_lease)
        and recovery_lease != initial_lease
        and type(recovery.get("initial_receipt_count")) is int
        and recovery["initial_receipt_count"] > 0
        and type(recovery.get("recovery_receipt_count")) is int
        and recovery["recovery_receipt_count"] > 0
        and all(
            isinstance(recovery.get(key), str)
            and re.fullmatch(r"[0-9a-f]{64}", recovery[key]) is not None
            for key in (
                "candidate_source_sha256_after_effect",
                "initial_receipt_sha256",
                "recovery_receipt_sha256",
                "combined_receipt_sha256",
            )
        )
    )


def _composed_runner_arguments(
    args: argparse.Namespace, host_netns: str, mounted_git_dir: Path,
) -> list[str]:
    command = [
        "/usr/bin/python3", "/run/worktree/scripts/run_mission_012_live_composed_e2e.py",
        "--state-dir", args.state_dir,
        "--forgejo-binary", "/opt/forgejo/forgejo",
        "--evidence-dir", f"/run/worktree/evidence/{Path(args.evidence_dir).name}",
        "--execution-id", args.execution_id,
        "--allocation-id", args.allocation_id,
        "--budget-cap-usd", "0.20",
        "--agent-adapter", "local-test-adapter",
        "--sandbox-test-adapter",
        "--reuse-attested-parent-scope",
    ]
    if getattr(args, "test_host_restart", False):
        command.append("--test-host-restart")
    else:
        command.append("--same-host-gate-barrier")
    command.extend([
        "--expected-host-network-namespace", host_netns,
        "--source-git-dir", str(mounted_git_dir),
        "--source-git-work-tree", "/run/worktree",
    ])
    return command


def _safe_failure_diagnostic(output: bytes) -> str | None:
    lines = output.decode("utf-8", "replace").splitlines()
    if not lines:
        return None
    last = lines[-1].strip()
    if not last.startswith("FAIL-CLOSED: "):
        return None
    last = re.sub(
        r"(?i)(token|secret|credential|password|api[_-]?key)(\s*[=:]\s*)[^\s,;]+",
        r"\1\2[redacted]",
        last,
    )
    last = re.sub(
        r"(?i)(?<!sha256=)\b[A-Za-z0-9_-]{32,}\b",
        "[redacted-long-value]",
        last,
    )
    return last[:1024]


def _write_exclusive(path: Path, data: bytes, mode: int) -> None:
    fd = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
        mode,
    )
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def _append_local_checksum(evidence: Path, filename: str) -> None:
    target = evidence / filename
    manifest = evidence / "SHA256SUMS"
    if (
        target.is_symlink() or not target.is_file()
        or manifest.is_symlink() or not manifest.is_file()
    ):
        raise RuntimeError("supervisor boundary evidence or composed checksum manifest is missing")
    line = f"{sha256(target.read_bytes())}  {filename}\n".encode("ascii")
    fd = os.open(
        manifest,
        os.O_WRONLY | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0),
    )
    with os.fdopen(fd, "ab") as stream:
        stream.write(line)
        stream.flush()
        os.fsync(stream.fileno())


def _runner_pid(pids: list[int]) -> int | None:
    expected = b"/run/worktree/scripts/run_mission_012_live_composed_e2e.py"
    for pid in pids:
        try:
            executable = os.readlink(f"/proc/{pid}/exe")
            argv = Path(f"/proc/{pid}/cmdline").read_bytes()
        except OSError:
            continue
        if Path(executable).name.startswith("python3") and expected in argv.split(b"\0"):
            return pid
    return None


def _bwrap_command(args: argparse.Namespace, host_netns: str, host_uid: int) -> list[str]:
    cargo_registry = Path.home() / ".cargo/registry"
    rust_toolchain = Path.home() / ".rustup/toolchains/1.95.0-aarch64-unknown-linux-gnu"
    runtime = Path(f"/run/user/{host_uid}")
    bus = runtime / "bus"
    if (
        cargo_registry.is_symlink() or not cargo_registry.is_dir()
        or rust_toolchain.is_symlink() or not rust_toolchain.is_dir()
        or runtime.is_symlink() or not runtime.is_dir()
        or runtime.stat().st_uid != host_uid or stat.S_IMODE(runtime.stat().st_mode) & 0o077
        or bus.is_symlink() or not stat.S_ISSOCK(bus.lstat().st_mode)
        or bus.lstat().st_uid != host_uid
    ):
        raise RuntimeError("read-only Rust cache or private user bus is unavailable")
    common_git_dir, relative_git_dir = netns._git_metadata_paths(ROOT)
    mounted_git_dir = Path("/run/source-git") / relative_git_dir

    command = [
        "/usr/bin/bwrap",
        "--unshare-user", "--unshare-net", "--die-with-parent",
        "--ro-bind", "/", "/",
        "--size", "2147483648", "--tmpfs", "/tmp",
        "--tmpfs", "/home", "--dir", "/home/pc6",
        "--dir", "/home/pc6/.cargo",
        "--ro-bind", str(cargo_registry), "/home/pc6/.cargo/registry",
        "--dir", "/home/pc6/.rustup",
        "--dir", "/home/pc6/.rustup/toolchains",
        "--ro-bind", str(rust_toolchain), str(rust_toolchain),
        "--tmpfs", "/run", "--dir", "/run/user",
        "--perms", "0700", "--dir", str(runtime),
        "--ro-bind", str(bus), str(bus),
        "--dir", "/run/source-git",
        "--ro-bind", str(common_git_dir), "/run/source-git",
        "--dir", "/run/worktree",
        "--tmpfs", "/opt", "--dir", "/opt/forgejo",
        "--ro-bind", str(Path(args.forgejo_binary).resolve(strict=True)), "/opt/forgejo/forgejo",
        "--ro-bind", str(ROOT), "/run/worktree",
        "--bind", str(ROOT / "evidence"), "/run/worktree/evidence",
        "--tmpfs", "/run/worktree/.claude",
        "--dev", "/dev", "--proc", "/proc",
        "--chdir", "/run/worktree",
        "--clearenv",
        "--setenv", "PATH", "/home/pc6/.cargo/bin:/usr/bin:/bin",
        "--setenv", "HOME", "/home/pc6",
        "--setenv", "XDG_RUNTIME_DIR", str(runtime),
        "--setenv", "DBUS_SESSION_BUS_ADDRESS", f"unix:path={bus}",
        "--setenv", "LANG", "C.UTF-8",
        "--setenv", "LC_ALL", "C.UTF-8",
        "--setenv", "PYTHONDONTWRITEBYTECODE", "1",
        *_composed_runner_arguments(args, host_netns, mounted_git_dir),
    ]
    return command


def _validate(args: argparse.Namespace) -> tuple[Path, Path]:
    binary = Path(args.forgejo_binary)
    evidence = Path(args.evidence_dir)
    if (
        binary.is_symlink() or not binary.is_file()
        or sha256(binary.read_bytes()) != FORGEJO_SHA256
    ):
        raise RuntimeError("Forgejo binary does not match the pinned local v17 fixture")
    expected_evidence_parent = (ROOT / "evidence").resolve(strict=True)
    if (
        not evidence.is_absolute() or evidence.parent.resolve(strict=True) != expected_evidence_parent
        or evidence.exists() or evidence.is_symlink()
        or not re.fullmatch(r"mission-012-live-agent-webhook-[a-z0-9-]{1,48}", evidence.name)
    ):
        raise RuntimeError("evidence destination must be a new local mission-012 evidence directory")
    state_name = Path(args.state_dir)
    if (
        not state_name.is_absolute() or state_name.parent != Path("/tmp")
        or state_name.exists() or state_name.is_symlink()
    ):
        raise RuntimeError("state path must be a new direct /tmp child inside the disposable namespace")
    if not re.fullmatch(r"live-mcp-[0-9]{3}", args.execution_id):
        raise RuntimeError("execution ID must match live-mcp-NNN")
    if not re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", args.allocation_id):
        raise RuntimeError("allocation ID must be a bounded mission012 identifier")
    return binary.resolve(strict=True), evidence


def run(args: argparse.Namespace) -> dict[str, Any]:
    binary, evidence = _validate(args)
    host_netns = os.readlink("/proc/self/ns/net")
    if re.fullmatch(r"net:\[[1-9][0-9]*\]", host_netns) is None:
        raise RuntimeError("cannot identify the host network namespace")
    if netns._host_connect_errno(FORGEJO_PORT) != 111:
        raise RuntimeError("host loopback port 3132 must be unused before the private run")
    command = _bwrap_command(args, host_netns, os.geteuid())
    environment = {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent", "LANG": "C.UTF-8"}
    try:
        scope = prepare_process_scope(command, environment, MAX_RUNTIME_SECONDS)
    except ProcessScopeUnavailable as error:
        raise RuntimeError(f"cannot prepare host-owned outer process scope: {error}") from None
    started = launch_scoped_process(
        scope,
        command,
        environment,
        cwd=ROOT,
        timeout=MAX_RUNTIME_SECONDS,
        merge_stderr=True,
    )
    process = started.process
    outer_scope_cgroup = started.cgroup_path
    if process is None or outer_scope_cgroup is None:
        detail = (started.startup_stdout + started.startup_stderr).decode("utf-8", "replace")
        raise RuntimeError(
            "host did not attest the outer process scope before Bubblewrap start"
            + (f": {detail[:1024]}" if detail else "")
        )
    assert process.stdout is not None
    captured = bytearray()
    overflow = threading.Event()

    def drain() -> None:
        assert process.stdout is not None
        while chunk := process.stdout.read(65536):
            if len(captured) + len(chunk) > MAX_CAPTURE_BYTES:
                overflow.set()
            elif not overflow.is_set():
                captured.extend(chunk)

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()
    observation: dict[str, Any] | None = None
    deadline = time.monotonic() + MAX_RUNTIME_SECONDS
    while process.poll() is None and time.monotonic() < deadline and not overflow.is_set():
        if observation is None:
            pids = netns.process_tree(process.pid)
            forgejo_pid = next((pid for pid in pids if netns._same_binary(pid, binary)), None)
            if forgejo_pid is not None:
                private_netns = netns._network_namespace_inode(forgejo_pid)
                listener = netns.loopback_listener_inode(forgejo_pid, FORGEJO_PORT)
                supervisor_pid = _runner_pid(pids)
                supervisor_netns = (
                    netns._network_namespace_inode(supervisor_pid)
                    if supervisor_pid is not None else None
                )
                if private_netns is not None and listener is not None:
                    observation = {
                        "host_network_namespace": host_netns,
                        "supervisor_pid": supervisor_pid,
                        "supervisor_network_namespace_inode": supervisor_netns,
                        "forgejo_pid": forgejo_pid,
                        "forgejo_network_namespace_inode": private_netns,
                        "forgejo_loopback_port": FORGEJO_PORT,
                        "forgejo_listener_owned_socket_inode": listener,
                        "host_loopback_probe_errno": netns._host_connect_errno(FORGEJO_PORT),
                        "host_setns_probe": netns._join_probe(forgejo_pid),
                        "observed_while_forgejo_process_alive": True,
                    }
        time.sleep(0.05)

    timed_out = process.poll() is None and time.monotonic() >= deadline
    scope_cleanup: str | None = None
    if timed_out or overflow.is_set():
        scope_cleanup = finish_process_scope(scope, outer_scope_cgroup)
    try:
        status = process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        timed_out = True
        scope_cleanup = finish_process_scope(scope, outer_scope_cgroup)
        try:
            status = process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            status = process.wait(timeout=5)
            scope_cleanup = finish_process_scope(scope, outer_scope_cgroup)
    if scope_cleanup is None:
        scope_cleanup = finish_process_scope(scope, outer_scope_cgroup)
    reader.join(timeout=5)

    result = None
    if status == 0 and not overflow.is_set() and scope_cleanup == "clean":
        try:
            lines = bytes(captured).splitlines()
            result = json.loads(lines[-1]) if lines else None
        except (UnicodeDecodeError, json.JSONDecodeError):
            result = None
    if not evidence.exists():
        evidence.mkdir(mode=0o755)
    gate_path = evidence / "rust-merge-authority-result.json"
    launch_path = evidence / "supervisor-capability-issuance.json"
    forgejo_path = evidence / "forgejo-result.json"
    gate_result: Any = None
    launch: Any = None
    forgejo_result: Any = None
    for path, name in ((gate_path, "gate"), (launch_path, "launch"), (forgejo_path, "forgejo")):
        if path.is_file() and not path.is_symlink() and path.stat().st_size <= 1024 * 1024:
            try:
                value = json.loads(path.read_bytes())
            except (OSError, json.JSONDecodeError):
                value = None
            if name == "gate":
                gate_result = value
            elif name == "launch":
                launch = value
            else:
                forgejo_result = value
    service = observation if isinstance(observation, dict) else {}
    audit = gate_result.get("writer_admission") if isinstance(gate_result, dict) else None
    postmerge_proof_file: Any = None
    postmerge_output: bytes | None = None
    try:
        proof_path = evidence / "postmerge-verification.json"
        output_path = evidence / "postmerge-cargo-test.log"
        proof_stat = proof_path.lstat()
        output_stat = output_path.lstat()
        if (
            stat.S_ISREG(proof_stat.st_mode) and not stat.S_ISLNK(proof_stat.st_mode)
            and proof_stat.st_uid == os.geteuid() and proof_stat.st_nlink == 1
            and proof_stat.st_size <= 64 * 1024
            and stat.S_ISREG(output_stat.st_mode) and not stat.S_ISLNK(output_stat.st_mode)
            and output_stat.st_uid == os.geteuid() and output_stat.st_nlink == 1
            and output_stat.st_size <= 256 * 1024
        ):
            postmerge_proof_file = json.loads(proof_path.read_bytes())
            postmerge_output = output_path.read_bytes()
    except (OSError, json.JSONDecodeError):
        pass
    checks = {
        "pinned_forgejo_binary": sha256(binary.read_bytes()) == FORGEJO_SHA256,
        "outer_scope_attested_before_bwrap": bool(outer_scope_cgroup),
        "outer_scope_cleanup_clean": scope_cleanup == "clean",
        "outer_runner_exit_zero": status == 0,
        "outer_runner_not_timed_out": not timed_out,
        "outer_output_within_bound": not overflow.is_set(),
        "private_forgejo_observed": bool(service.get("observed_while_forgejo_process_alive")),
        "forgejo_netns_differs_from_host": (
            isinstance(service.get("forgejo_network_namespace_inode"), int)
            and service.get("forgejo_network_namespace_inode")
            != int(host_netns.split("[")[1].rstrip("]"))
        ),
        "supervisor_shares_forgejo_netns": (
            _same_namespace_inode(
                service.get("supervisor_network_namespace_inode"),
                service.get("forgejo_network_namespace_inode"),
            )
        ),
        "host_loopback_connection_refused": service.get("host_loopback_probe_errno") == 111,
        "unprivileged_namespace_join_denied": (
            isinstance(service.get("host_setns_probe"), dict)
            and service["host_setns_probe"].get("status") == "denied_operation_not_permitted"
        ),
        "gate_capability_matches_live_processes": (
            isinstance(launch, dict) and isinstance(audit, dict)
            and launch.get("capability_sent_before_gate_exec") is True
            and launch.get("private_network_differs_from_host") is True
            and audit.get("supervisor") == launch.get("supervisor")
            and audit.get("gate") == launch.get("gate")
            and audit.get("forgejo") == launch.get("forgejo")
            and audit.get("pre_post_revalidated") is True
        ),
        "supervisor_policy_limit_explicit": (
            isinstance(audit, dict)
            and audit.get("supervisor_policy_independently_verified") is False
            and isinstance(gate_result, dict)
            and gate_result.get("external_writer_exclusion")
            == "trusted_supervisor_policy_not_independently_verified"
        ),
        "postmerge_forgejo_readback": (
            isinstance(forgejo_result, dict)
            and forgejo_result.get("merged") is True
            and forgejo_result.get("pull_request_state_after_merge") == "closed"
            and forgejo_result.get("main_head_after_merge") == forgejo_result.get("merge_commit")
        ),
        "postmerge_behavior_verified": _postmerge_behavior_is_verified(
            forgejo_result, postmerge_proof_file, postmerge_output,
        ),
        "composed_pipeline_result": (
            isinstance(result, dict) and result.get("status") == "merged_in_private_sandbox"
        ),
    }
    test_host_restart = bool(getattr(args, "test_host_restart", False))
    if test_host_restart:
        checks["host_restart_recovery_verified"] = _host_restart_recovery_is_verified(result)
    report = {
        "schema_version": 1,
        "scenario": (
            "MISSION-012 local-test-adapter with host-restart recovery under supervised private Forgejo network namespace"
            if test_host_restart
            else "MISSION-012 local-test-adapter under supervised private Forgejo network namespace"
        ),
        "test_host_restart": test_host_restart,
        "same_host_gate_barrier_enabled": not test_host_restart,
        "forgejo_binary_sha256": sha256(binary.read_bytes()),
        "forgejo_version": "17.0.0+gitea-1.22.0",
        "bubblewrap_version": subprocess.run(
            ["/usr/bin/bwrap", "--version"], capture_output=True, text=True,
            timeout=5, check=True,
        ).stdout.strip(),
        "inner_exit_code": status,
        "inner_timed_out": timed_out,
        "output_exceeded": overflow.is_set(),
        "inner_output_sha256": sha256(bytes(captured)),
        "inner_output_bytes": len(captured),
        "outer_scope_unit": scope.unit,
        "outer_scope_cgroup_path": outer_scope_cgroup,
        "outer_scope_cleanup": scope_cleanup,
        "failure_diagnostic": _safe_failure_diagnostic(bytes(captured)),
        "supervised_private_service_observation": service or None,
        "gate_verdict": gate_result.get("verdict") if isinstance(gate_result, dict) else None,
        "merge_attempted": gate_result.get("merge_attempted") if isinstance(gate_result, dict) else None,
        "merge_authorized": gate_result.get("merge_authorized") if isinstance(gate_result, dict) else None,
        "llm_invoked": False,
        "model_cost_usd": 0,
        "checks": checks,
        "verdict": "passed" if all(checks.values()) else "inconclusive",
        "limits": [
            "only the local deterministic test adapter ran; no LLM or AgentOS was used",
            "a host-created transient cgroup encloses the whole Bubblewrap run; each stage and the MCP adapter must return to the exact active PID set captured before launch, and outer cleanup kills any residual members",
            "Forgejo and the host supervisor share a rootless private network namespace; the gate and Forgejo share a PID namespace",
            "the gate checks a live peer/process-bound capability, but its policy cannot independently prove that all writers were excluded",
            "the model excludes root, privileged host actors, kernel compromise and namespace escape",
            "the deterministic adapter probes the pinned API, Web and receive-pack routes only; this does not establish behavior for arbitrary routes, hostile LLM code, privileged actors or kernel escape",
            "a single successful deterministic local run is not a controlled benchmark",
            *(
                ["in test-host-restart mode, the gate runs after the adapter stage exits; this mode does not assert the adapter is alive during gate verification"]
                if test_host_restart else []
            ),
        ],
    }
    observation_path = evidence / "supervised-private-netns-observation.json"
    _write_exclusive(observation_path, json.dumps(report, sort_keys=True, indent=2).encode() + b"\n", 0o600)
    manifest = evidence / "SHA256SUMS"
    if not manifest.exists():
        entries = []
        for path in sorted(evidence.iterdir(), key=lambda item: item.name):
            if path.name == "SHA256SUMS":
                continue
            if path.is_symlink() or not path.is_file():
                raise RuntimeError("supervised run evidence contains a symlink or non-file entry")
            entries.append(f"{sha256(path.read_bytes())}  {path.name}")
        _write_exclusive(manifest, ("\n".join(entries) + "\n").encode(), 0o600)
    else:
        _append_local_checksum(evidence, observation_path.name)
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument("--execution-id", default="live-mcp-021")
    parser.add_argument("--allocation-id", default="mission012-supervised-001")
    parser.add_argument(
        "--test-host-restart",
        action="store_true",
        help="run the debug-only local adapter crash/recovery path under the host-created scope",
    )
    args = parser.parse_args()
    try:
        report = run(args)
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0 if report["verdict"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
