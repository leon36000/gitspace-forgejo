#!/usr/bin/env python3
"""Read-only cross-artifact audit for MISSION-012-COMPOSE-064 receipts."""

from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any


REPO = Path(__file__).resolve().parents[2]
COMPOSE = REPO / "evidence/mission-012-live-agent-webhook-local-adapter-064"
ADMISSION = REPO / "evidence/mission-012-live-agent-webhook-local-adapter-064-admission"


class BindingError(ValueError):
    pass


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise BindingError(f"{path.name}: expected JSON object")
    return value


def require(ok: bool, name: str) -> None:
    if not ok:
        raise BindingError(name)


def bundle() -> dict[str, Any]:
    stages = read_json(COMPOSE / "composed-stage-summaries.json")
    admission_stage = stages["admission"]
    return {
        "run": read_json(COMPOSE / "composed-run.json"),
        "stages": stages,
        "event": read_json(ADMISSION / "webhook-event.json"),
        "task": read_json(ADMISSION / "forgejo-task-binding.json"),
        "admission_result": read_json(ADMISSION / "admission.json"),
        "admission_stage": admission_stage,
        "forgejo": read_json(COMPOSE / "forgejo-result.json"),
        "verifier": read_json(COMPOSE / "independent-verifier.json"),
        "authority": read_json(COMPOSE / "rust-merge-authority-result.json"),
        "postmerge": read_json(COMPOSE / "postmerge-fresh-checkout.json"),
        "webhook": read_json(COMPOSE / "pr-webhook-reconciliation.json"),
    }


def validate(data: dict[str, Any]) -> list[str]:
    run = data["run"]
    stages = data["stages"]
    admission_stage = data["admission_stage"]
    admission = data["admission_result"]
    host = admission["host_admission"]
    issue_event = admission_stage["webhook"]["event"]
    task = data["task"]
    agent = stages["agent"]
    pr = stages["pull_request"]
    forgejo = data["forgejo"]
    verifier = data["verifier"]
    authority = data["authority"]
    webhook = data["webhook"]
    postmerge = data["postmerge"]

    delivery_id = data["event"].get("delivery_id")
    base = run.get("base_commit")
    requirement = run.get("requirement_sha256")
    mission_id = run.get("mission_id")
    admission_sha = run.get("admission_sha256")
    candidate = run.get("candidate_commit")
    candidate_sha = run.get("candidate_sha256")
    merge = run.get("merge_commit")

    require(run.get("status") == "merged_in_private_sandbox", "run_status")
    require(run.get("model") == "local-test-adapter" and run.get("model_invoked_during_run") is False, "adapter_identity")
    require(
        data["event"].get("event") == "issues"
        and data["event"].get("action") == "opened"
        and data["event"].get("payload_state") == "open"
        and data["event"].get("issue_number") == 1,
        "native_issue_event",
    )
    require(
        issue_event.get("delivery_id") == delivery_id
        and issue_event.get("issue_number") == data["event"].get("issue_number"),
        "admission_event_identity",
    )
    require(
        task.get("forgejo_delivery_uuid") == delivery_id
        and task.get("receiver_delivery_id") == delivery_id
        and task.get("response_status") == 202,
        "forgejo_delivery_ack",
    )
    require(
        admission.get("verdict") == "claimed"
        and admission.get("admission_mode") == "webhook_delivery"
        and admission.get("profile_id") == "mission-012-clamp"
        and host.get("issue_number") == data["event"].get("issue_number")
        and host.get("base_commit") == base
        and host.get("requirement_sha256") == requirement
        and host.get("allocation", {}).get("mission_id") == mission_id
        and host.get("allocation", {}).get("allocation_id") == run.get("allocation_id"),
        "versioned_mission_admission",
    )
    require(
        admission_stage.get("mission_id") == mission_id
        and admission_stage.get("requirement_sha256") == requirement
        and admission_stage.get("base_commit") == base
        and admission_stage.get("host_admission_sha256") == admission_sha
        and agent.get("admission_sha256") == admission_sha
        and agent.get("base_commit") == base,
        "admission_to_agent_binding",
    )

    agent_candidate_sha = agent.get("candidate_file_sha256", {}).get("src/lib.rs")
    require(
        isinstance(agent_candidate_sha, str)
        and agent_candidate_sha == candidate_sha
        and agent.get("model") == "local-test-adapter"
        and agent.get("model_invoked_during_run") is False,
        "agent_candidate_bytes",
    )
    require(
        pr.get("candidate_commit") == candidate
        and pr.get("candidate_sha256") == candidate_sha
        and pr.get("base_commit") == base
        and pr.get("requirement_sha256") == requirement
        and forgejo.get("candidate_commit") == candidate
        and forgejo.get("candidate_source_sha256") == candidate_sha,
        "candidate_to_pr_and_forgejo",
    )
    require(
        verifier.get("verdict") == "pass"
        and verifier.get("candidate_commit") == candidate
        and verifier.get("candidate_source_sha256") == candidate_sha
        and verifier.get("base_commit") == base
        and verifier.get("requirement_sha256") == requirement,
        "independent_verification_binding",
    )
    require(
        authority.get("verdict") == "merged"
        and authority.get("submitted_head_sha") == candidate
        and authority.get("validated_base_sha") == base
        and authority.get("merge_commit_sha") == merge
        and authority.get("main_head_sha") == merge
        and authority.get("external_writer_exclusion") == "not_provided_by_cooperative_lock",
        "controlled_merge_binding_and_limit",
    )
    require(
        pr.get("merge_commit") == merge
        and forgejo.get("merge_commit") == merge
        and forgejo.get("main_head_after_merge") == merge
        and webhook.get("status") == "verified_current_merged_pr"
        and webhook.get("successful_http_202_count") == 3
        and webhook.get("event_count") == 3,
        "forgejo_merge_readback",
    )
    canonical = webhook.get("canonical_state", {})
    require(
        canonical.get("merged") is True
        and canonical.get("head_sha") == candidate
        and canonical.get("base_sha") == merge
        and canonical.get("merge_commit_sha") == merge
        and canonical.get("state") == "closed",
        "closed_webhook_current_state",
    )
    receipts = webhook.get("reconciliation_receipts", [])
    require(len(receipts) == 3, "pr_webhook_receipt_count")
    for receipt in receipts:
        state = receipt.get("canonical_state", {})
        require(
            receipt.get("exit_code") == 0
            and receipt.get("verdict") == "current_state_read"
            and state.get("head_sha") == candidate
            and state.get("merge_commit_sha") == merge,
            "individual_pr_webhook_receipt",
        )

    merged = postmerge.get("merged", {})
    base_run = postmerge.get("base", {})
    require(
        postmerge.get("verdict") == "pass_retrospective"
        and postmerge.get("merge_commit") == merge
        and merged.get("head") == merge
        and merged.get("source_sha256") == candidate_sha
        and merged.get("exit_code") == 0
        and any("2 passed; 0 failed" in line for line in merged.get("summary", []))
        and postmerge.get("candidate_tree_matches_merge_tree") is True,
        "postmerge_behavior_on_exact_merge",
    )
    require(
        base_run.get("head") == base
        and base_run.get("exit_code") != 0
        and any("1 passed; 1 failed" in line for line in base_run.get("summary", [])),
        "base_failure_preserved",
    )
    return [
        "native_delivery_to_versioned_admission",
        "admission_to_same_candidate_source_bytes",
        "candidate_to_independent_verification_and_pr",
        "merge_to_native_pr_webhooks_and_canonical_readback",
        "exact_merge_checkout_behavior_and_base_counterexample",
        "local_test_adapter_and_writer_exclusion_limit_recorded",
    ]


def rejects_mutation(name: str, mutate: Any) -> str:
    altered = bundle()
    mutate(altered)
    try:
        validate(altered)
    except BindingError:
        return name
    raise BindingError(f"mutation_not_rejected:{name}")


def main() -> None:
    data = bundle()
    checks = validate(data)
    mutations = [
        rejects_mutation(
            "native_delivery_id_mismatch",
            lambda item: item["event"].__setitem__("delivery_id", "other-delivery"),
        ),
        rejects_mutation(
            "candidate_bytes_mismatch",
            lambda item: item["stages"]["agent"]["candidate_file_sha256"].__setitem__("src/lib.rs", "0" * 64),
        ),
        rejects_mutation(
            "merged_commit_mismatch",
            lambda item: item["webhook"]["canonical_state"].__setitem__("merge_commit_sha", "0" * 40),
        ),
        rejects_mutation(
            "postmerge_source_mismatch",
            lambda item: item["postmerge"]["merged"].__setitem__("source_sha256", "0" * 64),
        ),
    ]
    output = {
        "audit": "pass_read_only_saved_evidence",
        "composition": "MISSION-012-COMPOSE-064",
        "checks": checks,
        "mutation_refusals": mutations,
        "delivery_id": data["event"]["delivery_id"],
        "mission_id": data["run"]["mission_id"],
        "requirement_sha256": data["run"]["requirement_sha256"],
        "base_commit": data["run"]["base_commit"],
        "candidate_commit": data["run"]["candidate_commit"],
        "candidate_sha256": data["run"]["candidate_sha256"],
        "merge_commit": data["run"]["merge_commit"],
        "model_invoked": False,
        "n": 1,
    }
    print(json.dumps(output, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
