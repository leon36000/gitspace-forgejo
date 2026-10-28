# WEBHOOK-DUPLICATE-HOST-PROCESS-001

Date: 2026-10-25

## Scope

Added `actual_receiver_rejects_duplicate_host_fields_before_append` to `tests/webhook_receiver_process_e2e.rs`. The test starts the actual local receiver, creates an empty private inbox, builds a correctly HMAC-signed issue event, then adds a second conflicting `Host` field. It requires HTTP 400 (`request_malformed`, non-retryable), byte-identical inbox contents, and an empty journal replay.

The shared worktree already contained uncommitted `Host` cardinality and authority validation in `src/webhook/http.rs` when this subtask began. I preserved that user-owned change and did not alter production code. Therefore the new process test passed immediately once its fixture was corrected; this subtask did not observe a red product test and did not independently establish who authored the pre-existing parser change.

The first test attempt failed in test setup because the receiver does not create a nonexistent inbox file at startup. The fixture now creates and syncs an empty mode-0600 inbox before starting the receiver. This was a harness correction, not a product failure.

## Verification

Commands were run offline against local code and loopback fixtures only; no credentials or external services were accessed.

| Command | Result |
| --- | --- |
| `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_rejects_duplicate_host_fields_before_append -- --exact --nocapture` | Pass, 1/1 |
| `cargo test --offline --locked --test webhook_receiver_process_e2e actual_receiver_rejects_ -- --nocapture` | Pass, 6/6 related rejection tests |
| `cargo test --offline --locked --test webhook_http_contract` | Pass, 6/6 |
| `cargo fmt --all -- --check` | Pass after formatting the new test |
| `git diff --check` | Pass |

The initial format check found one formatting change in the new test assertion; `rustfmt --edition 2024 tests/webhook_receiver_process_e2e.rs` applied it, and the final workspace format check passed. The initial test setup failure is recorded in the turn transcript; the successful final logs are in this directory.

Toolchain: Rust `1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, rustfmt `1.9.0-stable (59807616e1 2026-04-14)`, Python `3.12.3`, Linux `7.0.0-1019-nvidia aarch64`.

## Limits

No production code changed in this subtask. No full workspace test suite or live Forgejo instance was run. The current parser implementation is part of the heavily modified shared worktree and remains uncommitted/user-owned.
