# CONTINUATION-253 — webhook replay provenance contract

Date: 2026-10-27. Source worktree base: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; pre-lot status contained 1,626 changed/untracked entries. The existing dirty tree was preserved; no checkout, commit, service deployment, or upstream source was changed.

## Finding and change

`src/webhook.rs::ingest_forgejo_delivery` verifies the Forgejo HMAC before appending the parsed payload. The journal does not store the raw body or signature. `read_journaled_webhook_event` later validates the journal chain and envelope, but its source comment correctly says it does not reverify that HMAC and trusts the local filesystem writer.

Before this lot, that limitation appeared on selected error/capability paths but was absent from normal single and batch result envelopes. A consumer could read `current_state_read` without seeing the distinction between a canonical API read and source-webhook authentication. No new key or provenance service was introduced. Single responses now use schema 11, batch responses schema 12, capabilities schema 10 / `forgejo-reconcile-capabilities-v10`; each publishes the same `source_event_provenance_contract`. Its `current_state_read_means` is `canonical_api_read_only`; the HMAC is explicitly `not_performed` during replay, its per-delivery attestation is absent, and the hash chain does not authenticate a local writer. This is a contract description, not evidence that a particular legacy or rewritten row passed the receiver.

The schema bump was propagated through local Rust contracts, Python validators, and synthetic expected outputs. No Rust or Python dependency changed. The gate does not authorize a merge or change webhook ingestion.

## Red/green and verification

TDD red: `cargo test --offline --locked --bin forgejo-reconcile-local successful_reconciliation_states_that_webhook_hmac_is_not_reverified` failed before the output field existed (`null` versus expected `not_performed`). After implementation it passed 1/1. During schema migration, `webhook_quarantine_cli` once failed because one expected single-response version still said 10; the expectation was corrected to single 11 / batch 12, and the final run passed.

Fresh captured checks are in `checks-summary.json` and the per-command logs:

- `forgejo-reconcile-local` unit tests: 158 passed.
- Capabilities contract: 5 passed, 2 ignored (fixed-port scenarios).
- Single/batch CLI contract: 4 passed; quarantine CLI: 5 passed; journal-lock timeout: 3 passed.
- A fresh batch receiver → real Rust CLI case ran through `scripts/run_webhook_reconcile_cli_e2e.py` with Bubblewrap `--unshare-net --unshare-pid`: 1 passed, 153 filtered. It checks the version and provenance contract on the actual batch response while the API fixture remains local.
- Python consumer suites: emitted-webhook 131 passed, organization/reviewer 29 passed, native-replay 30 passed. The five affected runner scripts compile.
- `cargo fmt --all -- --check`, `git diff --check`, and `scripts/verify_cargo_license_state.py`: exit 0.

The `replay_checks.py` script reruns the complete listed set offline and writes exact command output under this evidence directory. `output-contract.json` records the public values. SHA-256 entries are in `source-sha256.txt` and the append-only manifests.

## Limits and open work

This verifies an interface statement using unit, process-fixture, capability, and validator tests. It does not cryptographically authenticate the local journal writer, preserve a receiver attestation, or reverify the HMAC at replay. The isolated batch test used a synthetic loopback API; this lot did not run a live Forgejo instance, an AgentOS provider, or an LLM and incurred no model cost. It proves no productivity or comparative gain. Universal writer admission, the remaining refusal matrix, upstream redistribution/licensing questions, and the controlled Forgejo+agent+CI comparison remain open.
