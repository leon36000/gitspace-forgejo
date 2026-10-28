# MERGE-016 paginated tree binding — replay 021

Date: 2026-10-23 (America/Montreal)

## Scope and provenance

This is a private local replay against Forgejo `17.0.0+gitea-1.22.0`. It exercised the isolated MERGE-016 candidate in this directory, not the canonical dirty worktree source. The sandbox copy has no Git metadata, so its exact source commit is not established. Tool versions: Rust/Cargo `1.95.0`, Bubblewrap `0.9.0`. SHA-256: gate source `5c4e06cda8dddec60b1368bd154b2f0ed222f7988da6994d673f42e7cf40ecf0`; E2E runner `93a7aa6b4e5f810b78c4e8ebf2821038b7bbbeb0c7cd2802fdeeceaeb8088036`; fixture candidate `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`; `Cargo.lock` `4cd31487a6a4c8ab6074375a7478d9f23506e3cd90d18fdda4d90a9591e93afb`.

The runner reused a previously produced candidate source from `mission-012-webhook-composed-run-005/live-mcp-006`; its SHA-256 is recorded in `result.json`. That source provenance was not verified in this replay. No model was invoked during this run. This therefore demonstrates the Rust gate against a replayed candidate, not a fresh LLM-driven request-to-merge path.

## Reproduction

The private Forgejo binary was `/tmp/codex-forgejo-v17.0.0/forgejo-17.0.0-linux-arm64` (SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`). The run command was:

```sh
python3 scripts/run_mission_012_live_target_gate_e2e.py \
  --forgejo-binary /tmp/codex-forgejo-v17.0.0/forgejo-17.0.0-linux-arm64 \
  --candidate-source evidence/mission-012-target-gate-replay-020/candidate-src-lib.rs \
  --state-dir /tmp/mission012-target-gate-paged021 \
  --evidence-dir evidence/mission-012-target-gate-replay-021
```

The runner built the Rust gate offline in Bubblewrap with networking disabled. The local Forgejo instance listened only on `127.0.0.1:3132` and was stopped in the runner's `finally` path; a post-run socket check found no listener on that port. Private state remains at `/tmp/mission012-target-gate-paged021` and was not deleted.

The fixture seeded 1,001 small root files before branch protection, for 1,008 root entries. Issue #1 and PR #3 were exercised. The base was `597bc70f2a17ab516fb748b06a5c5e93de555bc7`, candidate `ad06782e3eab4fbcf6587b9e532aa87129aad3db`, and merge/main `17dc37b6c6327183805d055c667b48e2f2210228`. The live gate reported equal Forgejo entry-manifest fingerprints for the reviewed candidate and merge commit (`fa44a174d454c8cba9364c857f0e3f1f4bd0f61e30f2eb176e7211e90b29e856`). A separate local Git readback found the candidate and merged commit tree OIDs equal (`50abb9eb819338488fe0ec4e1dbb7850285bc987`), the merge parents equal the recorded base and candidate commits, `main` at the merge commit, the PR merged, and the issue closed.

The replay refused a missing proof, missing required status, an executor's fake-success status, a valid status without a review, and a stale candidate proof. The stale proof did not attempt a merge. Result and individual refusal records are in this directory.

## Paginated API coverage

Two focused Rust tests in `cargo-test-all-targets.log` passed. The 1,001-entry fixture models Forgejo v17's `truncated=true` value on both pages, including the final partial page, and asserts the exact `page=1` and `page=2`, `per_page=1000`, `recursive=false` request lines. A second test asserts that a 404 on page 2 is an error. The live run's gate result depends on the full manifest read, but the raw HTTP requests from the live Forgejo process were not retained; the unit fixture is the direct request-line evidence.

## Verification

- `cargo test --offline --locked --all-targets -- --test-threads=4`: 32 test-result groups, 363 passed, 0 failed, 12 ignored.
- `rustfmt --edition 2024 --check src/bin/forgejo-reconcile-local.rs`: passed.
- `python3 -m py_compile scripts/run_mission_012_live_target_gate_e2e.py`: passed.
- `cargo fmt --all -- --check`: failed on formatting hunks in `src/forgejo.rs`, `src/lib.rs`, and `tests/webhook_receiver_process_e2e.rs`. These files were not edited in this tranche. Because this sandbox copy has no Git history, the age of those formatting differences cannot be established.

## Limits

This is a local test gate, not production merge authority. The process identity is a host-supplied label, not an OS identity. Its lock coordinates cooperating local clients only; Forgejo's merge endpoint has no expected-base compare-and-swap here, so a non-cooperating writer can still race the final check. Matching fingerprints establish matching read-back content, not software correctness. This replay does not integrate the gate into the fresh LLM pipeline, AgentOS, Forge Code, RAG, or memory, and makes no comparative productivity claim.

The gate change and added pagination assertions remain in this isolated copy; they have not been ported to the canonical dirty worktree. Do not infer that canonical source contains this fix from this evidence.
