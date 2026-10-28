#!/usr/bin/env python3
"""Check the mission-admission bindings in the saved live Forgejo result."""

import json
from pathlib import Path


RESULT = Path(__file__).resolve().parent / "live-run-003/result.json"
value = json.loads(RESULT.read_text(encoding="utf-8"))
webhook = value["webhook"]
recovery = webhook["receiver_overload_recovery"]
observation = webhook["automatic_retry_observation"]
replay = webhook["native_replay_binding"]
already_claimed = value["native_replay_admission"]
tasks = webhook["task_readback"]
native_tasks = webhook["native_replay_task_readback"]

checks = {
    "passed": value.get("verdict") == "passed",
    "forgejo_503": recovery.get("failed_status") == 503,
    "retry_after_five": recovery.get("retry_after_seconds") == 5,
    "failed_delivery_not_journaled": recovery.get("rejected_attempt_journaled") is False,
    "receiver_drained": recovery.get("queue_drained") is True,
    "replay_payload_unchanged": recovery.get("payload_unchanged") is True,
    "no_automatic_retry_observed": observation.get("automatic_retry_observed") is False
    and observation.get("observation_seconds") == 6.0
    and observation.get("task_count_before") == observation.get("task_count_after") == 1
    and observation.get("proxy_request_count_before")
    == observation.get("proxy_request_count_after")
    == 1,
    "failed_admitted_replay_and_second_replay_have_distinct_ids": len(
        {
            recovery.get("failed_delivery_id"),
            recovery.get("admitted_delivery_id"),
            replay.get("replay_delivery_id"),
        }
    )
    == 3,
    "first_accepted_delivery_is_sequence_one": recovery.get("receiver_sequence") == 1
    and webhook["event"].get("delivery_id") == recovery.get("admitted_delivery_id"),
    "second_accepted_delivery_is_sequence_two": replay.get("receiver_sequences") == [1, 2]
    and replay.get("original_delivery_id") == recovery.get("admitted_delivery_id"),
    "same_claim_reused": already_claimed.get("verdict") == "already_claimed"
    and already_claimed.get("delivery_id") == replay.get("replay_delivery_id")
    and already_claimed.get("mission_journal_unchanged") is True
    and already_claimed.get("admission_file_unchanged") is True,
    "task_statuses_show_one_failure_and_two_accepts": [
        (item.get("response_status"), item.get("is_succeed")) for item in native_tasks
    ]
    == [(503, False), (202, True), (202, True)],
    "no_runtime_executed": value["mission"].get("agent_runtime_invoked") is False,
    "cleanup_confirmed": value["process_cleanup"].get("tracked_processes_exited") is True
    and value["process_cleanup"].get("expected_tcp_listeners_absent") is True
    and value["process_cleanup"].get("hook_disabled_before_server_stop") is True,
}

for name, passed in checks.items():
    print(f"{name}: {'PASS' if passed else 'FAIL'}")
if not all(checks.values()):
    raise SystemExit(1)
print(f"checks_passed: {len(checks)}")
