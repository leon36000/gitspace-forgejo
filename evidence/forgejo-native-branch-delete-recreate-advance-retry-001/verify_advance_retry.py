#!/usr/bin/env python3
"""Validate that old branch webhooks reconcile to a genuinely newer recreated head."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
RUN = ROOT / "live-run"


def load(name: str):
    return json.loads((RUN / name).read_text(encoding="utf-8"))


def verify_manifest() -> int:
    count = 0
    for line in (RUN / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        expected, name = line.split("  ", 1)
        target = (RUN / name).resolve(strict=True)
        if RUN.resolve() not in target.parents:
            raise ValueError(f"manifest path escapes run directory: {name}")
        if hashlib.sha256(target.read_bytes()).hexdigest() != expected:
            raise ValueError(f"run artifact digest mismatch: {name}")
        count += 1
    if count == 0:
        raise ValueError("run manifest is empty")
    return count


def verify() -> dict[str, object]:
    result = load("result.json")
    initial = load("reconciliation.json")
    retry = load("reconciliation-retry.json")
    create = load("create-event.json")
    delete = load("delete-event.json")
    branch = load("branch-retry-readback.json")
    tasks = load("hook-tasks.json")
    tasks_after = load("hook-retry-task-readback.json")
    ids = [create.get("delivery_id"), delete.get("delivery_id")]
    first_sha = result.get("branch", {}).get("current_commit_sha")
    retry_sha = result.get("branch", {}).get("retry_api_sha")

    if result.get("verdict") != "passed" or result.get("branch", {}).get("remote_ref_absent_after_delete") is not True:
        raise ValueError("initial native create/delete lifecycle was not confirmed")
    if not result.get("branch", {}).get("advanced_after_delete_before_retry"):
        raise ValueError("the scenario did not create a new commit after the 404")
    if not isinstance(first_sha, str) or not isinstance(retry_sha, str) or first_sha == retry_sha:
        raise ValueError("recreated branch did not have a different current SHA")
    if branch.get("status") != 200 or branch.get("commit_sha") != retry_sha or branch.get("new_commit_after_delete") is not True:
        raise ValueError("read-only API did not confirm the new branch head")

    initial_results = initial.get("results", [])
    if initial.get("verdict") != "inconclusive" or [item.get("delivery_id") for item in initial_results] != ids:
        raise ValueError("initial absent-ref batch was not inconclusive for the captured IDs")
    if any(item.get("verdict") != "inconclusive" or "canonical_state" in item for item in initial_results):
        raise ValueError("initial 404 was incorrectly projected as canonical state")
    if retry.get("delivery_ids") != ids or retry.get("delivery_ids_unchanged") is not True:
        raise ValueError("retry did not use the same delivery IDs in order")
    if retry.get("retry_exit_status") != 0 or retry.get("retry_result", {}).get("verdict") != "current_state_read":
        raise ValueError("retry did not return current-state success")
    retry_results = retry.get("retry_result", {}).get("results", [])
    if [item.get("delivery_id") for item in retry_results] != ids:
        raise ValueError("retry results do not preserve delivery order")
    if any(
        item.get("verdict") != "current_state_read"
        or item.get("canonical_state", {}).get("state") != "present"
        or item.get("canonical_state", {}).get("head_sha") != retry_sha
        for item in retry_results
    ):
        raise ValueError("old webhook deliveries were not bound to the genuinely newer head")
    if retry.get("webhook_journal_unchanged") is not True or retry.get("hook_tasks_unchanged") is not True:
        raise ValueError("retry modified the inbox or generated additional Forgejo hook tasks")
    if tasks != tasks_after or len(tasks) != 2 or [item.get("uuid") for item in tasks] != ids:
        raise ValueError("native create/delete hook task set changed during branch recreation")
    if any(item.get("response_status") != 202 or item.get("is_succeed") is not True for item in tasks):
        raise ValueError("captured native webhooks were not acknowledged with HTTP 202")

    cleanup = result.get("cleanup", {})
    if (
        cleanup.get("hook_disabled") is not True
        or cleanup.get("cleanup_failure") is not None
        or cleanup.get("forgejo", {}).get("listener_addresses_after_stop") != []
        or cleanup.get("receiver", {}).get("listener_addresses_after_stop") != []
    ):
        raise ValueError("local service cleanup was not confirmed")
    return {
        "captured_delivery_ids": ids,
        "initial_head_sha": first_sha,
        "retry_head_sha": retry_sha,
        "heads_differ": True,
        "initial_verdict": initial["verdict"],
        "retry_verdict": retry["retry_result"]["verdict"],
        "inbox_unchanged": True,
        "hook_tasks_unchanged": True,
        "run_manifest_files_checked": verify_manifest(),
    }


if __name__ == "__main__":
    print(json.dumps(verify(), sort_keys=True, indent=2))
