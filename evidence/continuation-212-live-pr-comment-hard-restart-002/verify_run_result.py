#!/usr/bin/env python3
"""Verify the bounded SIGKILL/restart and PR/comment reconciliation claims."""

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
result = json.loads((ROOT / "result.json").read_text(encoding="utf-8"))
restart = json.loads((ROOT / "receiver-restart.json").read_text(encoding="utf-8"))
cleanup = json.loads((ROOT / "cleanup.json").read_text(encoding="utf-8"))
build = json.loads((ROOT / "build-provenance.json").read_text(encoding="utf-8"))
tasks = json.loads((ROOT / "forgejo-hook-task-acks.json").read_text(encoding="utf-8"))
webhook = result.get("webhook")
pull = webhook.get("journal_record") if isinstance(webhook, dict) else None
comment = webhook.get("issue_comment_journal_record") if isinstance(webhook, dict) else None
batch = result.get("pr_comment_batch")
reconciliation = result.get("rust_reconciliation")

if (
    result.get("verdict") != "passed"
    or result.get("llm_or_agent_invoked") is not False
    or "17.0.0+gitea-1.22.0" not in result.get("forgejo", {}).get("version", "")
    or not isinstance(pull, dict)
    or pull.get("event") != "pull_request"
    or pull.get("action") != "review_requested"
    or pull.get("sequence") != 1
    or not isinstance(comment, dict)
    or comment.get("event") != "issue_comment"
    or comment.get("action") != "created"
    or comment.get("is_pull") is not True
    or comment.get("sequence") != 2
    or not isinstance(restart, dict)
    or restart.get("receiver_kill_signal") != "SIGKILL"
    or restart.get("receiver_kill_exit_code") != -9
    or restart.get("old_listener_absent_before_restart") is not True
    or restart.get("restarted_listener_owned") is not True
    or restart.get("both_hook_tasks_acked_202_before_kill") is not True
    or restart.get("hook_task_count") != 2
    or restart.get("event_types") != ["pull_request", "issue_comment"]
    or restart.get("forgejo_hook_task_event_types")
    != ["pull_request_review_request", "pull_request_comment"]
    or restart.get("delivery_ids") != [pull.get("delivery_id"), comment.get("delivery_id")]
    or restart.get("journal_byte_identical_after_restart") is not True
    or restart.get("journal_sha256_before") != restart.get("journal_sha256_after")
    or restart.get("record_count_after_restart") != 2
    or tasks.get("source") != "read-only Forgejo SQLite hook_task projection; raw payloads omitted"
    or [
        (row.get("event_type"), row.get("uuid"), row.get("response_status"), row.get("is_succeed"))
        for row in tasks.get("rows", [])
    ] != [
        ("pull_request_review_request", pull.get("delivery_id"), 202, True),
        ("pull_request_comment", comment.get("delivery_id"), 202, True),
    ]
    or not isinstance(batch, dict)
    or batch.get("delivery_ids") != restart.get("delivery_ids")
    or batch.get("upstream_get_count") != 2
    or batch.get("upstream_read_attempts") != 2
    or batch.get("shared_current_parent_confirmed") is not True
    or batch.get("comment_body_matches_current_api_readback") is not True
    or batch.get("comment_body_trust") != "untrusted_user_content"
    or batch.get("requested_reviewers_teams") != result.get("team_review_request", {}).get("readback")
    or not isinstance(reconciliation, dict)
    or reconciliation.get("exit_status") != 0
    or reconciliation.get("verdict") != "current_state_read"
    or reconciliation.get("inbox_unchanged_by_read") is not True
    or reconciliation.get("read_only_credential_scopes")
    != ["read:user", "read:repository", "read:issue"]
    or result.get("read_only_write_probe", {}).get("http_status") != 403
    or result.get("read_only_write_probe", {}).get("write_effect") != "denied"
    or cleanup.get("server", {}).get("stopped") is not True
    or cleanup.get("receiver", {}).get("stopped") is not True
    or cleanup.get("listeners_on_3130_after_stop") != []
    or cleanup.get("listeners_on_receiver_port_after_stop") != []
    or "--unshare-net" not in build.get("policy", "")
    or "--offline --locked" not in build.get("policy", "")
):
    raise SystemExit("CONTINUATION-212 result did not match its bounded live-run claims")

print(json.dumps({
    "verdict": "passed",
    "receiver_kill": restart["receiver_kill_signal"],
    "native_events": restart["event_types"],
    "hook_tasks_acked": restart["hook_task_count"],
    "hook_task_statuses": [row["response_status"] for row in tasks["rows"]],
    "journal_unchanged": restart["journal_byte_identical_after_restart"],
    "adapter_api_gets": batch["upstream_get_count"],
    "comment_trust": batch["comment_body_trust"],
    "llm_or_agent_invoked": False,
}, sort_keys=True))
