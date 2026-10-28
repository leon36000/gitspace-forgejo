# CONTINUATION-227 — workflow action refusals with a useful field path

Date: 2026-10-27. Scope is the existing local Rust receiver and its tests. Base checkout is the dirty worktree at `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; this is not a clean-commit run.

## Finding and change

The published v3 workflow action contract already rejected unknown actions for both `workflow_run` and `workflow_job`, before append. The rejection was classified as `UnsupportedEvent`, so the HTTP response had no `details.field_path` even though the actionable field was known. The validation branches in `src/webhook/reconcile.rs` now return `InvalidEventPayloadField("action")` for an action that is present but absent from the published contract. The existing HTTP error mapping then identifies `payload.action`; supported event admission and unknown-action fail-closed behavior remain intact.

The new process test `actual_receiver_refuses_unknown_workflow_actions_without_journaling` launches the real `forgejo-webhook-local` binary. It sends signed, status-consistent `workflow_run_success` and `workflow_job_success` deliveries with a future action value. For both, the test checks HTTP 400, `accepted=false`, `event_payload_invalid`, `retryable=false`, `details.field_path=payload.action`, guidance referring to the event contract, and byte-for-byte unchanged journal state. A replay after shutdown is empty.

## Test-first observations

The first run exposed a fixture assumption: the receiver does not create the journal file at startup. The test was changed to compare optional file bytes, then the intended assertion failed with a missing (`null`) `details.field_path`. After the production mapping change, the previous unit assertion that expected the generic `UnsupportedEvent` classification also failed; it was updated to expect the precise action-field error. These are two harness/expectation adjustments followed by the observed product gap and its correction.

## Fresh verification

- Workflow action/status contract unit: 1 passed.
- Valid workflow run/job receiver handler contracts: 2 passed.
- Real receiver process refuses unknown run/job actions without journaling: 1 passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- Cargo license-state guard: passed; package `agent-forge-core`, `license=null`, `license_file=null`, `publish=[]`, no license selected, redistribution rights not assessed, SPDX `NOASSERTION`.

Command outputs are in the adjacent `*.log` files; exact invocations and tool versions are in `commands.txt` and `tool-versions.txt`. Source hashes are in `source-sha256.txt`.

## Coverage and limits

This demonstrates local process-level receiver admission for two workflow resource kinds and confirms that invalid future actions cannot enter this journal. It does not exercise Forgejo itself, the reconciliation CLI for a persisted future action, the merge gate, all event types, a live agent/LLM/AgentOS, or comparative project outcomes. It is not a security certification. The existing local workflow process proofs from CONT-226 are not repeated here. The overall mission remains incomplete.
