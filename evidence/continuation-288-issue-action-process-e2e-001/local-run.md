# CONTINUATION-288 — issue action correction through real local processes

Date: 2026-10-27. Worktree base HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; existing worktree is heavily modified. This lot changes process-test coverage and helpers, its isolated-runner allowlist, the checkpoint, and the refusal-evidence matrix. No runtime source or Cargo metadata changed in this lot.

## Question and result

Does the action correction already verified at library level survive the actual receiver process, durable inbox, and actual reconciler CLI boundary?

The targeted test `corrected_issue_action_retries_same_delivery_then_reconciles_through_real_processes` passes **1/1**, run only by the isolated process runner. The runner compiles the test and application binaries offline and executes them under Bubblewrap 0.9.0 with separate network and PID namespaces.

The test sends a real HMAC-signed `issues` HTTP request to the actual `forgejo-webhook-local` binary. The missing-action delivery is refused with HTTP 400, `event_payload_invalid`, `retryable=false`, and a repair action that names `GET /forgejo/capabilities`, redelivery with the same ID, and the fact that the rejected body was not journaled. Replaying the journal shows no reserved event. The corrected `issues/opened` request reuses the same delivery ID and is accepted as sequence 1. Repeating the corrected request returns `duplicate=true`, still sequence 1; the journal contains exactly one row.

The actual `forgejo-reconcile-local` CLI reads that journal against a local HTTP fixture at the fixed test profile. The event payload says the issue is `open` and contains stale content, while the API fixture says `closed` and returns different current content. The CLI returns `current_state_read`, projects the current closed state and fixture content, performs exactly one issue GET, and leaves the inbox bytes unchanged.

## Related PR helper regression and diagnosis

Because the issue test generalized the signed-webhook test helper, one affected PR bridge test was rerun. Its first run timed out waiting for the merge fixture and recorded zero fixture requests. A diagnostic rerun collected the real CLI's structured refusal: `supervisor_capability_missing` at `writer_admission`, before network access. The older test launcher had not been updated when the current local merge profile began requiring `--supervisor-capability-fd`.

The test-only launcher now creates an inherited AF_UNIX `SOCK_SEQPACKET`, binds a synthetic envelope to the actual gate PID/start time/namespaces and the test supervisor, and holds the peer open for the child lifetime. It does not claim that this fixture policy is an independently verified production supervisor. After the test-harness correction, `signed_webhook_reconciliation_preserves_the_merge_cli_readback_sha` passes 1/1. The final runner invocation executes that regression test and the issue correction test, both 1/1. The original timeout, diagnostic refusal, and corrected pass are all retained in separate logs.

## Verification

- Final isolated process run: issue correction 1 passed and PR helper regression 1 passed, each with 8 filtered out; log: `process-run.log`.
- `cargo test --locked --offline --all-targets`: 37 suites, 694 passed, 0 failed, 152 ignored; log: `cargo-all-targets.log`. Ignored tests are not covered by this command.
- `cargo fmt --all -- --check`: pass; log: `cargo-fmt.log`.
- `rustfmt --edition 2024 --check tests/forgejo_merge_post_crash_recovery_process_e2e.rs`: pass after formatting the new helper invocation. The first check found only a line-wrap difference; no behavioral test was run from that state.
- `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py`: pass; log: `python-compile.log`.
- `git diff --check`: pass; log: `git-diff-check.log`.
- Runner versions and staged source-tree digest are printed in `process-run.log`; `source-sha256.txt` covers the relevant source and docs.

This is test coverage and test-harness maintenance around behavior already implemented in CONT-287. The issue test passed against that existing runtime, so this lot made no runtime fix and has no red functional run for the issue action itself. The PR diagnostic uncovered an outdated test fixture, not a runtime failure; its initial timeout and machine-readable `supervisor_capability_missing` refusal remain recorded. The earlier library-level red/green record remains in CONT-287.

## Limits

The issue and PR tests use the actual Rust receiver and reconciler processes, but their Forgejo API is a synthetic HTTP fixture bound to loopback; no Forgejo server is started. The PR test uses a synthetic capability bound to its child process; it does not prove a production supervisor boundary or policy. These tests exercise only the targeted paths on Linux/aarch64 and the Bubblewrap profile. They do not prove AgentOS/LLM execution, a controlled productivity comparison, or general security. The process tests are ignored by normal `cargo test --all-targets` and were separately executed by the isolated runner. No external services, secrets, model calls, new cost, deployment, or production changes were used.

Cargo licensing metadata was not changed. CONT-287's recorded state remains the last explicit readback: `license=null`, `license_file=null`, `publish=[]`, no product license selected, and redistribution rights unassessed. This does not complete the dependency/source rights inventory.

## Next

Do not repeat this path absent a contract change. Continue with an uncovered functional refusal or recovery boundary and complete the sourced dependency/source license inventory. A controlled Forgejo-plus-agent baseline and any claim of general gain remain unproven.
