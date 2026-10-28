#!/usr/bin/env python3
"""Run signed webhook admission, Claude MCP work, and isolated Forgejo PR merge."""

from __future__ import annotations

import argparse
import base64
from decimal import Decimal, InvalidOperation
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import subprocess
import sys
import time
from typing import Callable


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))
import run_forgejo_live_webhook_mission_admission as live_admission  # noqa: E402
import run_mission_012_live_agent_pr_e2e as pr_driver  # noqa: E402
from process_group_lifecycle import (  # noqa: E402
    child_exit_is_waitable,
    finish_exited_process_group,
)
from systemd_process_scope import (  # noqa: E402
    ProcessScopeSnapshot,
    ProcessScopeUnavailable,
    current_attested_process_scope_snapshot,
    finish_process_scope,
    launch_scoped_process,
    prepare_process_scope,
    process_scope_snapshot_matches,
)
from mission_webhook_handoff import (  # noqa: E402
    validate_stage_admission_handoff,
)
from composition_source_snapshot import (  # noqa: E402
    SourceSnapshotError,
    capture_sources,
    copy_snapshot,
    discover_source_paths,
    verify_sources_unchanged,
)

FORGEJO_SHA256 = "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7"
MAX_BUDGET_USD = Decimal("0.20")
REAL_PROVIDER_EXECUTION_REFUSAL = (
    "Claude Code execution is disabled: provider egress and filesystem isolation are not attested"
)
# A PR receipt carries three independently bounded (2 MiB) verifier outputs
# as base64, plus its other evidence fields.
MAX_STAGE_OUTPUT_BYTES = 10 * 1024 * 1024
MAX_VERIFIER_OUTPUT_BYTES = 2 * 1024 * 1024
EXPECTED_STAGES = ("admission", "agent", "pull-request")
EXPECTED_PR_VERIFIER_CPU_STEPS = ("base-red", "candidate-green", "premerge-green")
PR_VERIFIER_CPU_LEDGER_SOURCE = (
    "mission execution journal; reservations and resolutions produced by the admission-provenance MCP host binary"
)
PR_VERIFIER_CPU_MEASUREMENT = (
    "conservative upper bound from wall time multiplied by enforced CPUQuota, plus host margin, capped at reservation"
)
EXPECTED_GREEN_ACCEPTANCE_TESTS = (
    "values_inside_the_range_are_unchanged",
    "values_outside_the_range_clamp_to_the_nearest_boundary",
)


def parse_budget(value: str) -> Decimal:
    try:
        budget = Decimal(value)
    except InvalidOperation:
        raise RuntimeError("model budget must be a positive decimal no higher than USD 0.20") from None
    if not budget.is_finite() or budget <= 0 or budget > MAX_BUDGET_USD:
        raise RuntimeError("model budget must be a positive decimal no higher than USD 0.20")
    return budget


def read_source_head(
    repo_root: Path,
    *,
    git_dir: str | None = None,
    git_work_tree: str | None = None,
) -> str:
    if (git_dir is None) != (git_work_tree is None):
        raise RuntimeError("source Git directory and worktree must be provided together")
    source_git_env = {
        "PATH": "/usr/bin:/bin",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_OPTIONAL_LOCKS": "0",
    }
    if git_dir is not None and git_work_tree is not None:
        source_git_env["GIT_DIR"] = git_dir
        source_git_env["GIT_WORK_TREE"] = git_work_tree
    return subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=repo_root, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5, check=True,
        env=source_git_env,
    ).stdout.decode().strip()


def _safe_stage_failure_diagnostic(output: bytes) -> str | None:
    for raw_line in reversed(output.decode("utf-8", "replace").splitlines()):
        line = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", raw_line).strip()
        if line.startswith("FAIL-CLOSED: "):
            pass
        elif re.match(r"^[A-Za-z_][A-Za-z0-9_.]*(?:Error|Exception):", line):
            pass
        else:
            continue
        line = re.sub(
            r"(?i)(token|secret|credential|password|api[_-]?key)(\s*[=:]\s*)[^\s,;]+",
            r"\1\2[redacted]",
            line,
        )
        line = re.sub(
            r"(?i)(?<!sha256=)\b[A-Za-z0-9_-]{32,}\b",
            "[redacted-long-value]",
            line,
        )
        return line[:1024]
    return None


def build_stage_commands(
    repo_root: Path,
    state_dir: Path,
    forgejo_binary: Path,
    evidence_dir: Path,
    execution_id: str,
    allocation_id: str,
    budget_cap_usd: str,
    *,
    agent_adapter: str = "claude",
    python_executable: str = sys.executable,
    inject_concurrent_base_advance: bool = False,
    inject_concurrent_code_conflict: bool = False,
    inject_candidate_mutation_after_verification: bool = False,
    test_host_restart: bool = False,
    sandbox_test_adapter: bool = False,
    same_host_gate_barrier: bool = False,
    reuse_attested_parent_scope: bool = False,
    expected_host_network_namespace: str | None = None,
    source_git_dir: str | None = None,
    source_git_work_tree: str | None = None,
) -> list[tuple[str, list[str], int]]:
    if agent_adapter == "claude":
        raise RuntimeError(REAL_PROVIDER_EXECUTION_REFUSAL)
    parse_budget(budget_cap_usd)
    if agent_adapter not in ("claude", "local-test-adapter"):
        raise RuntimeError("agent adapter must be claude or the explicit local-test-adapter")
    if sum((
        inject_concurrent_base_advance,
        inject_concurrent_code_conflict,
        inject_candidate_mutation_after_verification,
    )) > 1:
        raise RuntimeError("candidate and concurrent refusal fixture modes cannot be combined")
    if test_host_restart and agent_adapter != "local-test-adapter":
        raise RuntimeError("test host restart requires local-test-adapter")
    if sandbox_test_adapter and agent_adapter != "local-test-adapter":
        raise RuntimeError("sandbox mode is only available for local-test-adapter")
    if same_host_gate_barrier and (
        agent_adapter != "local-test-adapter" or not sandbox_test_adapter
    ):
        raise RuntimeError("same-host gate barrier requires the sandboxed local-test-adapter")
    if reuse_attested_parent_scope and (
        agent_adapter != "local-test-adapter" or not sandbox_test_adapter
    ):
        raise RuntimeError("parent-scope reuse requires the sandboxed local-test-adapter")
    if same_host_gate_barrier and (
        test_host_restart
        or inject_concurrent_base_advance
        or inject_concurrent_code_conflict
        or inject_candidate_mutation_after_verification
    ):
        raise RuntimeError("same-host gate barrier is limited to the complete success-path fixture")
    if test_host_restart and (
        inject_concurrent_base_advance
        or inject_concurrent_code_conflict
        or inject_candidate_mutation_after_verification
    ):
        raise RuntimeError("test host restart and refusal modes cannot be combined")
    if not state_dir.is_absolute() or state_dir.parent != Path("/tmp"):
        raise RuntimeError("state directory must be a direct child of /tmp")
    if not evidence_dir.is_absolute() or evidence_dir.parent != repo_root / "evidence":
        raise RuntimeError("evidence directory must be a direct child of this repository's evidence folder")
    if not re.fullmatch(r"live-mcp-[0-9]{3}", execution_id):
        raise RuntimeError("execution ID must match the private live-mcp-NNN format")
    if not re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", allocation_id):
        raise RuntimeError("allocation ID must be a bounded host-owned mission012 identifier")
    if expected_host_network_namespace is not None and re.fullmatch(
        r"net:\[[1-9][0-9]*\]", expected_host_network_namespace
    ) is None:
        raise RuntimeError("expected host network namespace must be a kernel netns link")
    if (source_git_dir is None) != (source_git_work_tree is None):
        raise RuntimeError("source Git directory and worktree must be provided together")
    if not re.fullmatch(r"mission-012-live-agent-webhook-[a-z0-9-]{1,48}", evidence_dir.name):
        raise RuntimeError("evidence directory must use the mission-012-live-agent-webhook-<run> form")

    admission_script = repo_root / "scripts/run_forgejo_live_webhook_mission_admission.py"
    agent_script = repo_root / "scripts/run_mission_012_live_mcp_agent_e2e.py"
    pull_request_script = repo_root / "scripts/run_mission_012_live_agent_pr_e2e.py"
    admission_evidence_dir = repo_root / "evidence" / f"{evidence_dir.name}-admission"
    if admission_evidence_dir.exists() or admission_evidence_dir.is_symlink():
        raise RuntimeError("admission evidence directory already exists")
    state = str(state_dir)
    forgejo = str(forgejo_binary)
    pull_request_command = [
        python_executable, str(pull_request_script),
        "--state-dir", state,
        "--forgejo-binary", forgejo,
        "--evidence-dir", str(evidence_dir),
        "--admission-evidence-dir", str(admission_evidence_dir),
        "--execution-id", execution_id,
        "--budget-cap-usd", budget_cap_usd,
        "--observe-pr-webhooks",
    ]
    if expected_host_network_namespace is not None:
        pull_request_command.extend([
            "--expected-host-network-namespace", expected_host_network_namespace,
        ])
    if inject_concurrent_base_advance:
        pull_request_command.append("--inject-concurrent-base-advance")
    elif inject_concurrent_code_conflict:
        pull_request_command.append("--inject-concurrent-code-conflict")
    elif inject_candidate_mutation_after_verification:
        pull_request_command.append("--inject-candidate-mutation-after-verification")
    admission_command = [
        python_executable, str(admission_script),
        "--forgejo-binary", forgejo,
        "--repo-root", str(repo_root),
        "--state-dir", state,
        "--evidence-dir", str(admission_evidence_dir),
        "--allocation-id", allocation_id,
        "--build-mcp-host",
    ]
    if source_git_dir is not None and source_git_work_tree is not None:
        admission_command.extend([
            "--source-git-dir", source_git_dir,
            "--source-git-work-tree", source_git_work_tree,
        ])
    agent_command = [
        python_executable, str(agent_script),
        "--state-dir", state,
        "--execution-id", execution_id,
        "--budget-cap-usd", budget_cap_usd,
        "--agent-adapter", agent_adapter,
    ]
    if test_host_restart:
        admission_command.append("--build-test-crash-injection-host")
        agent_command.extend([
            "--test-host-restart",
            "--admission-evidence-dir", str(admission_evidence_dir),
        ])
    if sandbox_test_adapter:
        agent_command.append("--sandbox-test-adapter")
    if same_host_gate_barrier:
        runtime_dir = state_dir / "project" / ".runtime"
        agent_command.extend([
            "--gate-barrier-ready-file", str(runtime_dir / "mcp-gate-ready.json"),
            "--gate-barrier-release-file", str(runtime_dir / "mcp-gate-release.json"),
        ])
    if reuse_attested_parent_scope:
        agent_command.append("--reuse-attested-parent-scope")
    return [
        (
            "admission",
            admission_command,
            480,
        ),
        (
            "agent",
            agent_command,
            900,
        ),
        (
            "pull-request",
            pull_request_command,
            360,
        ),
    ]


def _is_hex(value: object, length: int) -> bool:
    return isinstance(value, str) and re.fullmatch(rf"[0-9a-f]{{{length}}}", value) is not None


def validate_host_restart_evidence(
    agent: dict[str, object],
    expected_host_sha256: str,
) -> dict[str, object]:
    evidence = agent.get("restart_recovery")
    if not isinstance(evidence, dict):
        raise RuntimeError("agent result omitted the host restart recovery evidence")
    old_fence = evidence.get("old_fencing_token")
    new_fence = evidence.get("new_fencing_token")
    first_exit = evidence.get("first_attempt_exit_code")
    expiry_wait = evidence.get("lease_expiry_wait_ms")
    runtime_timeout = evidence.get("initial_runtime_timeout_ms")
    remaining_runtime = evidence.get("remaining_runtime_ms_at_recovery")
    recovery_ttl = evidence.get("recovery_lease_ttl_ms")
    initial_lease_id = evidence.get("initial_lease_id")
    recovery_lease_id = evidence.get("recovery_lease_id")
    initial_receipts = evidence.get("initial_receipt_count")
    recovery_receipts = evidence.get("recovery_receipt_count")
    if (
        evidence.get("crash_point") != "after_effect"
        or not isinstance(first_exit, int)
        or isinstance(first_exit, bool)
        or first_exit == 0
        or evidence.get("first_attempt_timed_out") is not False
        or evidence.get("first_attempt_output_exceeded") is not False
        or evidence.get("unresolved_intents_before_restart") != 1
        or evidence.get("results_before_restart") != 0
        or not _is_hex(evidence.get("candidate_source_sha256_after_effect"), 64)
        or not isinstance(evidence.get("operation_id"), str)
        or not evidence["operation_id"]
        or not isinstance(old_fence, int)
        or isinstance(old_fence, bool)
        or not isinstance(new_fence, int)
        or isinstance(new_fence, bool)
        or new_fence <= old_fence
        or not isinstance(initial_lease_id, str)
        or not re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", initial_lease_id)
        or not isinstance(recovery_lease_id, str)
        or not re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", recovery_lease_id)
        or recovery_lease_id == initial_lease_id
        or not isinstance(expiry_wait, int)
        or isinstance(expiry_wait, bool)
        or expiry_wait < 10_000
        or not isinstance(runtime_timeout, int)
        or isinstance(runtime_timeout, bool)
        or runtime_timeout <= 0
        or not isinstance(remaining_runtime, int)
        or isinstance(remaining_runtime, bool)
        or remaining_runtime <= 0
        or not isinstance(recovery_ttl, int)
        or isinstance(recovery_ttl, bool)
        or recovery_ttl < 10_000
        or recovery_ttl > 120_000
        or recovery_ttl >= remaining_runtime
        or not isinstance(initial_receipts, int)
        or isinstance(initial_receipts, bool)
        or initial_receipts <= 0
        or not isinstance(recovery_receipts, int)
        or isinstance(recovery_receipts, bool)
        or recovery_receipts <= 0
        or evidence.get("results_after_restart") != 1
        or evidence.get("resume_checkpoint_status") != "unverified"
        or not all(
            _is_hex(evidence.get(key), 64)
            for key in (
                "initial_receipt_sha256",
                "recovery_receipt_sha256",
                "combined_receipt_sha256",
            )
        )
        or not _is_hex(agent.get("host_binary_sha256"), 64)
        or agent.get("host_binary_sha256") != expected_host_sha256
    ):
        raise RuntimeError("agent restart evidence is incomplete or not bound to the tested host")
    return evidence


def _is_mission_id(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"mission-v1-[0-9a-f]{64}", value) is not None


def validate_admission_result(
    admission: dict[str, object], expected_allocation_id: str
) -> dict[str, object]:
    if not re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", expected_allocation_id):
        raise RuntimeError("expected allocation ID is invalid")
    mission = admission.get("mission")
    repository = admission.get("repository")
    webhook = admission.get("webhook")
    refusals = admission.get("refusals")
    cleanup = admission.get("process_cleanup")
    cli_admission = mission.get("admission") if isinstance(mission, dict) else None
    if not all(
        isinstance(value, dict)
        for value in (mission, repository, webhook, refusals, cleanup, cli_admission)
    ):
        raise RuntimeError("admission stage omitted its mission, webhook, policy, refusal, or cleanup record")
    if mission.get("allocation_id") != expected_allocation_id:
        raise RuntimeError("admission mission does not match the requested allocation ID")
    host_admission = cli_admission.get("host_admission")
    allocation = host_admission.get("allocation") if isinstance(host_admission, dict) else None
    if not isinstance(allocation, dict) or allocation.get("allocation_id") != expected_allocation_id:
        raise RuntimeError("admission claim does not match the requested allocation ID")
    request = host_admission.get("request") if isinstance(host_admission, dict) else None
    mission_id = allocation.get("mission_id")
    if (
        not _is_mission_id(mission_id)
        or not isinstance(request, dict)
        or request.get("mission_id") != mission_id
    ):
        raise RuntimeError("admission claim does not bind a valid mission ID to its execution request")

    event = webhook.get("event")
    tasks = webhook.get("task_readback")
    try:
        if not isinstance(event, dict) or not isinstance(tasks, list):
            raise ValueError("Forgejo event or task readback is missing")
        binding = live_admission.validate_single_hook_task_binding(tasks, event)
        live_admission.validate_admission_result(
            cli_admission,
            event=event,
            requirement_sha256=str(mission.get("requirement_sha256")),
            base_commit=str(mission.get("protected_base_commit")),
            allocation_id=expected_allocation_id,
        )
    except (AttributeError, TypeError, ValueError):
        raise RuntimeError("admission claim is not bound to the successful Forgejo webhook event") from None

    reported_binding = webhook.get("task_binding")
    invalid_credential = refusals.get("invalid_host_credential")
    replay = refusals.get("replay")
    if (
        admission.get("schema_version") != 1
        or admission.get("verdict") != "passed"
        or mission.get("issue_number") != 1
        or mission.get("issue_state") != "open"
        or mission.get("profile_id") != "mission-012-clamp"
        or mission.get("requirement_path") != "requirements/MISSION-012-CLAMP-001.json"
        or mission.get("admission_file_mode") != "0o600"
        or mission.get("mission_journal_mode") != "0o600"
        or mission.get("agent_runtime_invoked") is not False
        or mission.get("allocation_id") != expected_allocation_id
        or not _is_hex(mission.get("protected_base_commit"), 40)
        or not _is_hex(mission.get("requirement_sha256"), 64)
        or not _is_hex(mission.get("admission_file_sha256"), 64)
        or not isinstance(reported_binding, dict)
        or reported_binding != binding
        or webhook.get("active_after_run") is not False
        or webhook.get("receiver_journal_records") != 1
        or webhook.get("secret_recorded") is not False
        or repository.get("full_name") != "forge-admin/mission-012-clamp"
        or repository.get("private") is not True
        or repository.get("branch_protection_readback_verified") is not True
        or repository.get("reviewer_login") != "verifier-bot"
        or repository.get("merge_whitelist_usernames") != ["forge-admin"]
        or repository.get("merge_whitelist_teams") != []
        or not isinstance(invalid_credential, dict)
        or invalid_credential.get("exit_status") != 1
        or invalid_credential.get("error_code") != "mission_source_unauthorized"
        or invalid_credential.get("mission_journal_created") is not False
        or invalid_credential.get("admission_file_created") is not False
        or not isinstance(replay, dict)
        or replay.get("verdict") != "already_claimed"
        or replay.get("allocation_replayed") is not True
        or replay.get("mission_journal_unchanged") is not True
        or replay.get("admission_file_unchanged") is not True
        or replay.get("receiver_inbox_records") != 1
        or cleanup.get("cleanup_failure") is not None
        or cleanup.get("hook_disabled_before_server_stop") is not True
        or cleanup.get("tracked_processes_exited") is not True
        or cleanup.get("expected_tcp_listeners_absent") is not True
    ):
        raise RuntimeError("admission stage did not prove real delivery, protected claim, refusals, and cleanup")

    result = {
        "schema_version": 1,
        "admission_mode": "webhook_delivery",
        "base_commit": mission["protected_base_commit"],
        "mission_id": mission_id,
        "requirement_sha256": mission["requirement_sha256"],
        "allocation_id": expected_allocation_id,
        "host_admission_sha256": mission["admission_file_sha256"],
        "untrusted_work_item_admitted": request.get("untrusted_work_item") is not None,
        "webhook": {
            "event": event,
            "task_binding": binding,
            "active_after_run": False,
            "receiver_journal_records": 1,
            "invalid_host_credential": invalid_credential,
            "replay": replay,
        },
    }
    return result


def validate_test_adapter_issue_observation(
    admission: dict[str, object],
    agent: dict[str, object],
    *,
    agent_adapter: str,
) -> None:
    observed = agent.get("test_adapter_untrusted_work_item_observed")
    if agent_adapter == "local-test-adapter":
        expected = admission.get("untrusted_work_item_admitted")
        if type(expected) is not bool or type(observed) is not bool or observed is not expected:
            raise RuntimeError("test-adapter observation does not match admitted issue context")
        return
    if agent_adapter == "claude":
        if observed is not None:
            raise RuntimeError("model run cannot claim a test-adapter observation")
        return
    raise RuntimeError("agent adapter cannot report test-adapter issue observation")


def validate_agent_result(
    admission: dict[str, object],
    agent: dict[str, object],
    budget_cap_usd: str,
    expected_allocation_id: str,
    expected_mcp_host_sha256: str,
    agent_adapter: str = "claude",
    require_host_restart: bool = False,
    expected_webhook_handoff: dict[str, str] | None = None,
) -> None:
    if isinstance(admission.get("mission"), dict):
        admission = validate_admission_result(admission, expected_allocation_id)
    elif (
        admission.get("schema_version") != 1
        or admission.get("admission_mode") != "webhook_delivery"
        or not _is_hex(admission.get("base_commit"), 40)
        or not _is_hex(admission.get("requirement_sha256"), 64)
        or admission.get("allocation_id") != expected_allocation_id
        or not _is_hex(admission.get("host_admission_sha256"), 64)
        or type(admission.get("untrusted_work_item_admitted")) is not bool
    ):
        raise RuntimeError("agent stage received an invalid normalized admission binding")
    budget = parse_budget(budget_cap_usd)
    if agent_adapter not in ("claude", "local-test-adapter"):
        raise RuntimeError("expected agent adapter is invalid")
    expected_model = "claude-opus-5-5" if agent_adapter == "claude" else "local-test-adapter"
    expected_effort = "high" if agent_adapter == "claude" else "deterministic"
    if (
        agent.get("status") != "candidate_ready_for_independent_verification"
        or agent.get("agent_adapter") != agent_adapter
        or agent.get("model") != expected_model
        or agent.get("effort") != expected_effort
        or agent.get("base_commit") != admission.get("base_commit")
        or agent.get("admission_sha256") != admission.get("host_admission_sha256")
        or not _is_hex(agent.get("host_binary_sha256"), 64)
        or not _is_hex(agent.get("receipt_sha256"), 64)
        or not isinstance(agent.get("receipt_count"), int)
        or agent["receipt_count"] <= 0
        or agent.get("changed_paths") != ["src/lib.rs"]
        or not isinstance(agent.get("candidate_file_sha256"), dict)
        or set(agent["candidate_file_sha256"]) != {"src/lib.rs"}
        or not _is_hex(agent["candidate_file_sha256"].get("src/lib.rs"), 64)
    ):
        raise RuntimeError("agent candidate or MCP receipt is not bound to the admitted task")
    validate_test_adapter_issue_observation(
        admission,
        agent,
        agent_adapter=agent_adapter,
    )
    webhook = admission.get("webhook")
    event = webhook.get("event") if isinstance(webhook, dict) else None
    if (
        not isinstance(event, dict)
        or agent.get("admission_source_mode") != "webhook_delivery"
        or agent.get("admission_source_delivery_id") != event.get("delivery_id")
        or not _is_hex(agent.get("admission_source_sha256"), 64)
        or not _is_hex(agent.get("webhook_inbox_sha256"), 64)
    ):
        raise RuntimeError("MCP host did not report the verified webhook admission handoff")
    if expected_webhook_handoff is not None and (
        agent.get("admission_source_delivery_id") != expected_webhook_handoff.get("delivery_id")
        or agent.get("admission_source_sha256") != expected_webhook_handoff.get("admission_source_sha256")
        or agent.get("webhook_inbox_sha256") != expected_webhook_handoff.get("webhook_inbox_sha256")
    ):
        raise RuntimeError("MCP host webhook handoff digests differ from the trusted admission files")
    if (
        not _is_hex(expected_mcp_host_sha256, 64)
        or agent.get("host_binary_sha256") != expected_mcp_host_sha256
    ):
        raise RuntimeError("MCP host binary hash is not bound to the verified build provenance")
    if require_host_restart:
        validate_host_restart_evidence(agent, expected_mcp_host_sha256)
    if agent_adapter == "local-test-adapter":
        reported_cost = agent.get("reported_cost_usd")
        if (
            agent.get("model_invoked_during_run") is not False
            or isinstance(reported_cost, bool)
            or reported_cost not in (0, 0.0, "0", "0.0")
        ):
            raise RuntimeError("local test adapter did not prove a no-model execution")
        try:
            actual_cap = Decimal(str(agent.get("budget_cap_usd")))
        except InvalidOperation:
            raise RuntimeError("local test adapter summary did not report its configured cap") from None
        if not actual_cap.is_finite() or actual_cap != budget:
            raise RuntimeError("local test adapter summary has a different configured cap")
        return
    if agent.get("model_invoked_during_run") is not True:
        raise RuntimeError("Claude result does not prove that a model was invoked")
    try:
        actual_cap = Decimal(str(agent.get("budget_cap_usd")))
    except InvalidOperation:
        raise RuntimeError("agent summary did not report a valid model budget cap") from None
    if not actual_cap.is_finite() or actual_cap <= 0 or actual_cap > budget:
        raise RuntimeError("agent summary reported a budget above the configured model cap")
    reported_cost = agent.get("reported_cost_usd")
    if reported_cost is None:
        raise RuntimeError("Claude Code omitted its reported cost; refusing an unmeasured candidate")
    try:
        cost = Decimal(str(reported_cost))
    except InvalidOperation:
        raise RuntimeError("model reported an invalid cost value") from None
    if not cost.is_finite() or cost < 0 or cost > actual_cap:
        raise RuntimeError("model cost exceeded the agent-reported budget cap")


def validate_joined_results(
    admission: dict[str, object],
    agent: dict[str, object],
    pull_request: dict[str, object],
    budget_cap_usd: str,
    expected_allocation_id: str,
    expected_mcp_host_sha256: str,
    agent_adapter: str = "claude",
    require_host_restart: bool = False,
) -> dict[str, object]:
    admission = validate_admission_result(admission, expected_allocation_id)
    validate_agent_result(
        admission, agent, budget_cap_usd, expected_allocation_id,
        expected_mcp_host_sha256, agent_adapter,
        require_host_restart=require_host_restart,
    )
    source_hash = agent["candidate_file_sha256"]["src/lib.rs"]
    if (
        pull_request.get("status") != "merged_in_private_sandbox"
        or pull_request.get("base_commit") != admission["base_commit"]
        or pull_request.get("requirement_sha256") != admission["requirement_sha256"]
        or pull_request.get("policy_sha256") != pr_driver.POLICY_SHA
        or pull_request.get("verifier") != "pass"
        or pull_request.get("issue_state") != "closed"
        or pull_request.get("changed_paths") != ["src/lib.rs"]
        or pull_request.get("candidate_sha256") != source_hash
        or not isinstance(pull_request.get("pull_request"), int)
        or pull_request["pull_request"] <= 0
        or not _is_hex(pull_request.get("candidate_commit"), 40)
        or not _is_hex(pull_request.get("merge_commit"), 40)
        or pull_request.get("pre_review_merge_refusal_http") != 405
        or pull_request.get("reviewer_merge_refusal_http") != 405
        or pull_request.get("reviewer") != "verifier-bot"
        or pull_request.get("merge_authority") != "forgejo-reconcile-local-rust"
        or not _is_hex(pull_request.get("merge_gate_policy_sha256"), 64)
        or not _is_hex(pull_request.get("merge_gate_binary_sha256"), 64)
    ):
        raise RuntimeError("PR readback or independent verifier is not bound to the agent candidate")
    verifier_cpu_accounting = validate_pr_verifier_cpu_accounting(
        pull_request.get("verifier_cpu_accounting"),
        base_commit=admission["base_commit"],
        candidate_commit=pull_request["candidate_commit"],
        expected_mcp_host_sha256=expected_mcp_host_sha256,
        expected_mission_id=admission["mission_id"],
    )
    webhook_reconciliation = validate_pr_webhook_reconciliation(
        pull_request.get("webhook_reconciliation"),
        pull_request=pull_request.get("pull_request"),
        base_commit=pull_request.get("base_commit"),
        candidate_commit=pull_request.get("candidate_commit"),
        merge_commit=pull_request.get("merge_commit"),
    )
    return {
        "schema_version": 2,
        "admission_mode": admission["admission_mode"],
        "base_commit": admission["base_commit"],
        "mission_id": admission["mission_id"],
        "requirement_sha256": admission["requirement_sha256"],
        "allocation_id": admission["allocation_id"],
        "admission_sha256": admission["host_admission_sha256"],
        "model": agent["model"],
        "effort": agent["effort"],
        "agent_adapter": agent_adapter,
        "model_invoked_during_run": agent.get("model_invoked_during_run", agent_adapter == "claude"),
        "budget_cap_usd": str(parse_budget(budget_cap_usd)),
        "reported_cost_usd": agent.get("reported_cost_usd"),
        "reported_cost_provenance": (
            "deterministic local adapter; no model call"
            if agent_adapter == "local-test-adapter"
            else "reported by local Claude CLI; not independently attested"
        ),
        "receipt_sha256": agent["receipt_sha256"],
        "mcp_host_binary_sha256": agent["host_binary_sha256"],
        "candidate_sha256": source_hash,
        "changed_paths": ["src/lib.rs"],
        "pull_request": pull_request["pull_request"],
        "candidate_commit": pull_request["candidate_commit"],
        "verifier": pull_request["verifier"],
        "verifier_cpu_accounting": verifier_cpu_accounting,
        "reviewer": pull_request["reviewer"],
        "pre_review_merge_refusal_http": pull_request["pre_review_merge_refusal_http"],
        "reviewer_merge_refusal_http": pull_request["reviewer_merge_refusal_http"],
        "merge_authority": pull_request["merge_authority"],
        "merge_gate_policy_sha256": pull_request["merge_gate_policy_sha256"],
        "merge_gate_binary_sha256": pull_request["merge_gate_binary_sha256"],
        "merge_commit": pull_request["merge_commit"],
        **(
            {"host_restart_recovery": agent["restart_recovery"]}
            if require_host_restart
            else {}
        ),
        "issue_state": pull_request["issue_state"],
        "webhook_reconciliation": webhook_reconciliation,
        "status": pull_request["status"],
    }


def _is_loopback_webhook_target(value: object) -> bool:
    if not isinstance(value, str):
        return False
    match = re.fullmatch(r"http://127\.0\.0\.1:([1-9][0-9]{0,4})/forgejo/webhook", value)
    return match is not None and 1 <= int(match.group(1)) <= 65535


def _valid_pr_reconciliation_receipt(
    value: object,
    *,
    delivery_id: str,
    pull_request: int,
    candidate_commit: str,
    merge_commit: str,
) -> bool:
    if not isinstance(value, dict):
        return False
    resource = value.get("resource")
    canonical = value.get("canonical_state")
    return (
        value.get("delivery_id") == delivery_id
        and type(value.get("exit_code")) is int
        and value.get("exit_code") == 0
        and value.get("verdict") == "current_state_read"
        and type(value.get("upstream_read_attempts")) is int
        and value.get("upstream_read_attempts") == 1
        and isinstance(resource, dict)
        and resource.get("kind") == "pull_request"
        and resource.get("owner") == "forge-admin"
        and resource.get("repository") == "mission-012-clamp"
        and type(resource.get("number")) is int
        and resource.get("number") == pull_request
        and isinstance(canonical, dict)
        and canonical.get("state") == "closed"
        and canonical.get("merged") is True
        and canonical.get("head_sha") == candidate_commit
        and canonical.get("base_sha") == merge_commit
        and canonical.get("merge_commit_sha") == merge_commit
    )


def validate_pr_webhook_reconciliation(
    value: object,
    *,
    pull_request: object,
    base_commit: object,
    candidate_commit: object,
    merge_commit: object,
) -> dict[str, object]:
    delivery_id_pattern = r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
    if not isinstance(value, dict):
        raise RuntimeError("PR webhook reconciliation evidence is missing")
    cleanup = value.get("cleanup")
    hook = value.get("hook_readback")
    canonical = value.get("canonical_state")
    approved_review = value.get("approved_review_notification")
    opened_payload = value.get("opened_payload")
    closed_payload = value.get("closed_payload")
    opened_id = value.get("opened_delivery_id")
    approved_review_id = value.get("approved_review_delivery_id")
    closed_id = value.get("closed_delivery_id")
    opened_sequence = value.get("opened_sequence")
    approved_review_sequence = value.get("approved_review_sequence")
    closed_sequence = value.get("closed_sequence")
    reconciliation_receipts = value.get("reconciliation_receipts")
    if not isinstance(reconciliation_receipts, list) or len(reconciliation_receipts) != 3:
        raise RuntimeError("PR webhook reconciliation receipt evidence is missing or incomplete")
    expected_delivery_ids = (opened_id, approved_review_id, closed_id)
    receipt_ids = [
        receipt.get("delivery_id") if isinstance(receipt, dict) else None
        for receipt in reconciliation_receipts
    ]
    receipt_ids_are_valid = all(
        isinstance(delivery_id, str)
        and re.fullmatch(delivery_id_pattern, delivery_id) is not None
        for delivery_id in (*expected_delivery_ids, *receipt_ids)
    )
    receipts_valid = (
        receipt_ids_are_valid
        and len(set(receipt_ids)) == 3
        and set(receipt_ids) == set(expected_delivery_ids)
        and all(
            _valid_pr_reconciliation_receipt(
                receipt,
                delivery_id=receipt["delivery_id"],
                pull_request=pull_request,
                candidate_commit=candidate_commit,
                merge_commit=merge_commit,
            )
            for receipt in reconciliation_receipts
        )
    )
    if (
        type(value.get("schema_version")) is not int
        or value.get("schema_version") != 5
        or value.get("status") != "verified_current_merged_pr"
        or value.get("target_profile") != "mission-012-clamp"
        or isinstance(pull_request, bool)
        or not isinstance(pull_request, int)
        or not _is_hex(base_commit, 40)
        or type(value.get("pull_request_number")) is not int
        or value.get("pull_request_number") != pull_request
        or value.get("candidate_commit") != candidate_commit
        or value.get("merge_commit") != merge_commit
        or not _is_hex(candidate_commit, 40)
        or not _is_hex(merge_commit, 40)
        or not isinstance(value.get("hook_id"), int)
        or isinstance(value.get("hook_id"), bool)
        or value.get("hook_id", 0) < 1
        or not _is_hex(value.get("receiver_binary_sha256"), 64)
        or type(value.get("event_count")) is not int
        or value.get("event_count") != 3
        or type(value.get("forgejo_task_count")) is not int
        or value.get("forgejo_task_count") != 3
        or type(value.get("successful_http_202_count")) is not int
        or value.get("successful_http_202_count") != 3
        or not isinstance(opened_id, str)
        or re.fullmatch(delivery_id_pattern, opened_id) is None
        or not isinstance(approved_review_id, str)
        or re.fullmatch(delivery_id_pattern, approved_review_id) is None
        or not isinstance(closed_id, str)
        or re.fullmatch(delivery_id_pattern, closed_id) is None
        or len({opened_id, approved_review_id, closed_id}) != 3
        or any(
            not isinstance(sequence, int) or isinstance(sequence, bool) or sequence < 1
            for sequence in (opened_sequence, approved_review_sequence, closed_sequence)
        )
        or len({opened_sequence, approved_review_sequence, closed_sequence}) != 3
        or value.get("reconciled_delivery_ids") != [opened_id, approved_review_id, closed_id]
        or not receipts_valid
        or value.get("all_notifications_reconciled_to_current_state") is not True
        or value.get("review_notification_is_approval_proof") is not False
        or not isinstance(opened_payload, dict)
        or opened_payload.get("state") != "open"
        or opened_payload.get("merged") is not False
        or opened_payload.get("head_sha") != candidate_commit
        or opened_payload.get("base_sha") != base_commit
        or not isinstance(closed_payload, dict)
        or closed_payload.get("state") != "closed"
        or closed_payload.get("merged") is not True
        or closed_payload.get("head_sha") != candidate_commit
        or closed_payload.get("base_sha") != merge_commit
        or closed_payload.get("merge_commit_sha") != merge_commit
        or not isinstance(approved_review, dict)
        or approved_review.get("event") != "pull_request_approved"
        or approved_review.get("action") != "reviewed"
        or approved_review.get("review_type") != "pull_request_review_approved"
        or type(approved_review.get("pull_request_number")) is not int
        or approved_review.get("pull_request_number") != pull_request
        or approved_review.get("head_sha") != candidate_commit
        or approved_review.get("base_sha") != base_commit
        or approved_review.get("payload_state") != "open"
        or approved_review.get("payload_merged") is not False
        or "review_event_window" in value
        or not isinstance(canonical, dict)
        or canonical.get("state") != "closed"
        or canonical.get("merged") is not True
        or canonical.get("head_sha") != candidate_commit
        or canonical.get("base_sha") != merge_commit
        or canonical.get("merge_commit_sha") != merge_commit
        or not isinstance(hook, dict)
        or hook.get("type") != "gitea"
        or hook.get("content_type") != "json"
        or not isinstance(hook.get("events"), list)
        or "pull_request" not in hook.get("events", [])
        or hook.get("configuration_matches_expected") is not True
        or hook.get("active_after_run") is not False
        or hook.get("deactivation_readback_confirmed") is not True
        or hook.get("secret_value_recorded") is not False
        or not _is_loopback_webhook_target(hook.get("target"))
        or not isinstance(cleanup, dict)
        or cleanup.get("hook_deactivated") is not True
        or cleanup.get("receiver_stopped") is not True
        or cleanup.get("receiver_listener_absent") is not True
    ):
        raise RuntimeError("PR webhook reconciliation evidence is not bound to the merged candidate")
    return value


def concurrent_change_evidence_is_valid(
    concurrent: object,
    *,
    same_file_conflict: bool,
    expected_base: object,
    expected_main: object,
) -> bool:
    if not isinstance(concurrent, dict):
        return False
    if (
        concurrent.get("schema_version") != 1
        or concurrent.get("base_before") != expected_base
        or concurrent.get("main_after") != expected_main
        or concurrent.get("merged") is not True
        or concurrent.get("merge_http_status") != 200
        or concurrent.get("same_file_conflict") is not same_file_conflict
    ):
        return False
    if same_file_conflict:
        return (
            concurrent.get("changed_path") == "src/lib.rs"
            and _is_hex(concurrent.get("main_source_sha256"), 64)
            and concurrent.get("git_merge_tree_exit_code") == 1
            and concurrent.get("git_merge_conflict_paths") == ["src/lib.rs"]
            and _is_hex(concurrent.get("git_merge_tree_stdout_sha256"), 64)
            and concurrent.get("readme_marker_sha256") is None
        )
    return (
        concurrent.get("changed_path") == "README.md"
        and concurrent.get("main_source_sha256") is None
        and concurrent.get("git_merge_tree_exit_code") is None
        and concurrent.get("git_merge_conflict_paths") == []
        and concurrent.get("git_merge_tree_stdout_sha256") is None
        and _is_hex(concurrent.get("readme_marker_sha256"), 64)
    )


def validate_stale_base_refusal(
    admission: dict[str, object],
    agent: dict[str, object],
    pull_request: dict[str, object],
    budget_cap_usd: str,
    expected_allocation_id: str,
    expected_mcp_host_sha256: str,
    agent_adapter: str = "claude",
    same_file_conflict: bool = False,
) -> dict[str, object]:
    admission = validate_admission_result(admission, expected_allocation_id)
    validate_agent_result(
        admission, agent, budget_cap_usd, expected_allocation_id,
        expected_mcp_host_sha256, agent_adapter,
    )
    concurrent = pull_request.get("concurrent_base_advance")
    if not isinstance(concurrent, dict):
        raise RuntimeError("concurrent-change refusal omitted its approved competing PR readback")
    expected_main_source_sha = (
        concurrent.get("main_source_sha256") if same_file_conflict else pr_driver.BROKEN_SOURCE_SHA
    )
    expected_status = (
        "refused_after_concurrent_code_conflict_before_merge"
        if same_file_conflict else "refused_after_concurrent_base_advance_before_merge"
    )
    concurrent_evidence_valid = concurrent_change_evidence_is_valid(
        concurrent,
        same_file_conflict=same_file_conflict,
        expected_base=admission.get("base_commit"),
        expected_main=pull_request.get("main_head_after_advance"),
    )
    error_code = pull_request.get("merge_gate_error_code")
    if (
        pull_request.get("schema_version") != 1
        or pull_request.get("status") != expected_status
        or pull_request.get("base_commit") != admission.get("base_commit")
        or pull_request.get("requirement_sha256") != admission.get("requirement_sha256")
        or pull_request.get("policy_sha256") != pr_driver.POLICY_SHA
        or pull_request.get("candidate_sha256") != agent.get("candidate_file_sha256", {}).get("src/lib.rs")
        or pull_request.get("changed_paths") != ["src/lib.rs"]
        or pull_request.get("verifier") != "pass"
        or pull_request.get("reviewer") != "verifier-bot"
        or pull_request.get("pre_review_merge_refusal_http") != 405
        or pull_request.get("reviewer_merge_refusal_http") != 405
        or pull_request.get("issue_state") != "open"
        or pull_request.get("target_pull_request_state") != "open"
        or pull_request.get("target_pull_request_merged") is not False
        or pull_request.get("target_merge_commit") is not None
        or pull_request.get("target_pull_request_base_sha_after_advance")
        not in {admission.get("base_commit"), pull_request.get("main_head_after_advance")}
        or pull_request.get("main_head_before_advance") != admission.get("base_commit")
        or not _is_hex(pull_request.get("main_head_after_advance"), 40)
        or pull_request.get("main_head_after_advance") == admission.get("base_commit")
        or pull_request.get("main_source_sha256_after_refusal") != expected_main_source_sha
        or pull_request.get("same_file_conflict") is not same_file_conflict
        or not isinstance(pull_request.get("pull_request"), int)
        or isinstance(pull_request.get("pull_request"), bool)
        or pull_request.get("pull_request", 0) <= 0
        or pull_request.get("competing_pull_request_merged") is not True
        or pull_request.get("competing_merge_http_status") != 200
        or pull_request.get("merge_authority") != "forgejo-reconcile-local-rust"
        or not _is_hex(pull_request.get("merge_gate_policy_sha256"), 64)
        or not _is_hex(pull_request.get("merge_gate_binary_sha256"), 64)
        or pull_request.get("merge_gate_verdict") != "rejected"
        or pull_request.get("merge_authorized") is not False
        or pull_request.get("merge_attempted") is not False
        or error_code not in {
            "stale_base", "not_mergeable", "invalid_review_base_binding",
        }
        or concurrent.get("base_before") != admission.get("base_commit")
        or concurrent.get("competing_pull_request") != pull_request.get("competing_pull_request")
        or concurrent.get("merged") is not True
        or concurrent.get("merge_http_status") != 200
        or concurrent.get("main_after") != pull_request.get("main_head_after_advance")
        or concurrent.get("main_after") == admission.get("base_commit")
        or (
            same_file_conflict
            and pull_request.get("main_source_sha256_after_refusal")
            != concurrent.get("main_source_sha256")
        )
        or not concurrent_evidence_valid
        or not _is_hex(pull_request.get("candidate_commit"), 40)
    ):
        raise RuntimeError("concurrent-change refusal is not bound to the verified candidate and competing merge")
    verifier_cpu_accounting = validate_pr_verifier_cpu_accounting(
        pull_request.get("verifier_cpu_accounting"),
        base_commit=admission["base_commit"],
        candidate_commit=pull_request["candidate_commit"],
        expected_mcp_host_sha256=expected_mcp_host_sha256,
        expected_mission_id=admission["mission_id"],
    )
    return {
        "schema_version": 2,
        "admission_mode": admission["admission_mode"],
        "base_commit": admission["base_commit"],
        "mission_id": admission["mission_id"],
        "requirement_sha256": admission["requirement_sha256"],
        "allocation_id": admission["allocation_id"],
        "admission_sha256": admission["host_admission_sha256"],
        "model": agent["model"],
        "effort": agent["effort"],
        "agent_adapter": agent_adapter,
        "model_invoked_during_run": agent.get("model_invoked_during_run", agent_adapter == "claude"),
        "budget_cap_usd": str(parse_budget(budget_cap_usd)),
        "reported_cost_usd": agent.get("reported_cost_usd"),
        "reported_cost_provenance": (
            "deterministic local adapter; no model call"
            if agent_adapter == "local-test-adapter"
            else "reported by local Claude CLI; not independently attested"
        ),
        "receipt_sha256": agent["receipt_sha256"],
        "mcp_host_binary_sha256": agent["host_binary_sha256"],
        "candidate_sha256": agent["candidate_file_sha256"]["src/lib.rs"],
        "changed_paths": ["src/lib.rs"],
        "pull_request": pull_request["pull_request"],
        "candidate_commit": pull_request["candidate_commit"],
        "verifier": pull_request["verifier"],
        "verifier_cpu_accounting": verifier_cpu_accounting,
        "reviewer": pull_request["reviewer"],
        "pre_review_merge_refusal_http": pull_request["pre_review_merge_refusal_http"],
        "reviewer_merge_refusal_http": pull_request["reviewer_merge_refusal_http"],
        "merge_authority": pull_request["merge_authority"],
        "merge_gate_policy_sha256": pull_request["merge_gate_policy_sha256"],
        "merge_gate_binary_sha256": pull_request["merge_gate_binary_sha256"],
        "merge_gate_error_code": error_code,
        "merge_authorized": False,
        "merge_attempted": False,
        "main_head_after_advance": pull_request["main_head_after_advance"],
        "main_source_sha256_after_refusal": pull_request["main_source_sha256_after_refusal"],
        "same_file_conflict": pull_request["same_file_conflict"],
        "concurrent_base_advance": pull_request["concurrent_base_advance"],
        "competing_pull_request": pull_request["competing_pull_request"],
        "status": pull_request["status"],
        "issue_state": "open",
    }


def validate_pr_verifier_cpu_accounting(
    accounting: object,
    *,
    base_commit: str,
    candidate_commit: str,
    expected_mcp_host_sha256: str,
    expected_mission_id: str,
) -> dict[str, object]:
    final_ledger = accounting.get("final_ledger") if isinstance(accounting, dict) else None
    if (
        not isinstance(accounting, dict)
        or accounting.get("source") != PR_VERIFIER_CPU_LEDGER_SOURCE
        or accounting.get("verifier_binary_sha256") != expected_mcp_host_sha256
        or accounting.get("policy_sha256") != pr_driver.VERIFIER_CPU_POLICY_SHA256
        or accounting.get("mission_cpu_budget_ms") != pr_driver.MISSION_VERIFICATION_CPU_BUDGET_MS
        or accounting.get("charged_cpu_measurement") != PR_VERIFIER_CPU_MEASUREMENT
        or not _is_hex(expected_mcp_host_sha256, 64)
        or not _is_hex(base_commit, 40)
        or not _is_hex(candidate_commit, 40)
        or not _is_mission_id(expected_mission_id)
        or not isinstance(final_ledger, dict)
        or final_ledger.get("mission_id") != expected_mission_id
    ):
        raise RuntimeError("PR independent verifier CPU accounting is missing or not admission-bound")

    steps = accounting.get("steps")
    if not isinstance(steps, list) or len(steps) != len(EXPECTED_PR_VERIFIER_CPU_STEPS):
        raise RuntimeError("PR independent verifier CPU accounting must contain all three checks")
    for index, stage in enumerate(EXPECTED_PR_VERIFIER_CPU_STEPS):
        step = steps[index]
        tested_commit = base_commit if stage == "base-red" else candidate_commit
        reservation_id = pr_driver.verifier_reservation_id(stage, base_commit, candidate_commit)
        charged = step.get("charged_cpu_upper_bound_ms") if isinstance(step, dict) else None
        exit_code = step.get("exit_code") if isinstance(step, dict) else None
        if (
            not isinstance(step, dict)
            or step.get("stage") != stage
            or step.get("mission_id") != expected_mission_id
            or step.get("base_commit") != base_commit
            or step.get("candidate_commit") != candidate_commit
            or step.get("tested_commit") != tested_commit
            or step.get("reservation_id") != reservation_id
            or step.get("policy_sha256") != pr_driver.VERIFIER_CPU_POLICY_SHA256
            or step.get("reserved_cpu_ms") != pr_driver.VERIFIER_CPU_RESERVATION_MS
            or not isinstance(charged, int)
            or isinstance(charged, bool)
            or not 0 < charged <= pr_driver.VERIFIER_CPU_RESERVATION_MS
            or not isinstance(step.get("accounting_elapsed_ms"), int)
            or isinstance(step.get("accounting_elapsed_ms"), bool)
            or step["accounting_elapsed_ms"] <= 0
            or not isinstance(step.get("sandbox_elapsed_ms"), int)
            or isinstance(step.get("sandbox_elapsed_ms"), bool)
            or step["sandbox_elapsed_ms"] <= 0
            or step["sandbox_elapsed_ms"] > step["accounting_elapsed_ms"]
            or not isinstance(exit_code, int)
            or isinstance(exit_code, bool)
            or (stage == "base-red" and exit_code != 101)
            or (stage != "base-red" and exit_code != 0)
            or step.get("timed_out") is not False
            or step.get("output_truncated") is not False
            or not _is_hex(step.get("output_sha256"), 64)
            or step.get("expected_failure") != (
                pr_driver.EXPECTED_BASE_FAILURE if stage == "base-red" else None
            )
        ):
            raise RuntimeError("PR independent verifier CPU step is missing, stale, or malformed")
        encoded_output = step.get("output_b64")
        if not isinstance(encoded_output, str):
            raise RuntimeError("verifier receipt omitted its raw output")
        try:
            raw_output = base64.b64decode(encoded_output, validate=True)
        except (ValueError, TypeError):
            raise RuntimeError("verifier receipt contains malformed output") from None
        if (
            len(raw_output) > MAX_VERIFIER_OUTPUT_BYTES
            or hashlib.sha256(raw_output).hexdigest() != step.get("output_sha256")
        ):
            raise RuntimeError("verifier output does not match its bounded SHA-256 receipt")
        if stage == "base-red":
            required_output = (
                pr_driver.EXPECTED_BASE_FAILURE["test_name"].encode(),
                pr_driver.EXPECTED_BASE_FAILURE["assertion"].encode(),
                f"left: {pr_driver.EXPECTED_BASE_FAILURE['actual']}".encode(),
                f"right: {pr_driver.EXPECTED_BASE_FAILURE['expected']}".encode(),
                pr_driver.EXPECTED_BASE_FAILURE["summary"].encode(),
            )
            if (
                any(marker not in raw_output for marker in required_output)
            ):
                raise RuntimeError("base-red receipt is not bound to the expected acceptance assertion output")
        else:
            required_output = (
                *(f"test {test_name} ... ok".encode() for test_name in EXPECTED_GREEN_ACCEPTANCE_TESTS),
                b"test result: ok. 2 passed; 0 failed",
            )
            if any(marker not in raw_output for marker in required_output):
                raise RuntimeError("green verifier output omitted a required passing acceptance test")
        expected_charge = min(
            pr_driver.VERIFIER_CPU_RESERVATION_MS,
            max(
                1,
                (
                    (step["accounting_elapsed_ms"] + pr_driver.VERIFIER_CPU_ACCOUNTING_MARGIN_MS)
                    * pr_driver.VERIFIER_CPU_QUOTA_PERCENT
                    + 99
                )
                // 100,
            ),
        )
        if charged != expected_charge:
            raise RuntimeError("PR independent verifier CPU step charge does not match the pinned upper-bound formula")

    try:
        ledger = pr_driver.validate_final_verifier_cpu_ledger(
            accounting.get("final_ledger"), steps,
            expected_mission_id=expected_mission_id,
        )
    except RuntimeError as error:
        raise RuntimeError(f"PR independent verifier CPU ledger is invalid: {error}") from None
    return {**accounting, "final_ledger": ledger}


def validate_stale_base_readback(
    readback: dict[str, object], joined: dict[str, object], *, same_file_conflict: bool = False
) -> None:
    concurrent = readback.get("concurrent_base_advance")
    expected_main_source_sha = (
        concurrent.get("main_source_sha256")
        if isinstance(concurrent, dict) and same_file_conflict
        else pr_driver.BROKEN_SOURCE_SHA
    )
    expected_status = (
        "refused_after_concurrent_code_conflict_before_merge"
        if same_file_conflict else "refused_after_concurrent_base_advance_before_merge"
    )
    if (
        readback.get("status") != expected_status
        or readback.get("verifier") != "pass"
        or readback.get("base_commit") != joined.get("base_commit")
        or readback.get("requirement_sha256") != joined.get("requirement_sha256")
        or readback.get("verifier_cpu_accounting") != joined.get("verifier_cpu_accounting")
        or readback.get("policy_sha256") != pr_driver.POLICY_SHA
        or readback.get("candidate_commit") != joined.get("candidate_commit")
        or readback.get("candidate_sha256") != joined.get("candidate_sha256")
        or readback.get("changed_paths") != ["src/lib.rs"]
        or readback.get("issue_state") != "open"
        or readback.get("target_pull_request_state") != "open"
        or readback.get("target_pull_request_merged") is not False
        or readback.get("target_merge_commit") is not None
        or readback.get("main_head_after_advance") != joined.get("main_head_after_advance")
        or readback.get("main_head_after_advance") == joined.get("base_commit")
        or readback.get("main_source_sha256_after_refusal") != expected_main_source_sha
        or readback.get("same_file_conflict") is not same_file_conflict
        or readback.get("merge_authorized") is not False
        or readback.get("merge_attempted") is not False
        or readback.get("merge_gate_error_code") not in {
            "stale_base", "not_mergeable", "invalid_review_base_binding",
        }
        or readback.get("target_pull_request_base_sha_after_advance")
        not in {joined.get("base_commit"), joined.get("main_head_after_advance")}
        or not isinstance(concurrent, dict)
        or concurrent.get("merged") is not True
        or concurrent.get("main_after") != joined.get("main_head_after_advance")
        or concurrent != joined.get("concurrent_base_advance")
        or (
            same_file_conflict
            and (
                concurrent.get("changed_path") != "src/lib.rs"
                or concurrent.get("main_source_sha256") != readback.get("main_source_sha256_after_refusal")
                or concurrent.get("git_merge_tree_exit_code") != 1
                or concurrent.get("git_merge_conflict_paths") != ["src/lib.rs"]
                or not _is_hex(concurrent.get("git_merge_tree_stdout_sha256"), 64)
            )
        )
        or not concurrent_change_evidence_is_valid(
            concurrent,
            same_file_conflict=same_file_conflict,
            expected_base=readback.get("base_commit"),
            expected_main=readback.get("main_head_after_advance"),
        )
    ):
        raise RuntimeError("concurrent-change refusal readback does not prove that the target PR stayed unmerged")


def validate_candidate_mutation_refusal(
    admission_result: dict[str, object],
    agent_result: dict[str, object],
    pull_request: dict[str, object],
    budget_cap_usd: str,
    expected_allocation_id: str,
    expected_mcp_host_sha256: str,
    agent_adapter: str = "claude",
) -> dict[str, object]:
    admission = validate_admission_result(admission_result, expected_allocation_id)
    validate_agent_result(
        admission, agent_result, budget_cap_usd, expected_allocation_id,
        expected_mcp_host_sha256, agent_adapter,
    )
    candidate_sha = agent_result["candidate_file_sha256"]["src/lib.rs"]
    pr_driver.validate_candidate_mutation_refusal_result(
        pull_request,
        expected_base=admission["base_commit"],
        expected_candidate_commit=pull_request.get("candidate_commit"),
        expected_candidate_sha256=candidate_sha,
    )
    candidate_commit = pull_request["candidate_commit"]
    mutation = pull_request["candidate_mutation"]
    verifier_cpu_accounting = validate_pr_verifier_cpu_accounting(
        pull_request.get("verifier_cpu_accounting"),
        base_commit=admission["base_commit"],
        candidate_commit=candidate_commit,
        expected_mcp_host_sha256=expected_mcp_host_sha256,
        expected_mission_id=admission["mission_id"],
    )
    return {
        "schema_version": 2,
        "admission_mode": admission["admission_mode"],
        "base_commit": admission["base_commit"],
        "mission_id": admission["mission_id"],
        "requirement_sha256": admission["requirement_sha256"],
        "allocation_id": admission["allocation_id"],
        "admission_sha256": admission["host_admission_sha256"],
        "model": agent_result["model"],
        "effort": agent_result["effort"],
        "agent_adapter": agent_adapter,
        "model_invoked_during_run": agent_result.get(
            "model_invoked_during_run", agent_adapter == "claude"
        ),
        "budget_cap_usd": str(parse_budget(budget_cap_usd)),
        "reported_cost_usd": agent_result.get("reported_cost_usd"),
        "reported_cost_provenance": (
            "deterministic local adapter; no model call"
            if agent_adapter == "local-test-adapter"
            else "reported by local Claude CLI; not independently attested"
        ),
        "receipt_sha256": agent_result["receipt_sha256"],
        "mcp_host_binary_sha256": agent_result["host_binary_sha256"],
        "candidate_sha256": candidate_sha,
        "changed_paths": ["src/lib.rs"],
        "pull_request": pull_request["pull_request"],
        "candidate_commit": candidate_commit,
        "candidate_mutation": mutation,
        "mutated_candidate_commit": mutation["head_after"],
        "mutated_candidate_sha256": mutation["source_sha256_after"],
        "verifier": pull_request["verifier"],
        "verifier_cpu_accounting": verifier_cpu_accounting,
        "reviewer": pull_request["reviewer"],
        "pre_review_merge_refusal_http": pull_request["pre_review_merge_refusal_http"],
        "reviewer_merge_refusal_http": pull_request["reviewer_merge_refusal_http"],
        "merge_authority": pull_request["merge_authority"],
        "merge_gate_policy_sha256": pull_request["merge_gate_policy_sha256"],
        "merge_gate_binary_sha256": pull_request["merge_gate_binary_sha256"],
        "merge_gate_error_code": pull_request["merge_gate_error_code"],
        "merge_authorized": False,
        "merge_attempted": False,
        "merge_commit": None,
        "issue_state": pull_request["issue_state"],
        "status": pull_request["status"],
    }


def validate_candidate_mutation_readback(
    readback: dict[str, object], joined: dict[str, object]
) -> None:
    try:
        pr_driver.validate_candidate_mutation_refusal_result(
            readback,
            expected_base=joined["base_commit"],
            expected_candidate_commit=joined["candidate_commit"],
            expected_candidate_sha256=joined["candidate_sha256"],
        )
    except (KeyError, TypeError):
        raise RuntimeError("candidate-mutation refusal readback lacks its source binding") from None
    if (
        readback.get("candidate_mutation") != joined.get("candidate_mutation")
        or readback.get("candidate_commit") != joined.get("candidate_commit")
        or readback.get("candidate_sha256") != joined.get("candidate_sha256")
        or readback.get("verifier_cpu_accounting") != joined.get("verifier_cpu_accounting")
        or readback.get("merge_gate_error_code") != joined.get("merge_gate_error_code")
        or readback.get("merge_gate_error_code") != "invalid_review_base_binding"
    ):
        raise RuntimeError("candidate-mutation refusal readback differs from the validated refusal")


def validate_postmerge_readback(
    readback: dict[str, object], joined: dict[str, object]
) -> None:
    if (
        readback.get("merged") is not True
        or readback.get("independent_verdict") != "pass"
        or readback.get("base_commit") != joined.get("base_commit")
        or readback.get("requirement_sha256") != joined.get("requirement_sha256")
        or readback.get("policy_sha256") != pr_driver.POLICY_SHA
        or readback.get("candidate_commit") != joined.get("candidate_commit")
        or readback.get("merge_commit") != joined.get("merge_commit")
        or readback.get("main_head_after_merge") != joined.get("merge_commit")
        or readback.get("candidate_source_sha256") != joined.get("candidate_sha256")
        or readback.get("changed_paths") != ["src/lib.rs"]
        or readback.get("issue_state_after_merge") != "closed"
        or readback.get("approval_commit") != joined.get("candidate_commit")
        or readback.get("host_receipt_sha256") != joined.get("receipt_sha256")
        or readback.get("merge_authority") != joined.get("merge_authority")
        or readback.get("merge_gate_policy_sha256") != joined.get("merge_gate_policy_sha256")
        or readback.get("merge_gate_binary_sha256") != joined.get("merge_gate_binary_sha256")
        or readback.get("webhook_reconciliation") != joined.get("webhook_reconciliation")
    ):
        raise RuntimeError("Forgejo post-merge readback is not bound to the admitted candidate and receipt")


def stage_environment() -> dict[str, str]:
    home = str(Path.home())
    return {
        "HOME": home,
        "PATH": os.pathsep.join((
            f"{home}/.local/bin",
            f"{home}/.cargo/bin",
            str(pr_driver.RUST_TOOLCHAIN / "bin"),
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
        )),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TERM": "dumb",
        "NO_COLOR": "1",
    }


def run_stage_command(
    command: list[str],
    timeout: int,
    *,
    gate_barrier_ready_file: Path | None = None,
    on_gate_barrier_ready: Callable[[Path], None] | None = None,
) -> tuple[int, bytes]:
    if (gate_barrier_ready_file is None) != (on_gate_barrier_ready is None):
        raise RuntimeError("a gate barrier path and callback must be supplied together")
    environment = stage_environment()
    try:
        scope = prepare_process_scope(command, environment, timeout)
    except ProcessScopeUnavailable as error:
        return 125, f"agent stage refused: {error}\n".encode("utf-8", errors="replace")
    started = launch_scoped_process(
        scope,
        command,
        environment,
        cwd=ROOT,
        timeout=timeout,
        merge_stderr=True,
    )
    process = started.process
    scope_cgroup = started.cgroup_path
    if process is None or scope_cgroup is None:
        diagnostic = b"\nagent stage refused: transient cgroup scope could not be attested before command start\n"
        return 125, (started.startup_stdout + started.startup_stderr)[:MAX_STAGE_OUTPUT_BYTES] + diagnostic
    return _capture_stage_command(
        process,
        timeout,
        gate_barrier_ready_file=gate_barrier_ready_file,
        on_gate_barrier_ready=on_gate_barrier_ready,
        finish_scope=lambda: finish_process_scope(scope, scope_cgroup),
        scope_description="transient cgroup",
    )


def run_stage_command_in_attested_parent_scope(
    command: list[str],
    timeout: int,
    *,
    gate_barrier_ready_file: Path | None = None,
    on_gate_barrier_ready: Callable[[Path], None] | None = None,
) -> tuple[int, bytes]:
    """Run one stage in the host-created outer scope and reject any PID residue."""
    if (gate_barrier_ready_file is None) != (on_gate_barrier_ready is None):
        raise RuntimeError("a gate barrier path and callback must be supplied together")
    if timeout <= 0:
        return 125, b"agent stage refused: command timeout must be positive\n"
    snapshot = current_attested_process_scope_snapshot()
    if snapshot is None or not process_scope_snapshot_matches(snapshot):
        return 125, b"agent stage refused: attested parent cgroup scope is unavailable or changed\n"
    environment = stage_environment()
    try:
        process = subprocess.Popen(
            command,
            cwd=ROOT,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=0,
            close_fds=True,
            start_new_session=True,
        )
    except OSError:
        return 125, b"agent stage refused: command could not start inside its attested parent scope\n"
    return _capture_stage_command(
        process,
        timeout,
        gate_barrier_ready_file=gate_barrier_ready_file,
        on_gate_barrier_ready=on_gate_barrier_ready,
        finish_scope=lambda: (
            "clean" if process_scope_snapshot_matches(snapshot) else "parent_membership_changed"
        ),
        scope_description="attested parent cgroup",
    )


def _capture_stage_command(
    process: subprocess.Popen[bytes],
    timeout: int,
    *,
    gate_barrier_ready_file: Path | None,
    on_gate_barrier_ready: Callable[[Path], None] | None,
    finish_scope: Callable[[], str],
    scope_description: str,
) -> tuple[int, bytes]:
    """Bound output, cancellation, and descendants for either scope strategy."""
    if process.stdout is None:
        return 125, b"agent stage refused: command output pipe is unavailable\n"
    assert process.stdout is not None
    output = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout
    timed_out = False
    gate_callback_failed = False
    gate_callback_called = False
    output_exceeded = False
    interrupted = False
    try:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                _terminate_process_group(process)
                break
            if gate_barrier_ready_file is not None and not gate_callback_called:
                try:
                    os.lstat(gate_barrier_ready_file)
                except FileNotFoundError:
                    pass
                except OSError:
                    gate_callback_failed = True
                else:
                    gate_callback_called = True
                    try:
                        assert on_gate_barrier_ready is not None
                        on_gate_barrier_ready(gate_barrier_ready_file)
                    except Exception:
                        gate_callback_failed = True
                if gate_callback_failed:
                    _terminate_process_group(process)
                    break
            if child_exit_is_waitable(process) and not selector.get_map():
                break
            poll_interval = 0.05 if gate_barrier_ready_file is not None and not gate_callback_called else 0.25
            events = selector.select(min(remaining, poll_interval)) if selector.get_map() else []
            for key, _ in events:
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                available = MAX_STAGE_OUTPUT_BYTES + 1 - len(output)
                output.extend(chunk[:available])
                if len(output) > MAX_STAGE_OUTPUT_BYTES:
                    output_exceeded = True
                    _terminate_process_group(process)
                    break
            if output_exceeded:
                break
            if child_exit_is_waitable(process) and not selector.get_map():
                break
            if not selector.get_map() and not child_exit_is_waitable(process):
                time.sleep(min(poll_interval, max(0.0, remaining)))
    except KeyboardInterrupt:
        interrupted = True
        _terminate_process_group(process)
    finally:
        selector.close()
        if process.stdin is not None:
            try:
                process.stdin.close()
            except (OSError, ValueError):
                pass
        process.stdout.close()
    cleanup_result = finish_exited_process_group(process, grace_seconds=0.2)
    scope_result = finish_scope()
    if interrupted:
        raise KeyboardInterrupt
    if (
        cleanup_result != "clean"
        or scope_result != "clean"
        or output_exceeded
        or gate_callback_failed
    ):
        diagnostic = (
            f"\nagent stage {scope_description} was not cleanly completed "
            f"(group={cleanup_result}, scope={scope_result}, "
            f"output_limit={output_exceeded}, callback_failure={gate_callback_failed}); "
            "the stage is inconclusive and its owning scope must be stopped\n"
        ).encode("ascii")
        retained = bytes(output[: MAX_STAGE_OUTPUT_BYTES - len(diagnostic)])
        return 125, retained + diagnostic
    if timed_out:
        return 124, bytes(output)
    return process.wait(), bytes(output)


def _terminate_process_group(process: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        if process.poll() is None:
            process.wait(timeout=5)
        return
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        process.poll()
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.05)
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    if process.poll() is None:
        process.wait(timeout=5)


StageRunner = Callable[[list[str], int], tuple[int, bytes]]


def validate_admission_evidence(
    evidence_dir: Path, stage_result: dict[str, object]
) -> str:
    manifest = evidence_dir / "SHA256SUMS"
    result_path = evidence_dir / "result.json"
    if (
        evidence_dir.is_symlink()
        or not evidence_dir.is_dir()
        or stat.S_IMODE(evidence_dir.stat().st_mode) != 0o700
        or manifest.is_symlink()
        or not manifest.is_file()
        or stat.S_IMODE(manifest.stat().st_mode) != 0o600
        or result_path.is_symlink()
        or not result_path.is_file()
        or stat.S_IMODE(result_path.stat().st_mode) != 0o600
    ):
        raise RuntimeError("admission evidence directory or receipts have unsafe file metadata")

    expected: dict[str, str] = {}
    for line in manifest.read_text().splitlines():
        if len(line) < 67 or line[64:66] != "  ":
            raise RuntimeError("admission evidence checksum manifest is malformed")
        digest_value, name = line[:64], line[66:]
        if (
            not _is_hex(digest_value, 64)
            or not re.fullmatch(r"[A-Za-z0-9._-]{1,128}", name)
            or name in {".", "..", "SHA256SUMS"}
            or name in expected
        ):
            raise RuntimeError("admission evidence checksum manifest contains an invalid entry")
        expected[name] = digest_value
    actual_names = {
        path.name for path in evidence_dir.iterdir()
        if path.name != "SHA256SUMS"
    }
    if not expected or set(expected) != actual_names:
        raise RuntimeError("admission evidence checksum manifest is incomplete")
    for name, digest_value in expected.items():
        path = evidence_dir / name
        if (
            path.is_symlink()
            or not path.is_file()
            or stat.S_IMODE(path.stat().st_mode) != 0o600
            or hashlib.sha256(path.read_bytes()).hexdigest() != digest_value
        ):
            raise RuntimeError("admission evidence checksum does not match its file")
    try:
        recorded_result = json.loads(result_path.read_bytes())
    except (OSError, json.JSONDecodeError):
        raise RuntimeError("admission result evidence is not valid JSON") from None
    if recorded_result != stage_result:
        raise RuntimeError("admission result evidence does not match the stage output")
    return hashlib.sha256(manifest.read_bytes()).hexdigest()


def validate_admission_build_provenance(
    evidence_dir: Path,
    stage_result: dict[str, object],
    *,
    expect_test_crash_host: bool = False,
) -> dict[str, str]:
    manifest_sha256 = validate_admission_evidence(evidence_dir, stage_result)
    provenance_path = evidence_dir / "provenance.json"
    if (
        provenance_path.is_symlink()
        or not provenance_path.is_file()
        or stat.S_IMODE(provenance_path.stat().st_mode) != 0o600
    ):
        raise RuntimeError("admission provenance receipt has unsafe file metadata")
    try:
        provenance = json.loads(provenance_path.read_bytes())
    except (OSError, json.JSONDecodeError):
        raise RuntimeError("admission provenance receipt is not valid JSON") from None
    executable_hashes = provenance.get("built_executable_sha256") if isinstance(provenance, dict) else None
    mcp_host_sha256 = (
        executable_hashes.get("mission-012-mcp-host")
        if isinstance(executable_hashes, dict)
        else None
    )
    test_crash_host_sha256 = (
        executable_hashes.get("mission-012-mcp-host-test-crash")
        if isinstance(executable_hashes, dict)
        else None
    )
    if (
        not isinstance(provenance, dict)
        or provenance.get("mcp_host_built") is not True
        or not _is_hex(mcp_host_sha256, 64)
        or provenance.get("test_crash_injection_host_built") is not expect_test_crash_host
        or (
            expect_test_crash_host
            and (
                not _is_hex(test_crash_host_sha256, 64)
                or test_crash_host_sha256 != mcp_host_sha256
            )
        )
    ):
        raise RuntimeError("admission provenance does not bind a built MCP host binary")
    return {
        "manifest_sha256": manifest_sha256,
        "mcp_host_binary_sha256": mcp_host_sha256,
    }


def _read_private_json(path: Path, *, maximum_bytes: int) -> tuple[dict[str, object], bytes]:
    if path.is_symlink():
        raise RuntimeError("private gate receipt must not be a symbolic link")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        metadata = os.fstat(fd)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.geteuid()
            or metadata.st_nlink != 1
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_size > maximum_bytes
        ):
            raise RuntimeError("private gate receipt metadata is unsafe")
        content = bytearray()
        while len(content) <= maximum_bytes:
            chunk = os.read(fd, min(65536, maximum_bytes + 1 - len(content)))
            if not chunk:
                break
            content.extend(chunk)
    finally:
        os.close(fd)
    if len(content) > maximum_bytes:
        raise RuntimeError("private gate receipt exceeded its size bound")
    try:
        parsed = json.loads(content)
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise RuntimeError("private gate receipt is not valid JSON") from None
    if not isinstance(parsed, dict):
        raise RuntimeError("private gate receipt must be a JSON object")
    return parsed, bytes(content)


def _write_private_json_exclusive(path: Path, value: object) -> bytes:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        if os.write(fd, payload) != len(payload):
            raise OSError("short private gate receipt write")
        os.fsync(fd)
    finally:
        os.close(fd)
    return payload


def _stable_live_executable(pid: int) -> dict[str, object]:
    first = pr_driver.linux_process_binding(pid)
    try:
        executable = os.readlink(f"/proc/{pid}/exe")
        executable_bytes = Path(f"/proc/{pid}/exe").read_bytes()
    except OSError as error:
        raise RuntimeError("cannot read the live gate process executable") from error
    second = pr_driver.linux_process_binding(pid)
    try:
        second_executable = os.readlink(f"/proc/{pid}/exe")
    except OSError as error:
        raise RuntimeError("cannot re-read the live gate process executable") from error
    if (
        first["start_time_ticks"] != second["start_time_ticks"]
        or executable != second_executable
        or executable.endswith(" (deleted)")
    ):
        raise RuntimeError("gate process identity changed during executable hashing")
    return {
        "pid": pid,
        "start_time_ticks": first["start_time_ticks"],
        "executable": executable,
        "executable_sha256": hashlib.sha256(executable_bytes).hexdigest(),
        "pid_namespace": first["pid_namespace"],
        "network_namespace": first["network_namespace"],
    }


def _candidate_file_hashes(root: Path) -> tuple[dict[str, str], dict[str, bytes]]:
    if root.is_symlink() or not root.is_dir() or root.resolve(strict=True) != root:
        raise RuntimeError("agent candidate directory is missing, linked, or non-canonical")
    files: dict[str, bytes] = {}
    for path in root.rglob("*"):
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode):
            raise RuntimeError("agent candidate contains a symbolic link")
        if stat.S_ISDIR(metadata.st_mode):
            continue
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            raise RuntimeError("agent candidate contains a non-regular or multiply-linked file")
        relative = path.relative_to(root).as_posix()
        if relative in files:
            raise RuntimeError("agent candidate contains a duplicate path")
        files[relative] = path.read_bytes()
    if set(files) != pr_driver.EXPECTED_FILES:
        raise RuntimeError("agent candidate file set differs from the protected fixture")
    return (
        {name: hashlib.sha256(content).hexdigest() for name, content in files.items()},
        files,
    )


def verify_candidate_while_host_alive(
    ready_path: Path,
    release_path: Path,
    *,
    runtime_dir: Path,
    state_dir: Path,
    execution_id: str,
    admission: dict[str, object],
    expected_mcp_host_sha256: str,
) -> dict[str, object]:
    """Run the credential-free acceptance verifier while the admitted MCP child waits at shutdown."""
    if (
        runtime_dir != ready_path.parent
        or runtime_dir != release_path.parent
        or runtime_dir.name != ".runtime"
        or runtime_dir.resolve(strict=True) != runtime_dir
        or stat.S_IMODE(runtime_dir.stat().st_mode) != 0o700
        or runtime_dir.stat().st_uid != os.geteuid()
        or ready_path.name != "mcp-gate-ready.json"
        or release_path.name != "mcp-gate-release.json"
        or release_path.exists() or release_path.is_symlink()
    ):
        raise RuntimeError("same-host gate barrier paths are not new private runtime files")
    ready, _ = _read_private_json(ready_path, maximum_bytes=4096)
    nonce = ready.get("nonce")
    host_pid = ready.get("host_pid")
    host_start_time = ready.get("host_start_time_ticks")
    supervisor_pid = ready.get("supervisor_pid")
    if (
        ready.get("schema_version") != 1
        or not _is_hex(nonce, 64)
        or not isinstance(host_pid, int) or isinstance(host_pid, bool) or host_pid <= 0
        or not isinstance(host_start_time, int) or isinstance(host_start_time, bool) or host_start_time <= 0
        or not isinstance(supervisor_pid, int) or isinstance(supervisor_pid, bool) or supervisor_pid <= 0
        or host_pid == supervisor_pid
    ):
        raise RuntimeError("same-host gate ready marker has invalid process binding fields")

    receipt_path = state_dir / "same-host-gate-receipt.json"
    receipt: dict[str, object] = {
        "schema_version": 1,
        "verdict": "fail",
        "gate": "credential_free_acceptance_while_mcp_host_alive",
        "observed_while_mcp_host_alive": False,
        "failure_code": "gate_precondition_or_verifier_failed",
        "admission_delivery_id": None,
        "base_commit": None,
        "requirement_sha256": None,
        "policy_sha256": pr_driver.POLICY_SHA,
        "candidate_source_sha256": None,
        "acceptance_sha256": None,
        "host": {
            "pid": host_pid,
            "start_time_ticks": host_start_time,
            "supervisor_pid": supervisor_pid,
            "executable": None,
            "executable_sha256": None,
        },
        "verifier": {
            "id": pr_driver.MERGE_GATE_VERIFIER_ID,
            "command": ["cargo", "test", "--offline", "--locked", "--test", "acceptance"],
            "environment": "read-only candidate, fresh target/tmp, Bubblewrap network/PID/user namespaces, no host home or Forgejo credential",
            "exit_code": None,
            "elapsed_ms": None,
            "output_bytes": None,
            "output_sha256": None,
            "test_summary": None,
        },
    }
    try:
        live_host = _stable_live_executable(host_pid)
        live_supervisor = pr_driver.linux_process_binding(supervisor_pid)
        expected_executable = state_dir / "build/host-artifacts/mission-012-mcp-host"
        expected_executable = expected_executable.resolve(strict=True)
        if (
            live_host["start_time_ticks"] != host_start_time
            or live_host["executable"] != str(expected_executable)
            or live_host["executable_sha256"] != expected_mcp_host_sha256
            or not _is_hex(expected_mcp_host_sha256, 64)
        ):
            raise RuntimeError("same-host gate process is not the admitted Rust MCP host binary")
        receipt["host"] = {
            **live_host,
            "supervisor_pid": live_supervisor["pid"],
            "supervisor_start_time_ticks": live_supervisor["start_time_ticks"],
        }

        mission = admission.get("mission")
        webhook = admission.get("webhook")
        event = webhook.get("event") if isinstance(webhook, dict) else None
        if not isinstance(mission, dict) or not isinstance(event, dict):
            raise RuntimeError("same-host gate has no verified webhook admission")
        base_commit = mission.get("protected_base_commit")
        requirement_sha = mission.get("requirement_sha256")
        delivery_id = event.get("delivery_id")
        if not _is_hex(base_commit, 40) or not _is_hex(requirement_sha, 64) or not isinstance(delivery_id, str):
            raise RuntimeError("same-host gate admission binding is malformed")
        receipt["base_commit"] = base_commit
        receipt["requirement_sha256"] = requirement_sha
        receipt["admission_delivery_id"] = delivery_id

        candidate_root = runtime_dir / execution_id / "workspace/candidate"
        base_root = runtime_dir / execution_id / "workspace/base"
        base_hashes, base_files = _candidate_file_hashes(base_root)
        candidate_hashes, candidate_files = _candidate_file_hashes(candidate_root)
        if (
            base_hashes["src/lib.rs"] != pr_driver.BROKEN_SOURCE_SHA
            or base_hashes["tests/acceptance.rs"] != pr_driver.EXPECTED_ACCEPTANCE_SHA
            or candidate_hashes["tests/acceptance.rs"] != pr_driver.EXPECTED_ACCEPTANCE_SHA
            or candidate_hashes["src/lib.rs"] != pr_driver.EXPECTED_FIXED_SOURCE_SHA
            or any(
                candidate_files[name] != base_files[name]
                for name in pr_driver.EXPECTED_FILES - {"src/lib.rs"}
            )
        ):
            raise RuntimeError("agent candidate is not the single-source-path protected repair")
        pr_driver.verify_pinned_sources(
            candidate_files["src/lib.rs"], candidate_files["tests/acceptance.rs"]
        )
        receipt["candidate_source_sha256"] = candidate_hashes["src/lib.rs"]
        receipt["acceptance_sha256"] = candidate_hashes["tests/acceptance.rs"]

        started = time.monotonic()
        exit_code, output = pr_driver.run_postmerge_cargo_test(
            candidate_root, pr_driver.RUST_TOOLCHAIN
        )
        elapsed_ms = int((time.monotonic() - started) * 1000)
        after_hashes, _ = _candidate_file_hashes(candidate_root)
        output_sha = hashlib.sha256(output).hexdigest()
        passed = (
            exit_code == 0
            and b"2 passed; 0 failed" in output
            and after_hashes == candidate_hashes
        )
        receipt["verifier"] = {
            **receipt["verifier"],
            "exit_code": exit_code,
            "elapsed_ms": elapsed_ms,
            "output_bytes": len(output),
            "output_sha256": output_sha,
            "test_summary": "2 passed; 0 failed" if passed else None,
        }
        live_after = _stable_live_executable(host_pid)
        if (
            live_after["start_time_ticks"] != host_start_time
            or live_after["executable_sha256"] != expected_mcp_host_sha256
        ):
            raise RuntimeError("MCP host exited or changed identity during independent verification")
        if passed:
            receipt["verdict"] = "pass"
            receipt["observed_while_mcp_host_alive"] = True
            receipt["failure_code"] = None
    except Exception:
        # The held MCP child must be released on both outcomes. A failed or
        # incomplete verifier still produces a bound failure receipt and can
        # never be interpreted as a pass by the composed runner.
        receipt["verdict"] = "fail"
        receipt["observed_while_mcp_host_alive"] = False

    payload = _write_private_json_exclusive(receipt_path, receipt)
    release = {
        "schema_version": 1,
        "nonce": nonce,
        "gate_operation_sha256": hashlib.sha256(payload).hexdigest(),
    }
    _write_private_json_exclusive(release_path, release)
    receipt["receipt_sha256"] = hashlib.sha256(payload).hexdigest()
    receipt["receipt_path"] = str(receipt_path)
    return receipt


def execute_stages(
    stages: list[tuple[str, list[str], int]],
    log_path: Path,
    budget_cap_usd: str,
    *,
    expected_allocation_id: str,
    stage_runner: StageRunner = run_stage_command,
    admission_evidence_dir: Path,
    agent_adapter: str = "claude",
    expect_stale_base_refusal: bool = False,
    expect_same_file_conflict: bool = False,
    expect_candidate_mutation_refusal: bool = False,
    expect_test_host_restart: bool = False,
    same_host_gate_barrier: bool = False,
    webhook_runtime_dir: Path | None = None,
    state_dir: Path | None = None,
    execution_id: str | None = None,
    source_root: Path | None = None,
    source_sha256: dict[str, str] | None = None,
) -> dict[str, dict[str, object]]:
    if tuple(name for name, _, _ in stages) != EXPECTED_STAGES:
        raise RuntimeError("pipeline stages must be admission, agent, then pull-request")
    if (source_root is None) != (source_sha256 is None):
        raise RuntimeError("source root and source hashes must be provided together")
    if same_host_gate_barrier and (
        agent_adapter != "local-test-adapter"
        or webhook_runtime_dir is None
        or state_dir is None
        or execution_id is None
        or expect_test_host_restart
        or expect_stale_base_refusal
        or expect_same_file_conflict
        or expect_candidate_mutation_refusal
    ):
        raise RuntimeError("same-host gate barrier is not configured for the isolated success-path adapter")
    fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    results: dict[str, dict[str, object]] = {}
    expected_mcp_host_sha256: str | None = None
    verified_webhook_handoff: dict[str, str] | None = None
    same_host_gate_result: dict[str, object] | None = None
    with os.fdopen(fd, "wb") as log:
        for stage_name, command, timeout in stages:
            if stage_name == "agent":
                if verified_webhook_handoff is None:
                    raise RuntimeError("agent stage has no verified webhook admission handoff")
                if any(
                    option in command
                    for option in ("--webhook-inbox-file", "--admission-source-file")
                ):
                    raise RuntimeError("agent stage supplied untrusted webhook handoff paths")
                command = [
                    *command,
                    "--webhook-inbox-file",
                    verified_webhook_handoff["webhook_inbox_file"],
                    "--admission-source-file",
                    verified_webhook_handoff["admission_source_file"],
                ]
            if source_root is not None and source_sha256 is not None:
                try:
                    verify_sources_unchanged(source_root, source_sha256)
                except SourceSnapshotError as error:
                    raise RuntimeError(f"pipeline source changed before {stage_name}: {error}") from None
            if stage_name == "agent" and same_host_gate_barrier:
                assert webhook_runtime_dir is not None
                assert state_dir is not None
                assert execution_id is not None
                if stage_runner not in (
                    run_stage_command,
                    run_stage_command_in_attested_parent_scope,
                ):
                    raise RuntimeError("same-host gate barrier requires a verified bounded process stage runner")
                ready_file = webhook_runtime_dir / "mcp-gate-ready.json"
                release_file = webhook_runtime_dir / "mcp-gate-release.json"
                if ready_file.exists() or ready_file.is_symlink() or release_file.exists() or release_file.is_symlink():
                    raise RuntimeError("same-host gate barrier files already exist before the agent stage")

                def run_gate_during_host_lifetime(path: Path) -> None:
                    nonlocal same_host_gate_result
                    same_host_gate_result = verify_candidate_while_host_alive(
                        path,
                        release_file,
                        runtime_dir=webhook_runtime_dir,
                        state_dir=state_dir,
                        execution_id=execution_id,
                        admission=results["admission"],
                        expected_mcp_host_sha256=expected_mcp_host_sha256 or "",
                    )

                return_code, output = stage_runner(
                    command,
                    timeout,
                    gate_barrier_ready_file=ready_file,
                    on_gate_barrier_ready=run_gate_during_host_lifetime,
                )
            else:
                return_code, output = stage_runner(command, timeout)
            if source_root is not None and source_sha256 is not None:
                try:
                    verify_sources_unchanged(source_root, source_sha256)
                except SourceSnapshotError as error:
                    raise RuntimeError(f"pipeline source changed during {stage_name}: {error}") from None
            if not isinstance(output, bytes) or len(output) > MAX_STAGE_OUTPUT_BYTES:
                stage_output_limit_mib = MAX_STAGE_OUTPUT_BYTES // (1024 * 1024)
                raise RuntimeError(
                    f"{stage_name} stage output exceeded the {stage_output_limit_mib} MiB bound"
                )
            log.write(json.dumps({"stage": stage_name, "exit_code": return_code}).encode() + b"\n")
            log.write(output)
            if output and not output.endswith(b"\n"):
                log.write(b"\n")
            log.flush()
            os.fsync(log.fileno())
            if return_code != 0:
                output_sha256 = hashlib.sha256(output).hexdigest()
                diagnostic = _safe_stage_failure_diagnostic(output)
                detail = f"; stage_output_sha256={output_sha256}"
                if diagnostic is not None:
                    detail += f"; {diagnostic}"
                raise RuntimeError(
                    f"{stage_name} stage failed with exit code {return_code}{detail}"
                )
            try:
                parsed = json.loads(output)
            except (json.JSONDecodeError, UnicodeDecodeError):
                raise RuntimeError(f"{stage_name} stage did not return a JSON result; private log: {log_path}") from None
            if not isinstance(parsed, dict):
                raise RuntimeError(f"{stage_name} stage result must be a JSON object")
            if stage_name == "admission":
                validate_admission_result(parsed, expected_allocation_id)
                if webhook_runtime_dir is None:
                    raise RuntimeError("agent stage requires the private webhook runtime directory")
                verified_webhook_handoff = validate_stage_admission_handoff(
                    parsed, webhook_runtime_dir
                )
                provenance = validate_admission_build_provenance(
                    admission_evidence_dir,
                    parsed,
                    expect_test_crash_host=expect_test_host_restart,
                )
                expected_mcp_host_sha256 = provenance["mcp_host_binary_sha256"]
            elif stage_name == "agent":
                validate_agent_result(
                    results["admission"], parsed, budget_cap_usd,
                    expected_allocation_id, expected_mcp_host_sha256, agent_adapter,
                    require_host_restart=expect_test_host_restart,
                    expected_webhook_handoff=verified_webhook_handoff,
                )
                if same_host_gate_barrier:
                    if same_host_gate_result is None or same_host_gate_result.get("verdict") != "pass":
                        raise RuntimeError("same-host gate verifier did not pass while the MCP child was alive")
                    if webhook_runtime_dir is None or state_dir is None or execution_id is None:
                        raise RuntimeError("same-host gate runtime paths disappeared during the agent stage")
                    receipt_path = state_dir / "same-host-gate-receipt.json"
                    on_disk_receipt, receipt_bytes = _read_private_json(
                        receipt_path, maximum_bytes=64 * 1024
                    )
                    expected_receipt = {
                        key: value for key, value in same_host_gate_result.items()
                        if key not in {"receipt_sha256", "receipt_path"}
                    }
                    if (
                        on_disk_receipt != expected_receipt
                        or hashlib.sha256(receipt_bytes).hexdigest()
                        != same_host_gate_result.get("receipt_sha256")
                        or same_host_gate_result.get("candidate_source_sha256")
                        != parsed.get("candidate_file_sha256", {}).get("src/lib.rs")
                        or same_host_gate_result.get("host", {}).get("executable_sha256")
                        != expected_mcp_host_sha256
                        or parsed.get("private_execution_state")
                        != str(webhook_runtime_dir / execution_id)
                    ):
                        raise RuntimeError("same-host gate receipt is not bound to the finalized MCP candidate")
            elif expect_candidate_mutation_refusal:
                validate_candidate_mutation_refusal(
                    results["admission"], results["agent"], parsed, budget_cap_usd,
                    expected_allocation_id, expected_mcp_host_sha256, agent_adapter,
                )
            elif expect_stale_base_refusal:
                validate_stale_base_refusal(
                    results["admission"], results["agent"], parsed, budget_cap_usd,
                    expected_allocation_id, expected_mcp_host_sha256, agent_adapter,
                    same_file_conflict=expect_same_file_conflict,
                )
            else:
                validate_joined_results(
                    results["admission"], results["agent"], parsed, budget_cap_usd,
                    expected_allocation_id, expected_mcp_host_sha256, agent_adapter,
                )
            results[stage_name] = parsed
    if verified_webhook_handoff is None:
        raise RuntimeError("pipeline completed without a verified webhook admission handoff")
    results["webhook_admission_handoff"] = {
        key: verified_webhook_handoff[key]
        for key in ("delivery_id", "admission_source_sha256", "webhook_inbox_sha256")
    }
    if same_host_gate_barrier:
        if same_host_gate_result is None:
            raise RuntimeError("pipeline completed without the same-child gate verification")
        results["same_host_gate_verification"] = same_host_gate_result
    return results


def _write_json_exclusive(path: Path, value: object) -> None:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, "wb") as output:
        output.write(payload)
        output.flush()
        os.fsync(output.fileno())


def write_composed_checksums(evidence_dir: Path) -> Path:
    manifest = evidence_dir / "SHA256SUMS"
    if (
        evidence_dir.is_symlink()
        or not evidence_dir.is_dir()
        or manifest.exists()
        or manifest.is_symlink()
    ):
        raise RuntimeError("composed evidence directory or checksum destination is unsafe")
    lines = []
    for path in sorted(evidence_dir.iterdir(), key=lambda item: item.name):
        if (
            path.is_symlink()
            or not path.is_file()
            or not re.fullmatch(r"[A-Za-z0-9._-]{1,128}", path.name)
        ):
            raise RuntimeError("composed evidence contains a symlink or non-file entry")
        lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}")
    if not lines:
        raise RuntimeError("composed evidence directory is empty")
    payload = ("\n".join(lines) + "\n").encode()
    fd = os.open(manifest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as output:
        output.write(payload)
        output.flush()
        os.fsync(output.fileno())
    return manifest


def _admission_summary(
    result: dict[str, object], expected_allocation_id: str,
    mcp_host_binary_sha256: str,
    webhook_handoff: dict[str, object],
) -> dict[str, object]:
    return {
        **validate_admission_result(result, expected_allocation_id),
        "mcp_host_binary_sha256": mcp_host_binary_sha256,
        "webhook_source_binding": webhook_handoff,
    }


def _agent_summary(result: dict[str, object]) -> dict[str, object]:
    return {
        key: result.get(key) for key in (
            "status", "agent_adapter", "model", "effort", "budget_cap_usd", "reported_cost_usd",
            "model_invoked_during_run",
            "input_tokens", "output_tokens", "duration_ms",
            "base_commit", "admission_sha256", "host_binary_sha256",
            "admission_source_mode", "admission_source_delivery_id",
            "admission_source_sha256", "webhook_inbox_sha256",
            "receipt_sha256", "receipt_count",
            "test_adapter_untrusted_work_item_observed",
            "tool_names_from_receipts", "changed_paths", "candidate_file_sha256",
            "execution_sandbox", "sandbox_checks", "host_observed_agent_process_tree",
            "restart_recovery",
        )
    }


def _pull_request_summary(result: dict[str, object]) -> dict[str, object]:
    return {
        key: result.get(key) for key in (
            "status", "pull_request", "base_commit", "requirement_sha256", "policy_sha256",
            "candidate_commit", "merge_commit", "candidate_sha256",
            "changed_paths", "verifier", "verifier_cpu_accounting", "reviewer", "pre_review_merge_refusal_http",
            "reviewer_merge_refusal_http", "issue_state", "evidence_dir",
            "merge_authority", "merge_gate_policy_sha256", "merge_gate_binary_sha256",
            "merge_gate_error_code", "merge_authorized", "merge_attempted",
            "main_head_after_advance", "competing_pull_request", "competing_pull_request_merged",
            "main_source_sha256_after_refusal", "same_file_conflict", "concurrent_base_advance",
            "candidate_mutation", "target_pull_request_head_sha_after_mutation",
            "target_candidate_source_sha256_after_mutation", "merge_gate_error_code",
            "merge_authorized", "merge_attempted", "webhook_reconciliation",
        )
    }


def run(args: argparse.Namespace) -> dict[str, object]:
    if getattr(args, "agent_adapter", None) == "claude":
        raise RuntimeError(REAL_PROVIDER_EXECUTION_REFUSAL)
    repo_root = ROOT
    state_dir = Path(args.state_dir)
    forgejo_binary = Path(args.forgejo_binary)
    evidence_dir = Path(args.evidence_dir)
    expected_host_network_namespace = args.expected_host_network_namespace
    current_network_namespace = os.readlink("/proc/self/ns/net")
    if (
        re.fullmatch(r"net:\[[1-9][0-9]*\]", expected_host_network_namespace) is None
        or current_network_namespace == expected_host_network_namespace
    ):
        raise RuntimeError("the composed merge supervisor must run outside the host network namespace")
    admission_evidence_dir = repo_root / "evidence" / f"{evidence_dir.name}-admission"
    if (
        not state_dir.is_absolute() or state_dir.parent != Path("/tmp")
        or state_dir.exists() or state_dir.is_symlink()
    ):
        raise RuntimeError("state directory must be a new direct child of /tmp")
    if forgejo_binary.is_symlink() or not forgejo_binary.is_file():
        raise RuntimeError("Forgejo binary must be a regular local file")
    if hashlib.sha256(forgejo_binary.read_bytes()).hexdigest() != FORGEJO_SHA256:
        raise RuntimeError("Forgejo binary SHA-256 differs from the identified v17 fixture")
    expected_evidence_parent = (repo_root / "evidence").resolve(strict=True)
    if (
        not evidence_dir.is_absolute()
        or evidence_dir.parent.resolve(strict=True) != expected_evidence_parent
        or evidence_dir.exists() or evidence_dir.is_symlink()
        or admission_evidence_dir.exists() or admission_evidence_dir.is_symlink()
    ):
        raise RuntimeError("evidence directory must be a new direct child of repository evidence/")
    log_path = state_dir.parent / f".{state_dir.name}.pipeline.log"
    if log_path.exists() or log_path.is_symlink():
        raise RuntimeError("private pipeline log destination already exists")
    source_snapshot_path = state_dir.parent / f".{state_dir.name}.composition-source-snapshot.zip"
    if source_snapshot_path.exists() or source_snapshot_path.is_symlink():
        raise RuntimeError("composition source snapshot destination already exists")
    try:
        source_snapshot = capture_sources(
            repo_root,
            source_snapshot_path,
            discover_source_paths(repo_root),
        )
    except SourceSnapshotError as error:
        raise RuntimeError(f"cannot capture composition source inputs: {error}") from None
    source_head = read_source_head(
        repo_root,
        git_dir=getattr(args, "source_git_dir", None),
        git_work_tree=getattr(args, "source_git_work_tree", None),
    )
    inject_concurrent_base_advance = bool(getattr(args, "inject_concurrent_base_advance", False))
    inject_concurrent_code_conflict = bool(getattr(args, "inject_concurrent_code_conflict", False))
    inject_candidate_mutation = bool(
        getattr(args, "inject_candidate_mutation_after_verification", False)
    )
    test_host_restart = bool(getattr(args, "test_host_restart", False))
    sandbox_test_adapter = bool(getattr(args, "sandbox_test_adapter", False))
    same_host_gate_barrier = bool(getattr(args, "same_host_gate_barrier", False))
    reuse_attested_parent_scope = bool(
        getattr(args, "reuse_attested_parent_scope", False)
    )
    if sandbox_test_adapter and args.agent_adapter != "local-test-adapter":
        raise RuntimeError("sandbox mode is only available for local-test-adapter")
    if same_host_gate_barrier and (
        args.agent_adapter != "local-test-adapter"
        or not sandbox_test_adapter
        or test_host_restart
        or inject_concurrent_base_advance
        or inject_concurrent_code_conflict
        or inject_candidate_mutation
    ):
        raise RuntimeError("same-host gate barrier requires the unmodified sandboxed local-test-adapter success path")
    if reuse_attested_parent_scope and (
        args.agent_adapter != "local-test-adapter"
        or not sandbox_test_adapter
    ):
        raise RuntimeError("parent-scope reuse requires the sandboxed local-test-adapter")
    if reuse_attested_parent_scope and current_attested_process_scope_snapshot() is None:
        raise RuntimeError("parent-scope stage execution requires a live attested outer cgroup")
    if sum((inject_concurrent_base_advance, inject_concurrent_code_conflict, inject_candidate_mutation)) > 1:
        raise RuntimeError("candidate and concurrent refusal fixture modes cannot be combined")
    inject_concurrent_change = inject_concurrent_base_advance or inject_concurrent_code_conflict
    stages = build_stage_commands(
        repo_root, state_dir, forgejo_binary, evidence_dir,
        args.execution_id, args.allocation_id, args.budget_cap_usd,
        agent_adapter=args.agent_adapter,
        inject_concurrent_base_advance=inject_concurrent_base_advance,
        inject_concurrent_code_conflict=inject_concurrent_code_conflict,
        inject_candidate_mutation_after_verification=inject_candidate_mutation,
        test_host_restart=test_host_restart,
        sandbox_test_adapter=sandbox_test_adapter,
        same_host_gate_barrier=same_host_gate_barrier,
        reuse_attested_parent_scope=reuse_attested_parent_scope,
        expected_host_network_namespace=expected_host_network_namespace,
        source_git_dir=getattr(args, "source_git_dir", None),
        source_git_work_tree=getattr(args, "source_git_work_tree", None),
    )
    stage_runner = (
        run_stage_command_in_attested_parent_scope
        if reuse_attested_parent_scope
        else run_stage_command
    )
    stage_results = execute_stages(
        stages,
        log_path,
        args.budget_cap_usd,
        expected_allocation_id=args.allocation_id,
        stage_runner=stage_runner,
        admission_evidence_dir=admission_evidence_dir,
        agent_adapter=args.agent_adapter,
        expect_stale_base_refusal=inject_concurrent_change,
        expect_same_file_conflict=inject_concurrent_code_conflict,
        expect_candidate_mutation_refusal=inject_candidate_mutation,
        expect_test_host_restart=test_host_restart,
        same_host_gate_barrier=same_host_gate_barrier,
        webhook_runtime_dir=state_dir / "project" / ".runtime",
        state_dir=state_dir,
        execution_id=args.execution_id,
        source_root=repo_root,
        source_sha256=source_snapshot["source_sha256"],
    )
    admission_provenance = validate_admission_build_provenance(
        admission_evidence_dir,
        stage_results["admission"],
        expect_test_crash_host=test_host_restart,
    )
    if inject_candidate_mutation:
        joined = validate_candidate_mutation_refusal(
            stage_results["admission"], stage_results["agent"], stage_results["pull-request"],
            args.budget_cap_usd, args.allocation_id,
            admission_provenance["mcp_host_binary_sha256"],
            agent_adapter=args.agent_adapter,
        )
    elif inject_concurrent_change:
        joined = validate_stale_base_refusal(
            stage_results["admission"], stage_results["agent"], stage_results["pull-request"],
            args.budget_cap_usd, args.allocation_id,
            admission_provenance["mcp_host_binary_sha256"],
            agent_adapter=args.agent_adapter,
            same_file_conflict=inject_concurrent_code_conflict,
        )
    else:
        joined = validate_joined_results(
            stage_results["admission"], stage_results["agent"], stage_results["pull-request"],
            args.budget_cap_usd, args.allocation_id,
            admission_provenance["mcp_host_binary_sha256"],
            agent_adapter=args.agent_adapter,
            require_host_restart=test_host_restart,
        )
    same_host_gate_summary: dict[str, object] | None = None
    if same_host_gate_barrier:
        gate = stage_results.get("same_host_gate_verification")
        pull_request = stage_results.get("pull-request")
        if (
            not isinstance(gate, dict)
            or gate.get("verdict") != "pass"
            or gate.get("observed_while_mcp_host_alive") is not True
            or not isinstance(pull_request, dict)
            or gate.get("candidate_source_sha256") != joined.get("candidate_sha256")
            or gate.get("base_commit") != joined.get("base_commit")
            or gate.get("requirement_sha256") != joined.get("requirement_sha256")
            or gate.get("policy_sha256") != pr_driver.POLICY_SHA
        ):
            raise RuntimeError("same-host verifier receipt is not bound to the independently merged candidate")
        same_host_gate_summary = {
            "schema_version": 1,
            "status": "passed_while_admitted_mcp_host_remained_alive",
            "gate_receipt_sha256": gate["receipt_sha256"],
            "base_commit": joined["base_commit"],
            "requirement_sha256": joined["requirement_sha256"],
            "policy_sha256": pr_driver.POLICY_SHA,
            "candidate_source_sha256": joined["candidate_sha256"],
            "candidate_commit": pull_request["candidate_commit"],
            "pull_request": pull_request["pull_request"],
            "merge_commit": pull_request["merge_commit"],
            "admission_delivery_id": gate["admission_delivery_id"],
            "mcp_host": gate["host"],
            "verifier": gate["verifier"],
            "observed_while_mcp_host_alive": True,
            "lifecycle_limit": (
                "The same admitted Rust MCP child remained alive during this independent candidate acceptance gate. "
                "The supervisor then released shutdown and the agent stage finalized before the separate PR/merge stage; "
                "this receipt does not claim the child remained alive through Forgejo merge."
            ),
        }
        _write_json_exclusive(
            evidence_dir / "same-host-gate-verification.json", same_host_gate_summary
        )
        joined["same_host_gate_verification"] = same_host_gate_summary
    readback_path = evidence_dir / "forgejo-result.json"
    if readback_path.is_symlink() or not readback_path.is_file():
        raise RuntimeError("PR stage omitted its Forgejo post-merge readback")
    readback = json.loads(readback_path.read_bytes())
    if inject_candidate_mutation:
        validate_candidate_mutation_readback(readback, joined)
    elif inject_concurrent_change:
        validate_stale_base_readback(
            readback, joined, same_file_conflict=inject_concurrent_code_conflict
        )
    else:
        validate_postmerge_readback(readback, joined)
    try:
        verify_sources_unchanged(repo_root, source_snapshot["source_sha256"])
        snapshot_copy_sha256 = copy_snapshot(
            source_snapshot_path,
            evidence_dir / "composition-source-snapshot.zip",
            source_snapshot["archive_sha256"],
        )
    except SourceSnapshotError as error:
        raise RuntimeError(f"composition source snapshot verification failed: {error}") from None
    joined["source_head"] = source_head
    joined["composition_source_sha256"] = source_snapshot["source_sha256"]
    joined["composition_source_snapshot"] = {
        "schema_version": source_snapshot["schema_version"],
        "archive": "composition-source-snapshot.zip",
        "archive_sha256": snapshot_copy_sha256,
        "manifest_sha256": source_snapshot["manifest_sha256"],
        "file_count": source_snapshot["file_count"],
        "total_bytes": source_snapshot["total_bytes"],
    }
    joined["state_dir"] = str(state_dir)
    joined["webhook_admission_handoff"] = stage_results["webhook_admission_handoff"]
    joined["admission_evidence_dir"] = str(admission_evidence_dir)
    joined["admission_evidence_manifest_sha256"] = admission_provenance["manifest_sha256"]
    joined["private_pipeline_log"] = str(log_path)
    joined["evidence_dir"] = str(evidence_dir)
    joined["readback_sha256"] = hashlib.sha256(readback_path.read_bytes()).hexdigest()
    _write_json_exclusive(evidence_dir / "composed-run.json", joined)
    _write_json_exclusive(evidence_dir / "composed-stage-summaries.json", {
        "admission": _admission_summary(
            stage_results["admission"], args.allocation_id,
            admission_provenance["mcp_host_binary_sha256"],
            stage_results["webhook_admission_handoff"],
        ),
        "agent": _agent_summary(stage_results["agent"]),
        "pull_request": _pull_request_summary(stage_results["pull-request"]),
        **({"same_host_gate_verification": same_host_gate_summary} if same_host_gate_summary is not None else {}),
        "forgejo_readback_sha256": joined["readback_sha256"],
    })
    write_composed_checksums(evidence_dir)
    return joined


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument("--execution-id", default="live-mcp-006")
    parser.add_argument("--allocation-id", default="mission012-webhook-live-001")
    parser.add_argument("--budget-cap-usd", default="0.20")
    parser.add_argument("--expected-host-network-namespace", required=True)
    parser.add_argument("--source-git-dir")
    parser.add_argument("--source-git-work-tree")
    parser.add_argument("--agent-adapter", choices=("claude", "local-test-adapter"), default="claude")
    parser.add_argument(
        "--sandbox-test-adapter",
        action="store_true",
        help="run only the explicit no-LLM adapter in Bubblewrap with MCP over stdio",
    )
    parser.add_argument(
        "--reuse-attested-parent-scope",
        action="store_true",
        help="run stages in a host-created outer cgroup and refuse any residual PID after each stage",
    )
    parser.add_argument(
        "--test-host-restart",
        action="store_true",
        help="add a debug-only SIGKILL/recovery stage for local-test-adapter before PR verification",
    )
    parser.add_argument(
        "--same-host-gate-barrier",
        action="store_true",
        help="hold the admitted sandboxed Rust MCP child at shutdown while an independent gate checks its candidate",
    )
    concurrent = parser.add_mutually_exclusive_group()
    concurrent.add_argument("--inject-concurrent-base-advance", action="store_true")
    concurrent.add_argument("--inject-concurrent-code-conflict", action="store_true")
    concurrent.add_argument("--inject-candidate-mutation-after-verification", action="store_true")
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
