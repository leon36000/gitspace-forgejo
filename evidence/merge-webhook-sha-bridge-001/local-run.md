# MERGE-WEBHOOK-SHA-BRIDGE-001 — readback SHA preserved through signed webhook reconciliation

Date: 2026-10-25. Product worktree `local/WEBHOOK-005-canonical-links`, Git HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; the worktree was dirty. The isolated runner recorded the staged source-tree and lockfile digests in `isolated-e2e.log`.

## Scope and result

One ignored process-level Rust integration test was run through Bubblewrap 0.9.0 with `--unshare-net` and `--unshare-pid`. It exercises the actual `forgejo-reconcile-local` merge authority, actual `forgejo-webhook-local` receiver, and actual read-only reconciliation CLI. A local HTTP fixture occupies the compiled-in MISSION-012 loopback origin; this is not a live Forgejo instance.

The merge authority validates a synthetic target-bound verifier proof, commits one merge POST in the fixture, and returns `merge_commit_sha=3333333333333333333333333333333333333333`. The test then admits a signed synthetic `pull_request` delivery through the receiver, shuts it down cleanly, and reconciles that delivery against the same fixture. The resulting `canonical_state.merge_commit_sha` equals the merge authority readback SHA; the PR is `closed` and `merged`, and the webhook inbox bytes are unchanged by reconciliation. The fixture observed one merge POST and a GET of PR 23 during reconciliation. The focused process test passed 1/1, with 0 failures and 3 filtered tests.

This closes the data-flow link from merge readback to webhook canonical state. It does not rerun the independent verifier as a separate executable; the proof is a synthetic, locally signed test fixture consumed by the real merge CLI. It does not newly execute the merged repository tree, exercise a live Forgejo instance, demonstrate a model/AgentOS integration, or establish comparative project gains. No Forgejo source is copied or modified.

## Reproduction

The exact isolated test command is in `commands.txt`; the full Bubblewrap invocation, staged source-tree digest, toolchain versions, and raw result are in `isolated-e2e.log`. The runner gained repeatable `--only TEST_NAME` selection so this test could run without replaying unrelated process scenarios. The default runner matrix remains unchanged.

Focused Rust test compilation, runner CLI parsing, Python byte-compilation, rustfmt, and `git diff --check` each exited 0; outputs and exit codes are recorded alongside the process log. Source hashes for the test, runner, lockfile, and participating binaries/modules are in `source-sha256.txt`.

## Isolation and limits

- Bubblewrap reports Linux `aarch64`, Rust `1.95.0`, Cargo `1.95.0`, Python and Bubblewrap versions in `tool-versions.txt` and `isolated-e2e.log`.
- The Forgejo API is a deterministic HTTP fixture bound to `127.0.0.1:3132`; the webhook receiver uses a dynamically reserved loopback port inside the isolated test namespace.
- The signed delivery uses synthetic test secret bytes in a private temporary directory. No production secret, external service, or private user data is involved.
- One process scenario is evidence of this wiring only. LLM execution, AgentOS/RAG/memory integrations, controlled conventional baseline, and generalized efficacy remain unproven.
