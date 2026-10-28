# CONTINUATION-283 — admission refusal for a torn attested inbox

Date: 27 October 2026. Base Git HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. The checkout is deliberately dirty with extensive pre-existing tracked and untracked work; this lot did not stage, commit, reset, or clean it.

## Finding and change

`read_journaled_webhook_event_verified` already uses read-only journal replay, which refuses an incomplete final JSONL record without repairing it. However, `forgejo-mission-local::inbox_read_error` sent `JournalError::Corrupt` through the generic `webhook_inbox_untrusted` fallback. Its action told operators to repair owner/mode metadata, hiding that the inbox contents were corrupt.

The admission CLI now returns `webhook_inbox_corrupt`, identifies the failing line, sets `canonical_state_created=false`, and recommends preserving a copy and using the trusted receiver recovery path or a verified backup. This changes the diagnosis; the underlying refusal and non-mutating read-only journal behavior were already present.

## Process scenario

The ignored integration test `launched_cli_reports_torn_attested_journal_before_credential_without_repair` writes one valid synthetic issue delivery through the HMAC and Ed25519 receiver-attestation path, then appends the incomplete bytes `{"sequence":` without a terminating newline. It launches the compiled `forgejo-mission-local` process with the correct trusted public key, a missing credential path, and a handoff path.

The process exits 1 with `verdict=inconclusive`, `error.code=webhook_inbox_corrupt`, and `details.line=2`. The credential remains absent; the mission journal and handoff are not created; inbox bytes are identical before and after. The runner executes only this process case under Bubblewrap `--unshare-net --unshare-pid`: **1 passed, 0 failed, 6 filtered out**. The fixture is synthetic; no Forgejo server is started or contacted.

TDD evidence: before the mapping change, the same test got `webhook_inbox_untrusted` instead of `webhook_inbox_corrupt`. A first post-change retry reached the wording assertion and showed that its expected phrase was too strict; the test assertion was corrected to match the emitted action. The red diagnostic is recorded in `red-diagnostic.txt`.

## Additional verification

`cargo test --locked --offline --bin forgejo-mission-local` passed **36/36** unit tests. `cargo fmt --all -- --check`, `git diff --check`, Python AST parsing of the runner, and `scripts/verify_cargo_license_state.py` exited 0. The Cargo metadata guard reports `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, redistribution rights `not_assessed`, and `NOASSERTION`; no product license was selected.

Commands, environment, full isolated runner output, unit output, checks and source digests are stored beside this report. Append-only local/worktree/project manifests are created and checked by `append_manifests.py`; historical prefixes are required to remain byte-identical.

## Scope and limits

This is one Linux/aarch64 local process scenario with synthetic HMAC, key and payload. It establishes a precise refusal and diagnostic for the tested torn-tail condition, not recovery across storage failure, anti-rollback of a valid journal prefix, a privileged host threat, all corruption forms, Forgejo live, an LLM/AgentOS run, or comparative value. Signature verification establishes provenance/integrity relative to the configured key, not correctness. The overall forge mission remains incomplete.
