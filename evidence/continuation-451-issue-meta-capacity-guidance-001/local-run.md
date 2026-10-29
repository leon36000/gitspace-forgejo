# CONTINUATION-451 — issue repository metadata partial and inbox-capacity guidance

Date: 2026-10-28 (America/Montreal). Worktree: `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branch `local/WEBHOOK-005-canonical-links`, dirty before this tranche. No reset, commit, push, branch rewrite, deletion, or production change.

## Change and evidence

`tests/webhook_reconcile_contract.rs::current_issue_repository_metadata_must_match_the_requested_resource` was extended with partial identity cases. Before the implementation change, the targeted test failed because `repository: {"name":"agent-forge-core"}` returned `Ok(ReconciledState)` instead of `InvalidCurrentStateField("repository")`. `src/webhook/reconcile.rs::validate_current_issue_repository` now requires either `full_name` or both `owner` and `name`; if either member of that pair is present, the other must also be present. Each supplied value is checked against the requested repository. Missing/null whole metadata remains compatible; complete `full_name` alone and complete `owner`+`name` remain valid.

The new `actual_reconcile_cli_process_refuses_partial_issue_repository_metadata` test uses the real local webhook receiver, signed synthetic delivery, journal, real reconcile CLI, and a loopback HTTP fixture. `scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_partial_issue_repository_metadata` runs it under Bubblewrap `--unshare-net --unshare-pid`. It returns `inconclusive/invalid_current_state`, identifies the `repository` field, emits no canonical state, and leaves the accepted journal byte-for-byte unchanged. Result: 1/1; the other 209 tests in that binary were filtered.

Capacity errors had claimed that an operator could apply verified compaction/migration even though this build has no such command. HTTP refusal, receiver startup, reconcile CLI, mission CLI, and over-limit source snapshot now say where to pause, preserve a private copy, and not edit the inbox. The staged-restore refusal for an oversized snapshot separately states that there is no verified recovery path and says not to edit that snapshot. Tests assert this guidance and the capacity tests check unchanged or absent inbox state.

## Commands and results

- `cargo test --locked --offline --test webhook_reconcile_contract`: 109 passed, 0 failed.
- `cargo test --locked --offline --test webhook_inbox_capacity`: 6 passed, 0 failed.
- `cargo test --locked --offline --bin forgejo-webhook-local snapshot_and_restore_capacity_refusals_do_not_claim_a_migration_exists`: 1 passed, 0 failed.
- `cargo test --locked --offline --bin forgejo-mission-local mission_capacity_error_names_only_available_recovery_actions`: 1 passed, 0 failed.
- `python3 scripts/run_webhook_reconcile_cli_e2e.py --scenario actual_reconcile_cli_process_refuses_partial_issue_repository_metadata`: Bubblewrap process test 1 passed, 0 failed.
- `cargo test --locked --offline --all-targets -- --test-threads=1`: exit 0 across 39 test targets: 789 passed, 0 failed, 212 ignored; captured at [all-targets.log](all-targets.log). Ignored tests are not counted as executed by this command. The separate Bubblewrap scenario above is the only new process scenario claimed here.
- `cargo fmt --all -- --check`, `git diff --check`, Python `compile(...)` of the changed runner, and `python3 -B scripts/verify_cargo_license_state.py`: pass.

Environment: Linux aarch64, kernel `7.0.0-1019-nvidia`, Rust/Cargo `1.95.0`, Python `3.12.3`, Bubblewrap `0.9.0`. Lockfile and Cargo tests were run offline. Full command list: [commands.txt](commands.txt); source checksums: [source-sha256.txt](source-sha256.txt).

## GitLab state and limits

The user's request to continue on PC4 was honored with read-only calls through the already authenticated GitLab PC4 connector; the email itself was not opened and no credential value was read or copied. Readback reports private project `root/gitspace-forgejo` (ID 4), default branch `codex/pc6-transfer-2026-10-28`, and pipeline 328 failed at transfer commit `9ac877767e9b458a9cd911e67a10f87aeeaefbdb` on `codex/pc6-transfer-work-2026-10-28`. The earlier redacted report identifies ten 64-character Forgejo read-token-shaped evidence values with revocation unproven and a potentially exposed CI upload token. No GitLab write was made in this tranche. The remote write/cleanup decision remains with the user.

The process test uses a synthetic API fixture, not a live Forgejo server; `n=1`. The capacity message does not implement or validate recovery/rotation. Retention policy is still undecided. No AgentOS, LLM, RAG, or memory interface was exercised; this lot demonstrates no comparative gain.

Cargo licensing remains deliberately unchosen: `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, redistribution rights `not_assessed`, SPDX custom marker `NOASSERTION`.
