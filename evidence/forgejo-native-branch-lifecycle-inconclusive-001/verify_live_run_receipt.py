from pathlib import Path
import hashlib
import json

root = Path(__file__).resolve().parent
live = root / "live-run"

def check(condition, message):
    if not condition:
        raise SystemExit(f"receipt check failed: {message}")

def read_json(name):
    return json.loads((live / name).read_text(encoding="utf-8"))

result = read_json("result.json")
create = read_json("create-event.json")
delete = read_json("delete-event.json")
tasks = read_json("hook-tasks.json")
batch = read_json("reconciliation.json")
attempts = read_json("reconciliation-attempts.json")
branch = read_json("branch-readback.json")
check(result["verdict"] == "passed", "runner-level expectation")
check("17.0.0" in result["forgejo"]["version"], "local Forgejo version")
check(create["event"] == "create" and create["sequence"] == 1, "native create event")
check(delete["event"] == "delete" and delete["sequence"] == 2, "native delete event")
check(create["branch"] == delete["branch"] == result["branch"]["name"], "same branch binding")
check(create["ref_type"] == delete["ref_type"] == "branch", "branch-only event type")
check(result["branch"]["remote_ref_absent_after_delete"] is True, "Git readback after delete")
check(result["branch"]["recreated_before_reconciliation"] is False, "no recreation before reconciliation")
check(branch["remote_ref_absent_after_delete"] is True, "branch observation artifact")
check(len(tasks) == 2 and [x["event_type"] for x in tasks] == ["create", "delete"], "two ordered native hook tasks")
check(all(x["is_delivered"] and x["is_succeed"] and x["response_status"] == 202 for x in tasks), "both webhook deliveries accepted")
check(batch["schema_version"] == 6 and batch["verdict"] == "inconclusive", "batch schema and verdict")
check(batch["upstream_read_attempts"] == 1, "CLI-reported batch cache count")
check(len(batch["results"]) == 2 and len(attempts) == 2, "two result rows")
ids = [create["delivery_id"], delete["delivery_id"]]
check([x["delivery_id"] for x in batch["results"]] == ids, "result delivery order")
check([x["event"] for x in attempts] == ["create", "delete"], "attempt event order")
for item in batch["results"]:
    check(item["verdict"] == "inconclusive", "item remains inconclusive")
    error = item["error"]
    check(error["code"] == "resource_not_found" and error["retryable"] is False, "ambiguous 404 is non-retryable refusal")
    check("HTTP 404 does not distinguish an absent branch" in error["action"], "404 ambiguity is actionable")
    check("canonical_state" not in item, "404 does not become canonical absence")
check(result["reconciliation"]["exit_status"] == 1, "CLI signals inconclusive batch")
check(result["reconciliation"]["journal_unchanged"] is True, "append-only inbox preserved")
check(result["reconciliation"]["credential_recorded"] is False, "credential not recorded")
check(result["cleanup"]["hook_disabled"] is True, "hook deactivation readback")
check(result["cleanup"]["forgejo"]["stop"]["stopped"] is True, "Forgejo process stopped")
check(result["cleanup"]["receiver"]["stop"]["stopped"] is True, "receiver process stopped")
check(not result["cleanup"]["forgejo"]["listener_addresses_after_stop"], "Forgejo listener stopped")
check(not result["cleanup"]["receiver"]["listener_addresses_after_stop"], "receiver listener stopped")

checksums = {}
for line in (live / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
    digest, name = line.split(maxsplit=1)
    checksums[name] = digest
for name, digest in checksums.items():
    actual = hashlib.sha256((live / name).read_bytes()).hexdigest()
    check(actual == digest, f"nested manifest {name}")
check(len(checksums) == 8, "expected raw evidence payload count")

correction = json.loads((root / "result-correction.json").read_text(encoding="utf-8"))
check(correction["raw_result_preserved"] is True, "raw result retained")
check(correction["runner_source_hash_at_run"] == result["provenance"]["source_sha256"]["scripts/run_forgejo_native_branch_delete_webhook_reconcile.py"], "run-time source hash")
check(correction["future_runner_source_hash"] == "923de0a0881557ac354671debbfd49b2c506d3067f53eeb9d561247cdbd0f673", "current wording correction source hash")
check("no live service was contacted" in result["limitations"][0], "original inaccurate string is transparently retained")

print("native Forgejo create/delete batch receipt: all invariants verified")
print(f"raw nested manifest: {len(checksums)}/{len(checksums)} artifacts verified")
print("result wording discrepancy: recorded and corrected without rewriting the raw run")
