# FORGEJO-PREFLIGHT-ANONYMOUS-401-002

Date: 2026-10-25 (local workspace date)
Product worktree HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`

## Finding and correction

The read-only merge preflight is deliberately anonymous: its capabilities entry names `read_only_merge_preflight` and `anonymous_only` in `src/bin/forgejo-reconcile-local.rs` (current source around lines 4683 and 4864). On HTTP 401, the structured refusal previously advised checking a host credential that this operation does not send. This made the next step misleading, especially for private repositories.

The preflight 401 action now says to confirm anonymous readability and states that the read-only request sends no credential; private-repository checks remain within the separate host-authenticated merge authority. The status remains `upstream_unauthorized`, verdict `inconclusive`, non-retryable, with HTTP status 401. It does not authorize merge or claim that private repository access was checked.

The ignored process test `actual_preflight_cli_reports_anonymous_401_without_claiming_a_credential_was_sent` drives the actual CLI against a loopback fixture. It asserts one GET, no Authorization header, structured JSON, no merge success, no retry, explicit anonymous/no-credential guidance, and no disclosure of a marker in the upstream response body. The unit test `merge_preflight_unauthorized_error_explains_anonymous_read_contract` locks the same action semantics.

The initial red failure was observed before the fix, but its raw log was not retained. `tdd-red-observation.txt` records it transparently as an annotation, not a raw transcript.

## Verification

Commands and results are preserved in the adjacent logs:

- `CARGO_TARGET_DIR=/tmp/agent-forge-webhook-046-target cargo test --offline --locked --bin forgejo-reconcile-local merge_preflight_unauthorized_error_explains_anonymous_read_contract -- --nocapture`: 1 passed. An earlier `--exact` invocation matched zero tests; its output is retained as `cargo-unit-filter-miss.log` and is not counted as verification.
- `PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_preflight_cli_e2e.py --scenario actual_preflight_cli_reports_anonymous_401_without_claiming_a_credential_was_sent`: 1 passed inside Bubblewrap with the network namespace isolated.
- `PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_preflight_cli_e2e.py`: 3 scenarios passed, including the pre-existing stale-proof refusal and HTTP parser case.
- `CARGO_TARGET_DIR=/tmp/agent-forge-webhook-046-target cargo test --offline --locked --all-targets -- --test-threads=1`: 462 passed, 0 failed, 33 ignored across 34 test groups.
- `cargo fmt --all -- --check`, `git diff --check`, `git diff --cached --check`, runner Python compilation, and `scripts/verify_cargo_license_state.py`: passed. Cargo readback remains `license=null`, `license_file=null`, `publish=[]`, decision `unselected`, redistribution rights `not_assessed`.

The preflight runner now stages only `Cargo.toml`, `Cargo.lock`, `src/`, `tests/`, and `build.rs`, copies only locked registry inputs, mounts that staged source read-only, uses an isolated network namespace and a tmpfs home, and does not bind the host root or host home. It runs the fixture inside Bubblewrap; the test does not connect to Forgejo.

## Evidence and limits

`process-e2e-matrix.log` records the staged-source, Cargo-lock, runner, registry, tool and platform hashes/version strings plus commands and results. `source-sha256.txt` records current source and manifest hashes. `SHA256SUMS` verifies the evidence files in this directory.

This establishes the adapter's anonymous 401 contract against a synthetic local HTTP fixture. It does not demonstrate behavior against a live Forgejo instance, a private repository, a model/LLM, a verified credential, end-to-end merge, or comparative project value. It changes guidance and regression coverage only; no Forgejo server or production instance was modified.
