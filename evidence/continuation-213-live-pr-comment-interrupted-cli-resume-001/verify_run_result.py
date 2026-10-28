#!/usr/bin/env python3
"""Independent structural check for the CONT-213 live Forgejo receipt."""

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
LIVE = ROOT / "live-e2e-001"


def read(name):
    return json.loads((LIVE / name).read_text(encoding="utf-8"))


def require(condition, message):
    if not condition:
        raise SystemExit(message)


result = read("result.json")
proxy = read("api-proxy-requests.json")
cleanup = read("cleanup.json")
environment = read("environment.json")
interrupted = read("interrupted-cli-resume.json")
access_projection = json.loads(
    (ROOT / "forgejo-access-log-projection.json").read_text(encoding="utf-8")
)

require(result.get("verdict") == "passed", "live runner did not pass")
require(result.get("llm_or_agent_invoked") is False, "receipt must not claim an LLM/agent call")
require(result.get("forgejo", {}).get("version", "").startswith("forgejo version 17.0.0+gitea-1.22.0"), "unexpected Forgejo build")
require(result.get("forgejo", {}).get("binary_sha256") == "e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7", "Forgejo binary digest changed")
require(result.get("forgejo", {}).get("commit") == "unknown", "build commit was not established")

require(environment.get("state_dir_mode") == "0700", "Forgejo fixture state was not private")
require(environment.get("forgejo_port") == 3131 and environment.get("local_api_proxy_port") == 3130, "unexpected local API topology")
require(proxy.get("listen") == "127.0.0.1:3130" and proxy.get("fixed_upstream") == "127.0.0.1:3131", "proxy was not fixed to loopback Forgejo")
records = proxy.get("records")
require(isinstance(records, list) and len(records) == result.get("api_proxy", {}).get("record_count"), "API proxy record count differs")
require(
    access_projection.get("forgejo_rows") == access_projection.get("proxy_forwarded_rows"),
    "Forgejo access log and fixed-loopback proxy request projections differ",
)
require(
    access_projection.get("forgejo_parent_get_count") == 5
    and access_projection.get("proxy_parent_get_count") == 5
    and access_projection.get("forgejo_comment_get_count") == 1
    and access_projection.get("proxy_comment_get_count") == 1,
    "Forgejo access log and proxy GET counts differ",
)

pull = f"/api/v1/repos/forge-team-org/team-review-readback/pulls/{result['pull_request']['number']}"
comment = f"/api/v1/repos/forge-team-org/team-review-readback/issues/comments/{result['pr_comment_batch']['comment_id']}"
gated = [record for record in records if record.get("gate") == "allowlisted"]
require(len(gated) == 5, "expected one denied POST, two interrupted requests, and two resumed requests")
expected = [
    ("POST", comment, 403, False, "not_allowlisted"),
    ("GET", pull, 200, True, "none"),
    ("GET", comment, 503, False, "cancelled_before_upstream"),
    ("GET", pull, 200, True, "none"),
    ("GET", comment, 200, True, "none"),
]
for record, (method, path, status, forwarded, barrier) in zip(gated, expected, strict=True):
    require(
        (record.get("method"), record.get("path"), record.get("status"), record.get("forwarded"), record.get("barrier"))
        == (method, path, status, forwarded, barrier),
        "API gate request sequence/status differs from the expected interruption and resume",
    )
    require(record.get("reader_credential_match") is True, "gate request did not carry the read-only credential")

require(interrupted.get("parent_get_completed_before_interruption") is True, "parent read was not confirmed before interruption")
require(interrupted.get("comment_get_held_then_cancelled_before_upstream") is True, "comment read was not held before upstream")
require(interrupted.get("forbidden_post_reached_forgejo") is False, "unlisted POST reached Forgejo")
require(interrupted.get("journal_unchanged_after_kill") is True, "journal changed during interrupted attempt")
require(interrupted.get("journal_sha256_before") == interrupted.get("journal_sha256_after_kill"), "journal hashes differ")
require(interrupted.get("barrier", {}).get("forward") is False, "comment barrier forwarded the interrupted request")
process = interrupted.get("killed_process", {})
require(process.get("signal") == 9 and process.get("returncode") == -9 and process.get("group_gone") is True, "CLI process group did not stop by SIGKILL")
require(process.get("credential_exposed") is False, "CLI output exposed the read-only credential")
require(interrupted.get("resume", {}).get("resumed_cli_completed_same_two_delivery_batch") is True, "same journal batch did not resume")
require(interrupted.get("resume", {}).get("verdict") == "current_state_read", "resumed CLI did not return a current read")
require(result.get("pr_comment_batch", {}).get("upstream_get_count") == 2, "resumed Forgejo read count differs")
require(result.get("pr_comment_batch", {}).get("comment_body_trust") == "untrusted_user_content", "comment trust label was lost")
require(result.get("rust_reconciliation", {}).get("inbox_unchanged_by_read") is True, "final reconciliation changed the journal")
require(result.get("pull_request", {}).get("merged") is False, "test fixture unexpectedly merged its PR")

require(cleanup.get("server", {}).get("stopped") is True, "Forgejo process cleanup failed")
require(cleanup.get("receiver", {}).get("stopped") is True, "receiver process cleanup failed")
require(cleanup.get("api_proxy", {}).get("stopped") is True, "API proxy cleanup failed")
for key in ("listeners_on_3130_after_stop", "forgejo_listeners_on_3131_after_stop", "listeners_on_receiver_port_after_stop"):
    require(cleanup.get(key) == [], f"listener remained after cleanup: {key}")

print(json.dumps({
    "verdict": "passed",
    "api_proxy_gate_requests": len(gated),
    "forgejo_resume_gets": result["pr_comment_batch"]["upstream_get_count"],
    "ordered_access_log_proxy_projection_matches": True,
    "journal_sha256": interrupted["journal_sha256_before"],
    "cli_sigkill_group_gone": True,
    "same_journal_resume": True,
    "cleanup_listeners_absent": True,
    "llm_or_agent_invoked": False,
}, sort_keys=True))
