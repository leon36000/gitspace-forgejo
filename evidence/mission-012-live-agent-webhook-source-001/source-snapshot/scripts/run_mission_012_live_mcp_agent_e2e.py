#!/usr/bin/env python3
"""Run the admitted Forgejo fixture through the bounded MCP host and Claude.

The input state is a private output of run_mission_012_live_admission_e2e.py.
Only its non-secret admission DTO and claim journal are handed to the host.
Claude receives only the four MCP tools enforced by the host. Its API budget
and turn count are capped; no built-in shell or file tools are enabled.
"""

from __future__ import annotations

import argparse
from decimal import Decimal, InvalidOperation
import hashlib
import io
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import sys
import tarfile
import time

EXPECTED_ORIGIN = "http://127.0.0.1:3132"
EXPECTED_OWNER = "forge-admin"
EXPECTED_REPOSITORY = "mission-012-clamp"
EXPECTED_BRANCH = "main"
EXPECTED_REQUIREMENT_SHA256 = "3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431"
EXPECTED_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    "README.md",
    "src/lib.rs",
    "tests/acceptance.rs",
    "requirements/MISSION-012-CLAMP-001.json",
}
MCP_TOOLS = (
    "mcp__mission012__mission_context",
    "mcp__mission012__workspace_read_text",
    "mcp__mission012__workspace_write_text",
    "mcp__mission012__cargo_test",
)
MAX_CLAUDE_OUTPUT_BYTES = 1024 * 1024


def private_file(path: Path, data: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def _terminate_cli_group(process: subprocess.Popen[bytes]) -> None:
    """Stop Claude and its relay group, allowing bounded MCP host finalization first."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    # The MCP supervisor handles this signal by closing host stdin and waiting
    # for its separately grouped host to write its final receipt (up to 15 s).
    deadline = time.monotonic() + 18
    while time.monotonic() < deadline:
        process.poll()
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.025)
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    if process.poll() is None:
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def run_bounded_cli(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    timeout: float,
    byte_limit: int = MAX_CLAUDE_OUTPUT_BYTES,
) -> tuple[int, bytes, bytes, bool, bool]:
    """Capture stdout/stderr under one strict budget and clean up on all exits."""
    if timeout <= 0 or byte_limit <= 0:
        raise ValueError("Claude capture timeout and byte limit must be positive")
    captures = {"stdout": bytearray(), "stderr": bytearray()}
    deadline = time.monotonic() + timeout
    captured_bytes = 0
    timed_out = False
    exceeded = False
    cancelled_status: int | None = None
    cancelled_signal: int | None = None
    previous_sigterm = signal.getsignal(signal.SIGTERM)
    previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM})

    def parent_cancelled(signum: int, _frame: object) -> None:
        # Never raise asynchronously: Python can deliver this between any two
        # bytecodes, including inside finally while cleanup is in progress.
        nonlocal cancelled_signal
        cancelled_signal = signum

    signal.signal(signal.SIGTERM, parent_cancelled)
    process: subprocess.Popen[bytes] | None = None
    selector: selectors.BaseSelector | None = None
    try:
        # Keep SIGTERM pending until the process group and both pipes are owned.
        child_unblock = (
            "import os,signal,sys; "
            "signal.pthread_sigmask(signal.SIG_UNBLOCK,{signal.SIGTERM}); "
            "os.execvpe(sys.argv[1],sys.argv[1:],os.environ)"
        )
        process = subprocess.Popen(
            [sys.executable, "-c", child_unblock, *command],
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            bufsize=0,
            start_new_session=True,
        )
        assert process.stdout is not None and process.stderr is not None
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        # The caller may have entered with SIGTERM blocked. Keep the temporary
        # cancellation handler reachable until cleanup has completed.
        signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGTERM})
        while selector.get_map():
            if cancelled_signal is not None:
                timed_out = True
                cancelled_status = 128 + cancelled_signal
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                break
            for key, _ in selector.select(min(remaining, 0.25)):
                if cancelled_signal is not None:
                    timed_out = True
                    cancelled_status = 128 + cancelled_signal
                    break
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                available = byte_limit - captured_bytes
                accepted = chunk[:available]
                captures[key.data].extend(accepted)
                captured_bytes += len(accepted)
                if len(accepted) != len(chunk):
                    exceeded = True
                    break
            if exceeded:
                break
            if cancelled_signal is not None:
                timed_out = True
                cancelled_status = 128 + cancelled_signal
                break
        if not timed_out and not exceeded and process.poll() is None:
            while process.poll() is None:
                if cancelled_signal is not None:
                    timed_out = True
                    cancelled_status = 128 + cancelled_signal
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    break
                try:
                    process.wait(timeout=min(remaining, 0.25))
                except subprocess.TimeoutExpired:
                    continue
    finally:
        try:
            # Drain any same-group helpers even after normal CLI exit. MCP hosts
            # are isolated in their own group and finalize through the relay.
            if process is not None:
                _terminate_cli_group(process)
        finally:
            # Keep the non-throwing handler active through group cleanup. A
            # signal delivered anywhere in this finally block only records
            # cancellation; it cannot unwind resource or process cleanup.
            if cancelled_signal is not None:
                timed_out = True
                cancelled_status = 128 + cancelled_signal
            try:
                if selector is not None:
                    selector.close()
            finally:
                try:
                    if process is not None:
                        for stream in (process.stdout, process.stderr):
                            if stream is not None:
                                try:
                                    stream.close()
                                except OSError:
                                    pass
                finally:
                    try:
                        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM})
                    finally:
                        try:
                            if cancelled_signal is not None:
                                timed_out = True
                                cancelled_status = 128 + cancelled_signal
                            signal.signal(signal.SIGTERM, previous_sigterm)
                        finally:
                            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
    assert process is not None
    if cancelled_status is not None:
        status = cancelled_status
    elif exceeded:
        status = 125
    elif timed_out:
        status = 124
    else:
        status = process.returncode
    return status if status is not None else 124, bytes(captures["stdout"]), bytes(captures["stderr"]), timed_out, exceeded


def git(args: list[str], *, timeout: int = 30) -> bytes:
    env = {
        "PATH": "/usr/bin:/bin",
        "HOME": "/nonexistent",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_TERMINAL_PROMPT": "0",
    }
    result = subprocess.run(
        ["/usr/bin/git", *args],
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"trusted local Git operation refused (exit {result.returncode})")
    return result.stdout


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def validate_private_file(path: Path) -> None:
    metadata = path.lstat()
    if (
        path.is_symlink()
        or not path.is_file()
        or metadata.st_uid != os.geteuid()
        or metadata.st_nlink != 1
        or metadata.st_mode & 0o7777 != 0o600
    ):
        raise RuntimeError(f"host admission state is not a private regular file: {path.name}")


def materialize_snapshot(git_dir: Path, commit: str, destination: Path) -> None:
    destination.mkdir(mode=0o700)
    archive = subprocess.run(
        ["/usr/bin/git", "-C", str(git_dir), "archive", commit],
        env={
            "PATH": "/usr/bin:/bin",
            "HOME": "/nonexistent",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
        },
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=20,
        check=False,
    )
    if archive.returncode or len(archive.stdout) > 2 * 1024 * 1024:
        raise RuntimeError("could not read a bounded archive from the admitted Git commit")
    with tarfile.open(fileobj=io.BytesIO(archive.stdout), mode="r:") as tar:
        members = tar.getmembers()
        files = {member.name for member in members if member.isfile()}
        directories = {member.name for member in members if member.isdir()}
        if (
            files != EXPECTED_FILES
            or directories != {"requirements", "src", "tests"}
            or any(member.issym() or member.islnk() or not (member.isfile() or member.isdir()) for member in members)
            or any(member.size > 1024 * 1024 for member in members)
        ):
            raise RuntimeError("admitted repository tree differs from the bounded mission fixture")
        tar.extractall(destination, filter="data")


def build_agent_command(
    agent_adapter: str,
    mcp_config: Path,
    budget_cap: Decimal,
    prompt: str,
) -> list[str]:
    if agent_adapter == "local-test-adapter":
        adapter = Path(__file__).with_name("mission_012_test_agent_adapter.py")
        if adapter.is_symlink() or not adapter.is_file():
            raise RuntimeError("the explicit local MISSION-012 test adapter is unavailable")
        return [sys.executable, str(adapter), "--mcp-config", str(mcp_config)]
    if agent_adapter != "claude":
        raise RuntimeError("agent adapter must be claude or the explicit local-test-adapter")
    claude = shutil.which("claude")
    if claude is None:
        raise RuntimeError("Claude Code CLI is not installed")
    return [
        claude,
        "--print",
        "--model", "claude-opus-5-5",
        "--effort", "high",
        "--max-turns", "12",
        "--max-budget-usd", str(budget_cap),
        "--no-session-persistence",
        "--strict-mcp-config",
        "--mcp-config", str(mcp_config),
        "--tools", "",
        "--allowedTools", ",".join(MCP_TOOLS),
        "--permission-mode", "dontAsk",
        "--output-format", "stream-json",
        "--verbose",
        prompt,
    ]


def run(args: argparse.Namespace) -> dict[str, object]:
    state = Path(args.state_dir).resolve(strict=True)
    project = state / "project"
    runtime = project / ".runtime"
    admission_file = Path(args.admission_file) if args.admission_file else runtime / "host-admission.json"
    journal = Path(args.journal_file) if args.journal_file else runtime / "mission-execution.jsonl"
    token_file = runtime / "forgejo-read.token"
    try:
        budget_cap = Decimal(args.budget_cap_usd)
    except InvalidOperation:
        raise RuntimeError("budget cap must be a decimal amount no higher than USD 0.20") from None
    if not budget_cap.is_finite() or budget_cap <= 0 or budget_cap > Decimal("0.20"):
        raise RuntimeError("budget cap must be a positive decimal amount no higher than USD 0.20")
    if (
        not admission_file.is_absolute()
        or not journal.is_absolute()
        or admission_file.parent != runtime
        or journal.parent != runtime
    ):
        raise RuntimeError("admission DTO and execution journal must remain in the project .runtime directory")
    for path in (admission_file, journal, token_file):
        validate_private_file(path)
    if state.parent != Path("/tmp") or state.stat().st_uid != os.geteuid() or state.stat().st_mode & 0o777 != 0o700:
        raise RuntimeError("admission state must be an owner-only direct child of /tmp")
    dto = json.loads(admission_file.read_bytes())
    if (
        dto.get("schema_version") != 1
        or dto.get("scope") != {
            "origin": EXPECTED_ORIGIN,
            "owner": EXPECTED_OWNER,
            "repository": EXPECTED_REPOSITORY,
            "branch": EXPECTED_BRANCH,
        }
        or dto.get("issue_number") != 1
        or dto.get("requirement_sha256") != EXPECTED_REQUIREMENT_SHA256
        or type(dto.get("allocation", {}).get("execution_generation")) is not int
        or dto.get("allocation", {}).get("execution_generation", 0) < 1
        or dto.get("request", {}).get("capabilities", {}).get("write_paths") != ["src/lib.rs"]
    ):
        raise RuntimeError("admission DTO does not match the pinned MISSION-012 profile")

    base_commit = dto.get("base_commit")
    if not isinstance(base_commit, str) or len(base_commit) != 40:
        raise RuntimeError("admission DTO has no full Git base commit")
    forgejo_git = state / "repositories" / EXPECTED_OWNER / f"{EXPECTED_REPOSITORY}.git"
    if forgejo_git.is_symlink() or not forgejo_git.is_dir():
        raise RuntimeError("private Forgejo test repository is unavailable")

    if not re.fullmatch(r"live-mcp-[0-9]{3}", args.execution_id):
        raise RuntimeError("execution ID must match the private live-mcp-NNN format")
    execution_root = runtime / args.execution_id
    execution_root.mkdir(mode=0o700)
    # The MCP host invokes `git -C <trusted_repo>`, so this must be a trusted
    # working clone rather than a bare mirror. Keep it separate from the
    # candidate workspace, which is writable by the model through MCP.
    trusted_git = execution_root / "trusted-repository"
    git(["clone", "--", str(forgejo_git), str(trusted_git)], timeout=60)
    head = git(["-C", str(trusted_git), "rev-parse", f"refs/heads/{EXPECTED_BRANCH}"]).decode().strip()
    if head != base_commit:
        raise RuntimeError("trusted Forgejo Git head differs from the admitted base commit")

    workspace_root = execution_root / "workspace"
    workspace_root.mkdir(mode=0o700)
    base_dir = workspace_root / "base"
    candidate_dir = workspace_root / "candidate"
    target_dir = workspace_root / "target"
    materialize_snapshot(trusted_git, base_commit, base_dir)
    shutil.copytree(base_dir, candidate_dir)
    target_dir.mkdir(mode=0o700)

    sysroot = Path("/home/pc6/.rustup/toolchains/1.95.0-aarch64-unknown-linux-gnu").resolve(strict=True)
    host = state / "build/host-artifacts/mission-012-mcp-host"
    if host.is_symlink() or not host.is_file():
        raise RuntimeError("the isolated current MISSION-012 MCP host binary is missing")
    supervisor_path = Path(__file__).with_name("mission_012_mcp_supervisor.py")
    if supervisor_path.is_symlink() or not supervisor_path.is_file():
        raise RuntimeError("the local MCP shutdown supervisor is unavailable")
    supervisor = supervisor_path.resolve(strict=True)
    host_hash = sha256(host)
    host_config = execution_root / "host-config.json"
    receipt_file = execution_root / "host-receipts.jsonl"
    config = {
        "schema_version": 4,
        "workspace": {
            "base_dir": str(base_dir),
            "candidate_dir": str(candidate_dir),
            "target_dir": str(target_dir),
            "rust_sysroot": str(sysroot),
            "trusted_git_dir": str(trusted_git),
        },
        "receipt_file": str(receipt_file),
        "journal_file": str(journal),
        "admission_file": str(admission_file),
        "lease_id": f"mission012-{args.execution_id}",
        "holder_id": "mission012-host-001",
        "lease_ttl_ms": 300000,
    }
    private_file(host_config, json.dumps(config, sort_keys=True, separators=(",", ":")).encode() + b"\n")
    mcp_config = execution_root / "claude-mcp.json"
    server = {
        "mcpServers": {
            "mission012": {
                "command": "/usr/bin/python3",
                "args": [
                    str(supervisor),
                    "--",
                    str(host),
                    "--config",
                    str(host_config),
                ],
                "env": {},
            }
        }
    }
    private_file(mcp_config, json.dumps(server, sort_keys=True, separators=(",", ":")).encode() + b"\n")

    prompt = (
        "Tu es l’agent d’exécution de la mission MISSION-012. Utilise uniquement les outils MCP "
        "mission012 exposés; les outils intégrés et l’accès au système sont désactivés. "
        "Appelle mission_context pour consommer la demande et le contexte admis. Lis les fichiers "
        "autorisés nécessaires. Lance cargo_test avant toute modification, identifie l’assertion "
        "rouge et sa cause. Corrige ensuite uniquement src/lib.rs pour que les valeurs au-dessus "
        "de maximum retournent maximum, tout en préservant les autres cas. Relance cargo_test "
        "jusqu’à réussite. N’essaie pas de lire les credentials hôte, d’écrire les tests ou les "
        "critères, d’appeler le réseau, d’utiliser une autre capacité ou de fusionner. Termine "
        "par un résumé factuel du défaut, du changement et du résultat."
    )
    command = build_agent_command(args.agent_adapter, mcp_config, budget_cap, prompt)
    start = time.monotonic()
    agent_status, agent_stdout, agent_stderr, agent_timed_out, agent_output_exceeded = run_bounded_cli(
        command,
        cwd=project,
        env={
            "PATH": f"{Path.home() / '.local/bin'}:/usr/bin:/bin",
            "HOME": str(Path.home()),
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
        },
        timeout=600,
    )
    duration_ms = int((time.monotonic() - start) * 1000)
    agent_label = "Claude Code" if args.agent_adapter == "claude" else "local test adapter"
    private_file(execution_root / f"{args.agent_adapter}-output.stream.jsonl", agent_stdout)
    private_file(execution_root / f"{args.agent_adapter}-stderr.log", agent_stderr)
    if agent_timed_out:
        raise RuntimeError(f"bounded {agent_label} execution timed out after 600 seconds")
    if agent_output_exceeded:
        raise RuntimeError(f"bounded {agent_label} output exceeded the combined 1 MiB capture limit")
    if agent_status:
        raise RuntimeError(f"bounded {agent_label} execution exited with status {agent_status}")
    try:
        events = [json.loads(line) for line in agent_stdout.splitlines() if line]
    except json.JSONDecodeError:
        raise RuntimeError(f"{agent_label} output was not bounded JSONL") from None
    result_events = [event for event in events if isinstance(event, dict) and event.get("type") == "result"]
    response = result_events[-1] if result_events else {}
    if response.get("is_error") is True or response.get("subtype") not in (None, "success"):
        raise RuntimeError(f"{agent_label} did not report a successful run")

    changed_paths = []
    expected_after = {}
    for path in EXPECTED_FILES:
        before = (base_dir / path).read_bytes()
        after = (candidate_dir / path).read_bytes()
        if before != after:
            changed_paths.append(path)
            expected_after[path] = hashlib.sha256(after).hexdigest()
    if len(changed_paths) != 1 or set(changed_paths) != {"src/lib.rs"}:
        raise RuntimeError("candidate did not change exactly the authorized source path")
    if not receipt_file.is_file():
        raise RuntimeError("MCP host produced no durable tool receipts")
    receipt_lines = receipt_file.read_bytes().splitlines()
    receipts = [json.loads(line) for line in receipt_lines if line]
    calls = [item for item in receipts if item.get("event") == "tool_call"]
    tool_names = sorted({item["tool_id"] for item in calls if isinstance(item.get("tool_id"), str)})
    expected_tools = {"mission.context", "workspace.read_text", "workspace.write_text", "cargo.test"}
    test_calls = [item for item in calls if item.get("tool_id") == "cargo.test"]
    write_indices = [index for index, item in enumerate(calls) if item.get("tool_id") == "workspace.write_text"]
    test_indices = [index for index, item in enumerate(calls) if item.get("tool_id") == "cargo.test"]
    finalization = next(
        (item for item in reversed(receipts) if item.get("event") == "host_finalization"),
        None,
    )
    if (
        not expected_tools.issubset(set(tool_names))
        or len(test_calls) < 2
        or not write_indices
        or not test_indices
        or test_indices[0] >= write_indices[0]
        or test_indices[-1] <= write_indices[-1]
        or finalization is None
        or finalization.get("outcome") != "candidate_ready"
    ):
        raise RuntimeError("host receipts do not show context, test-fix-test, and candidate_ready")
    result_text = response.get("result")
    if not isinstance(result_text, str):
        result_text = ""
    is_model_run = args.agent_adapter == "claude"
    result = {
        "schema_version": 1,
        "status": "candidate_ready_for_independent_verification",
        "agent_adapter": args.agent_adapter,
        "model": "claude-opus-5-5" if is_model_run else "local-test-adapter",
        "effort": "high" if is_model_run else "deterministic",
        "model_invoked_during_run": is_model_run,
        "budget_cap_usd": float(budget_cap),
        "execution_generation": dto["allocation"]["execution_generation"],
        "reported_cost_usd": (
            response.get("total_cost_usd", response.get("cost_usd")) if is_model_run else 0.0
        ),
        "input_tokens": response.get("usage", {}).get("input_tokens") if is_model_run and isinstance(response.get("usage"), dict) else None,
        "output_tokens": response.get("usage", {}).get("output_tokens") if is_model_run and isinstance(response.get("usage"), dict) else None,
        "duration_ms": duration_ms,
        "forgejo_scope": dto["scope"],
        "issue_number": dto["issue_number"],
        "base_commit": base_commit,
        "admission_sha256": sha256(admission_file),
        "host_binary_sha256": host_hash,
        "receipt_sha256": sha256(receipt_file),
        "receipt_count": len(receipts),
        "tool_names_from_receipts": tool_names,
        "changed_paths": changed_paths,
        "candidate_file_sha256": expected_after,
        "host_receipt_path": str(receipt_file),
        "private_execution_state": str(execution_root),
        "agent_summary": result_text[:2000],
    }
    private_file(execution_root / "agent-run-summary.json", json.dumps(
        result, sort_keys=True, separators=(",", ":")
    ).encode() + b"\n")
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--execution-id", default="live-mcp-005")
    parser.add_argument("--admission-file")
    parser.add_argument("--journal-file")
    parser.add_argument("--budget-cap-usd", default="0.20")
    parser.add_argument("--agent-adapter", choices=("claude", "local-test-adapter"), default="claude")
    args = parser.parse_args()
    try:
        result = run(args)
    except Exception as error:
        print(f"FAIL-CLOSED: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
