#!/usr/bin/env python3
"""Check the bounded claims in the CONTINUATION-211 live-run receipt."""

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
result = json.loads((ROOT / "result.json").read_text(encoding="utf-8"))
cleanup = json.loads((ROOT / "cleanup.json").read_text(encoding="utf-8"))
environment = json.loads((ROOT / "environment.json").read_text(encoding="utf-8"))
build = json.loads((ROOT / "build-provenance.json").read_text(encoding="utf-8"))

webhook = result.get("webhook")
pull = webhook.get("journal_record") if isinstance(webhook, dict) else None
comment = webhook.get("issue_comment_journal_record") if isinstance(webhook, dict) else None
batch = result.get("pr_comment_batch")
reconciliation = result.get("rust_reconciliation")
read_only_probe = result.get("read_only_write_probe")

if (
    result.get("verdict") != "passed"
    or result.get("forgejo", {}).get("version", "").find("17.0.0+gitea-1.22.0") < 0
    or result.get("llm_or_agent_invoked") is not False
    or result.get("repository", {}).get("private") is not True
    or result.get("pull_request", {}).get("state") != "open"
    or result.get("pull_request", {}).get("merged") is not False
    or not isinstance(pull, dict)
    or pull.get("sequence") != 1
    or pull.get("event") != "pull_request"
    or pull.get("action") != "review_requested"
    or not isinstance(comment, dict)
    or comment.get("sequence") != 2
    or comment.get("event") != "issue_comment"
    or comment.get("action") != "created"
    or comment.get("is_pull") is not True
    or comment.get("pull_request_number") != result.get("pull_request", {}).get("number")
    or not isinstance(batch, dict)
    or batch.get("event_order") != ["pull_request", "issue_comment"]
    or batch.get("delivery_ids") != [pull.get("delivery_id"), comment.get("delivery_id")]
    or batch.get("upstream_read_attempts") != 2
    or batch.get("upstream_get_count") != 2
    or batch.get("pull_request_get_confirmed") is not True
    or batch.get("comment_get_confirmed") is not True
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
    or not isinstance(read_only_probe, dict)
    or read_only_probe.get("http_status") != 403
    or read_only_probe.get("write_effect") != "denied"
    or cleanup.get("server", {}).get("stopped") is not True
    or cleanup.get("receiver", {}).get("stopped") is not True
    or cleanup.get("listeners_on_3130_after_stop") != []
    or cleanup.get("listeners_on_receiver_port_after_stop") != []
    or environment.get("state_dir_mode") != "0700"
    or "--unshare-net" not in build.get("policy", "")
    or "--offline --locked" not in build.get("policy", "")
):
    raise SystemExit("CONTINUATION-211 result did not match its bounded live-run claims")

print(json.dumps({
    "verdict": "passed",
    "native_event_order": [pull["event"], comment["event"]],
    "adapter_api_gets": batch["upstream_get_count"],
    "comment_trust": batch["comment_body_trust"],
    "read_only_comment_write": read_only_probe["http_status"],
    "llm_or_agent_invoked": False,
    "listeners_after_stop": 0,
}, sort_keys=True))
