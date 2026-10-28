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

FORGEJO_SHA256 = "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7"
MAX_BUDGET_USD = Decimal("0.20")
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
) -> list[tuple[str, list[str], int]]:
    parse_budget(budget_cap_usd)
    if agent_adapter not in ("claude", "local-test-adapter"):
        raise RuntimeError("agent adapter must be claude or the explicit local-test-adapter")
    if not state_dir.is_absolute() or state_dir.parent != Path("/tmp"):
        raise RuntimeError("state directory must be a direct child of /tmp")
    if not evidence_dir.is_absolute() or evidence_dir.parent != repo_root / "evidence":
        raise RuntimeError("evidence directory must be a direct child of this repository's evidence folder")
    if not re.fullmatch(r"live-mcp-[0-9]{3}", execution_id):
        raise RuntimeError("execution ID must match the private live-mcp-NNN format")
    if not re.fullmatch(r"mission012-[A-Za-z0-9_-]{1,96}", allocation_id):
        raise RuntimeError("allocation ID must be a bounded host-owned mission012 identifier")
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
    if inject_concurrent_base_advance:
        pull_request_command.append("--inject-concurrent-base-advance")
    return [
        (
            "admission",
            [
                python_executable, str(admission_script),
                "--forgejo-binary", forgejo,
                "--repo-root", str(repo_root),
                "--state-dir", state,
                "--evidence-dir", str(admission_evidence_dir),
                "--allocation-id", allocation_id,
                "--build-mcp-host",
            ],
            480,
        ),
        (
            "agent",
            [
                python_executable, str(agent_script),
                "--state-dir", state,
                "--execution-id", execution_id,
                "--budget-cap-usd", budget_cap_usd,
                "--agent-adapter", agent_adapter,
            ],
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

    return {
        "schema_version": 1,
        "admission_mode": "webhook_delivery",
        "base_commit": mission["protected_base_commit"],
        "mission_id": mission_id,
        "requirement_sha256": mission["requirement_sha256"],
        "allocation_id": expected_allocation_id,
        "host_admission_sha256": mission["admission_file_sha256"],
        "webhook": {
            "event": event,
            "task_binding": binding,
            "active_after_run": False,
            "receiver_journal_records": 1,
            "invalid_host_credential": invalid_credential,
            "replay": replay,
        },
    }


def validate_agent_result(
    admission: dict[str, object],
    agent: dict[str, object],
    budget_cap_usd: str,
    expected_allocation_id: str,
    expected_mcp_host_sha256: str,
    agent_adapter: str = "claude",
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
    if (
        not _is_hex(expected_mcp_host_sha256, 64)
        or agent.get("host_binary_sha256") != expected_mcp_host_sha256
    ):
        raise RuntimeError("MCP host binary hash is not bound to the verified build provenance")
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
) -> dict[str, object]:
    admission = validate_admission_result(admission, expected_allocation_id)
    validate_agent_result(
        admission, agent, budget_cap_usd, expected_allocation_id,
        expected_mcp_host_sha256, agent_adapter,
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
        or value.get("schema_version") != 4
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


def validate_stale_base_refusal(
    admission: dict[str, object],
    agent: dict[str, object],
    pull_request: dict[str, object],
    budget_cap_usd: str,
    expected_allocation_id: str,
    expected_mcp_host_sha256: str,
    agent_adapter: str = "claude",
) -> dict[str, object]:
    admission = validate_admission_result(admission, expected_allocation_id)
    validate_agent_result(
        admission, agent, budget_cap_usd, expected_allocation_id,
        expected_mcp_host_sha256, agent_adapter,
    )
    concurrent = pull_request.get("concurrent_base_advance")
    if not isinstance(concurrent, dict):
        raise RuntimeError("stale-base refusal omitted its approved competing PR readback")
    error_code = pull_request.get("merge_gate_error_code")
    if (
        pull_request.get("schema_version") != 1
        or pull_request.get("status") != "refused_after_concurrent_base_advance_before_merge"
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
        or pull_request.get("main_source_sha256_after_refusal") != pr_driver.BROKEN_SOURCE_SHA
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
        or not _is_hex(pull_request.get("candidate_commit"), 40)
    ):
        raise RuntimeError("stale-base refusal is not bound to the verified candidate and competing merge")
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
    readback: dict[str, object], joined: dict[str, object]
) -> None:
    concurrent = readback.get("concurrent_base_advance")
    if (
        readback.get("status") != "refused_after_concurrent_base_advance_before_merge"
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
        or readback.get("main_source_sha256_after_refusal") != pr_driver.BROKEN_SOURCE_SHA
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
    ):
        raise RuntimeError("stale-base refusal readback does not prove that the target PR stayed unmerged")


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
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
        )),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TERM": "dumb",
        "NO_COLOR": "1",
    }


def run_stage_command(command: list[str], timeout: int) -> tuple[int, bytes]:
    process = subprocess.Popen(
        command,
        cwd=ROOT,
        env=stage_environment(),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    assert process.stdout is not None
    output = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout
    timed_out = False
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                timed_out = True
                _terminate_process_group(process)
                break
            events = selector.select(min(remaining, 0.25))
            for key, _ in events:
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                available = MAX_STAGE_OUTPUT_BYTES + 1 - len(output)
                output.extend(chunk[:available])
                if len(output) > MAX_STAGE_OUTPUT_BYTES:
                    _terminate_process_group(process)
                    return 125, bytes(output[:MAX_STAGE_OUTPUT_BYTES])
            if process.poll() is not None and not events and selector.get_map():
                # A descendant still holds the output pipe; keep the stage bounded by its deadline.
                continue
        if process.poll() is None:
            try:
                process.wait(timeout=max(0.0, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                timed_out = True
                _terminate_process_group(process)
    except KeyboardInterrupt:
        _terminate_process_group(process)
        raise
    finally:
        selector.close()
        process.stdout.close()
    if timed_out:
        return 124, bytes(output)
    return process.returncode, bytes(output)


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
    evidence_dir: Path, stage_result: dict[str, object]
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
    if (
        not isinstance(provenance, dict)
        or provenance.get("mcp_host_built") is not True
        or not _is_hex(mcp_host_sha256, 64)
    ):
        raise RuntimeError("admission provenance does not bind a built MCP host binary")
    return {
        "manifest_sha256": manifest_sha256,
        "mcp_host_binary_sha256": mcp_host_sha256,
    }


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
) -> dict[str, dict[str, object]]:
    if tuple(name for name, _, _ in stages) != EXPECTED_STAGES:
        raise RuntimeError("pipeline stages must be admission, agent, then pull-request")
    fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    results: dict[str, dict[str, object]] = {}
    expected_mcp_host_sha256: str | None = None
    with os.fdopen(fd, "wb") as log:
        for stage_name, command, timeout in stages:
            return_code, output = stage_runner(command, timeout)
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
                raise RuntimeError(f"{stage_name} stage failed with exit code {return_code}; private log: {log_path}")
            try:
                parsed = json.loads(output)
            except (json.JSONDecodeError, UnicodeDecodeError):
                raise RuntimeError(f"{stage_name} stage did not return a JSON result; private log: {log_path}") from None
            if not isinstance(parsed, dict):
                raise RuntimeError(f"{stage_name} stage result must be a JSON object")
            if stage_name == "admission":
                validate_admission_result(parsed, expected_allocation_id)
                provenance = validate_admission_build_provenance(admission_evidence_dir, parsed)
                expected_mcp_host_sha256 = provenance["mcp_host_binary_sha256"]
            elif stage_name == "agent":
                validate_agent_result(
                    results["admission"], parsed, budget_cap_usd,
                    expected_allocation_id, expected_mcp_host_sha256, agent_adapter,
                )
            elif expect_stale_base_refusal:
                validate_stale_base_refusal(
                    results["admission"], results["agent"], parsed, budget_cap_usd,
                    expected_allocation_id, expected_mcp_host_sha256, agent_adapter,
                )
            else:
                validate_joined_results(
                    results["admission"], results["agent"], parsed, budget_cap_usd,
                    expected_allocation_id, expected_mcp_host_sha256, agent_adapter,
                )
            results[stage_name] = parsed
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
) -> dict[str, object]:
    return {
        **validate_admission_result(result, expected_allocation_id),
        "mcp_host_binary_sha256": mcp_host_binary_sha256,
    }


def _agent_summary(result: dict[str, object]) -> dict[str, object]:
    return {
        key: result.get(key) for key in (
            "status", "agent_adapter", "model", "effort", "budget_cap_usd", "reported_cost_usd",
            "model_invoked_during_run",
            "input_tokens", "output_tokens", "duration_ms",
            "base_commit", "admission_sha256", "host_binary_sha256",
            "receipt_sha256", "receipt_count",
            "tool_names_from_receipts", "changed_paths", "candidate_file_sha256",
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
            "webhook_reconciliation",
        )
    }


def run(args: argparse.Namespace) -> dict[str, object]:
    repo_root = ROOT
    state_dir = Path(args.state_dir)
    forgejo_binary = Path(args.forgejo_binary)
    evidence_dir = Path(args.evidence_dir)
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
    stages = build_stage_commands(
        repo_root, state_dir, forgejo_binary, evidence_dir,
        args.execution_id, args.allocation_id, args.budget_cap_usd,
        agent_adapter=args.agent_adapter,
        inject_concurrent_base_advance=args.inject_concurrent_base_advance,
    )
    stage_results = execute_stages(
        stages,
        log_path,
        args.budget_cap_usd,
        expected_allocation_id=args.allocation_id,
        admission_evidence_dir=admission_evidence_dir,
        agent_adapter=args.agent_adapter,
        expect_stale_base_refusal=args.inject_concurrent_base_advance,
    )
    admission_provenance = validate_admission_build_provenance(
        admission_evidence_dir, stage_results["admission"]
    )
    if args.inject_concurrent_base_advance:
        joined = validate_stale_base_refusal(
            stage_results["admission"], stage_results["agent"], stage_results["pull-request"],
            args.budget_cap_usd, args.allocation_id,
            admission_provenance["mcp_host_binary_sha256"],
            agent_adapter=args.agent_adapter,
        )
    else:
        joined = validate_joined_results(
            stage_results["admission"], stage_results["agent"], stage_results["pull-request"],
            args.budget_cap_usd, args.allocation_id,
            admission_provenance["mcp_host_binary_sha256"],
            agent_adapter=args.agent_adapter,
        )
    readback_path = evidence_dir / "forgejo-result.json"
    if readback_path.is_symlink() or not readback_path.is_file():
        raise RuntimeError("PR stage omitted its Forgejo post-merge readback")
    readback = json.loads(readback_path.read_bytes())
    if args.inject_concurrent_base_advance:
        validate_stale_base_readback(readback, joined)
    else:
        validate_postmerge_readback(readback, joined)
    joined["source_head"] = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=repo_root, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5, check=True,
        env={"PATH": "/usr/bin:/bin", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"},
    ).stdout.decode().strip()
    joined["composition_source_sha256"] = {
        str(path.relative_to(repo_root)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in (
            Path(__file__).resolve(),
            SCRIPTS_DIR / "run_forgejo_live_webhook_mission_admission.py",
            SCRIPTS_DIR / "run_forgejo_live_webhook_reconcile_emitted.py",
            SCRIPTS_DIR / "run_forgejo_live_webhook_reconcile_e2e.py",
            SCRIPTS_DIR / "run_mission_012_live_mcp_agent_e2e.py",
            SCRIPTS_DIR / "mission_012_test_agent_adapter.py",
            SCRIPTS_DIR / "run_mission_012_live_agent_pr_e2e.py",
            repo_root / "src/agent_runtime.rs",
            repo_root / "src/mission_012_host.rs",
            repo_root / "src/bin/mission-012-mcp-host.rs",
            repo_root / "src/bin/forgejo-reconcile-local.rs",
            repo_root / "src/bin/forgejo-webhook-local.rs",
            repo_root / "src/webhook.rs",
            repo_root / "src/webhook/http.rs",
            repo_root / "src/webhook/reconcile.rs",
        )
    }
    joined["state_dir"] = str(state_dir)
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
        ),
        "agent": _agent_summary(stage_results["agent"]),
        "pull_request": _pull_request_summary(stage_results["pull-request"]),
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
    parser.add_argument("--agent-adapter", choices=("claude", "local-test-adapter"), default="claude")
    parser.add_argument("--inject-concurrent-base-advance", action="store_true")
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
