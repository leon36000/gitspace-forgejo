#!/usr/bin/env python3
"""Cross-check selected CONTINUATION-210 result fields against the saved Forgejo access log."""
import json
from pathlib import Path

evidence = Path(__file__).resolve().parent
result = json.loads((evidence / "result.json").read_text())
attempt = json.loads((evidence / "git-receive-pack-attempt.json").read_text())
control = json.loads((evidence / "git-receive-pack-write-control.json").read_text())
cleanup = json.loads((evidence / "cleanup.json").read_text())
expected_events = json.loads((evidence / "receive-pack-access-events.json").read_text())
state = Path("/tmp/forgejo-team-review-cont210-state-001")
log = (state / "log/forgejo-server.log").read_text(errors="replace").splitlines()
repo = "/forge-team-org/team-review-readback.git"
actual_events = []
for line in log:
    if f"{repo}/git-receive-pack" in line:
        method = next(m for m in ("GET", "POST") if f"completed {m} " in line)
        status = next(s for s in ("403 Forbidden", "200 OK") if s in line)
        actual_events.append({"method": method, "route": f"{repo}/git-receive-pack", "status": status})
    elif f"{repo}/info/refs?service=git-receive-pack" in line:
        method = "GET"
        status = next(s for s in ("403 Forbidden", "200 OK") if s in line)
        actual_events.append({"method": method, "route": f"{repo}/info/refs?service=git-receive-pack", "status": status})
assert actual_events == expected_events, (actual_events, expected_events)
assert result["verdict"] == "passed"
assert result["read_only_git_receive_pack_probe"]["valid_push_post_http_status"] == 403
assert attempt["target_ref_absent_before"] and attempt["target_ref_absent_after"]
assert control["http_status"] == 200 and control["write_effect_confirmed"]
assert result["write_credential_receive_pack_control"]["target_oid"] == control["target_oid"]
assert control["target_branch"] == "agent/read-write-control-probe"
assert cleanup["server"]["returncode"] == cleanup["receiver"]["returncode"] == 0
assert cleanup["listeners_on_3130_after_stop"] == []
assert result["llm_or_agent_invoked"] is False
assert state.stat().st_mode & 0o777 == 0o700
summary = {
    "verdict": "consistent",
    "access_events_match_saved_server_log": True,
    "read_only_valid_post_denied": True,
    "read_only_target_absent_before_after": True,
    "write_enabled_control_accepted_and_read_back": True,
    "server_and_receiver_stopped": True,
    "state_dir_mode": "0700",
    "agent_or_llm_invoked": False,
}
(evidence / "run-validation.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, sort_keys=True))
