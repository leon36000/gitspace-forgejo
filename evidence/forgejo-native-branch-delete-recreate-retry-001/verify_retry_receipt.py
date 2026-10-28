#!/usr/bin/env python3
"""Independently validate the disposable local Forgejo retry receipt."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
FAILED = ROOT / "live-run"
PASSED = ROOT / "live-run-002"


def read_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def verify_manifest(directory: Path) -> int:
    manifest = directory / "SHA256SUMS"
    checked = 0
    for line in manifest.read_text(encoding="utf-8").splitlines():
        digest, name = line.split("  ", 1)
        path = (directory / name).resolve(strict=True)
        if directory.resolve() not in path.parents:
            raise ValueError(f"manifest path escapes evidence directory: {name}")
        observed = hashlib.sha256(path.read_bytes()).hexdigest()
        if observed != digest:
            raise ValueError(f"evidence digest mismatch: {name}")
        checked += 1
    if checked == 0:
        raise ValueError("evidence manifest is empty")
    return checked


def verify() -> dict[str, object]:
    failed_result = read_json(FAILED / "result.json")
    if (
        failed_result.get("verdict") != "failed"
        or failed_result.get("stage") != "retry_same_webhook_deliveries_after_branch_recreation"
        or failed_result.get("failure_type") != "NameError"
        or failed_result.get("cleanup", {}).get("hook_disabled") is not True
        or failed_result.get("cleanup", {}).get("forgejo", {}).get("listener_addresses_after_stop") != []
        or failed_result.get("cleanup", {}).get("receiver", {}).get("listener_addresses_after_stop") != []
    ):
        raise ValueError("first harness-failure receipt or cleanup differs")

    result = read_json(PASSED / "result.json")
    initial = read_json(PASSED / "reconciliation.json")
    retry = read_json(PASSED / "reconciliation-retry.json")
    create_event = read_json(PASSED / "create-event.json")
    delete_event = read_json(PASSED / "delete-event.json")
    hook_tasks = read_json(PASSED / "hook-tasks.json")
    retry_hook_tasks = read_json(PASSED / "hook-retry-task-readback.json")
    retry_attempts = read_json(PASSED / "reconciliation-retry-attempts.json")
    branch_readback = read_json(PASSED / "branch-retry-readback.json")
    delivery_ids = [create_event["delivery_id"], delete_event["delivery_id"]]

    if result.get("verdict") != "passed" or result.get("branch", {}).get("remote_ref_absent_after_delete") is not True:
        raise ValueError("native create/delete event lifecycle was not confirmed")
    if initial.get("verdict") != "inconclusive" or initial.get("upstream_read_attempts") != 1:
        raise ValueError("initial deleted-branch read was not inconclusive")
    if [item.get("delivery_id") for item in initial.get("results", [])] != delivery_ids:
        raise ValueError("initial batch does not match captured create/delete IDs")
    if any(item.get("verdict") != "inconclusive" or "canonical_state" in item for item in initial["results"]):
        raise ValueError("initial batch improperly published a canonical state")
    if retry.get("initial_exit_status") != 1 or retry.get("initial_verdict") != "inconclusive":
        raise ValueError("retry receipt does not retain the initial 404 refusal")
    if retry.get("delivery_ids") != delivery_ids or retry.get("delivery_ids_unchanged") is not True:
        raise ValueError("retry changed or reordered captured delivery IDs")
    if retry.get("webhook_journal_unchanged") is not True:
        raise ValueError("retry modified the append-only webhook journal")
    if retry.get("hook_tasks_unchanged") is not True or retry_hook_tasks != hook_tasks:
        raise ValueError("branch recreation added or changed a native webhook task")
    if len(hook_tasks) != 2 or [task.get("uuid") for task in hook_tasks] != delivery_ids:
        raise ValueError("expected exactly the original create and delete hook tasks")
    if any(task.get("response_status") != 202 or task.get("is_succeed") is not True for task in hook_tasks):
        raise ValueError("Forgejo did not deliver each original webhook as HTTP 202")

    current_head = result.get("branch", {}).get("retry_api_sha")
    if branch_readback.get("status") != 200 or branch_readback.get("commit_sha") != current_head:
        raise ValueError("recreated branch API readback does not match reported head")
    batch = retry.get("retry_result", {})
    if (
        retry.get("retry_exit_status") != 0
        or batch.get("verdict") != "current_state_read"
        or batch.get("upstream_read_attempts") != 1
        or [item.get("delivery_id") for item in batch.get("results", [])] != delivery_ids
        or [item.get("result", {}).get("delivery_id") for item in retry_attempts] != delivery_ids
    ):
        raise ValueError("retry batch did not read both original delivery IDs")
    if any(
        item.get("verdict") != "current_state_read"
        or item.get("canonical_state", {}).get("state") != "present"
        or item.get("canonical_state", {}).get("head_sha") != current_head
        for item in batch.get("results", [])
    ):
        raise ValueError("retry did not resolve both events to the current branch head")

    cleanup = result.get("cleanup", {})
    if (
        cleanup.get("cleanup_failure") is not None
        or cleanup.get("hook_disabled") is not True
        or cleanup.get("forgejo", {}).get("listener_addresses_after_stop") != []
        or cleanup.get("receiver", {}).get("listener_addresses_after_stop") != []
    ):
        raise ValueError("disposable local services were not confirmed stopped")
    return {
        "failed_harness_run_cleanup_verified": True,
        "passed_run_delivery_ids": delivery_ids,
        "initial_verdict": initial["verdict"],
        "retry_verdict": batch["verdict"],
        "hook_tasks_unchanged": True,
        "webhook_journal_unchanged": True,
        "local_evidence_manifest_files": verify_manifest(ROOT),
        "passed_run_manifest_files": verify_manifest(PASSED),
        "failed_run_manifest_files": verify_manifest(FAILED),
    }


if __name__ == "__main__":
    print(json.dumps(verify(), sort_keys=True, indent=2))
