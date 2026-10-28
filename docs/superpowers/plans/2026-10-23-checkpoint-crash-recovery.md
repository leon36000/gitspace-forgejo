# Reprise durable des checkpoints après arrêt — plan

> **For agentic workers:** implement this plan task-by-task with test-first changes; the repository is an ignored local prototype snapshot, not an independently versioned package.

**État canonique au 25 octobre 2026 :** les cases cochées documentent le snapshot ignoré `.runtime/mission-012-agent-pr-gate-004/` uniquement. Le worktree produit n’a pas reçu les implémentations des Tasks 1–5 ni la feature `test-crash-injection`; la Task 6 y reste à porter sélectivement. Voir [le rapprochement des sources produit](../../evidence/mission-012-product-port-gap-001/local-run.md).

**Goal:** recover the exact unverified checkpoint after either of the two observed process-crash windows without granting a stale worker authority or invalidating an otherwise resumable patch.

**Architecture:** keep `MissionExecutionJournalV1` as the only transition authority. Add provenance to resumed lease events so repeated attempts can point to the same latest released checkpoint; let the trusted host promote a verified, pre-expiry orphan checkpoint to the journal after expiry, then resume it normally. This is an append-only extension and does not make the JSONL file and checkpoint file one atomic storage transaction.

**Tech Stack:** Rust 2024, existing JSONL `EventJournal`, private local receipt/checkpoint files, Rust contract tests; no dependency or database migration.

**Spec:** [architecture decision, checkpoint recovery subsection](../../architecture.md#checkpoint-crash-recovery).

## Global Constraints

- Keep the existing local-only, no-publication scope and preserve all unrelated modified files.
- Do not set `license` or `license-file`; retain `publish = false`.
- Treat the journal as source of truth; agent-controlled status or checkpoint bytes alone never authorize a resume.
- Bind every resume to the exact requirement, request, allocation, source lease, checkpoint hash and fencing history.
- Fail closed on missing, altered, late, contradictory or superseded evidence.
- Keep same-UID writers and external Forgejo writers outside the isolation guarantee.

## Review Focus

- A different or newer checkpoint must supersede the old one; test old-hash refusal after a new release.
- A regular lease acquired after a checkpoint must invalidate that checkpoint; test that it cannot be resumed.
- Repeated retries must only follow resume leases bound to the same checkpoint hash; test both same-hash success and changed-hash refusal.
- Orphan promotion must require checkpoint creation before lease expiry and exact receipt bindings; test late checkpoint and duplicate promotion.
- Concurrency and fencing must remain atomic under the journal lock; retain the barrier race test and verify the losing worker cannot act.

---

### Task 1: Resume lineage in the journal

**Files:** `src/agent_runtime.rs`, `tests/mission_execution_journal_contract.rs`.

- [x] Add a failing contract: one released checkpoint can start a resumed lease, an expired second attempt can resume that same release with a higher fence, and the old checkpoint is rejected after a newer checkpoint is released.
- [x] Add a failing contract: a later ordinary lease or a resume attempt presenting another hash invalidates/refuses the old release.
- [x] Extend only new `mission_execution_lease_acquired_v1` payloads with optional `(resume_checkpoint_lease_id, resume_checkpoint_sha256)` provenance; preserve parsing of historical payloads without these fields.
- [x] Validate lineage from the latest release through every later lease in the generation, under the existing mission transaction lock.
- [x] Run `cargo test --offline --locked --test mission_execution_journal_contract` and preserve the existing concurrency/fencing contracts.

### Task 2: Promote a verified orphan checkpoint

**Files:** `src/agent_runtime.rs`, `src/lib.rs`, `src/bin/mission-012-mcp-host.rs`, `tests/mission_execution_journal_contract.rs`, host tests.

- [x] Add a failing contract for journal recovery after lease expiry using the exact lease, checkpoint hash and checkpoint creation time; reject a post-expiry checkpoint, an older lease and conflicting duplicate.
- [x] Add a dedicated append-only recovery event with actual recovery time and the asserted pre-expiry checkpoint time; keep ordinary release semantics unchanged.
- [x] In the host, call recovery only after parsing the private checkpoint, matching all admission/base/policy bindings and validating prior receipts; invoke this before acquiring any new lease.
- [x] Keep receipt-file suffix handling idempotent if the process stopped after journal append but before receipt append.
- [x] Sample the release/recovery clock inside the journal transaction; if normal release observes expiry there, retry via the expiry-recovery transition.
- [x] Run journal and host focused suites; test tampered, missing and late receipts remain refused.

### Task 3: Process-level crash replay and evidence

**Files:** host and lifecycle tests, `evidence/mission-012-live-agent-webhook-resume-008/`, `docs/checkpoint.md`, `docs/architecture.md`.

- [x] Reconstruct the checkpoint-written/journal-missing durable state from an actual host-produced checkpoint; restart and show verified promotion followed by a higher-fence resume.
- [x] Reconstruct lease-acquired/receipt-not-created, expire that lease, restart from the same checkpoint, and show the next resume succeeds while the prior lease is fenced.
- [x] Exercise checkpoint tampering, missing prior receipts and concurrent retries; verify there is no false `candidate_ready` result before the resumed red/green run.
- [x] Run the focused tests, `cargo test --offline --locked --all-targets`, release build, targeted rustfmt and `git diff --check`; record all outputs and exact source hashes.
- [x] Append results, coverage and residual same-UID / non-atomic-storage limitations to the canonical checkpoint.

### Task 4: SIGKILL at durable boundaries behind an opt-in test feature

**Files:** `Cargo.toml`, `src/bin/mission-012-mcp-host.rs`, `tests/mission_012_mcp_lifecycle.rs`, `evidence/mission-012-live-agent-webhook-resume-009/`, `docs/architecture.md`, `docs/checkpoint.md`.

- [x] Add a feature-gated test first and show it fails because no process kill occurs without the host failpoint.
- [x] Add two failpoints compiled only with `test-crash-injection`, and clear inherited test-hook environment for normal child starts.
- [x] Kill the real host after the checkpoint file is synced but before journal release, then recover the same patch content; disclose that the expiry fixture rewrites checkpoint metadata and hashes.
- [x] Kill a resumed host after its lease append but before receipt creation, let the short lease expire, then retry and verify red/green plus the higher fence.
- [x] Run the feature-specific test, default all-targets, release build, Clippy, rustfmt, diff-check and final source/evidence hashes; record that hooks require both the feature and `debug_assertions`.
- [x] Append the SIGKILL results and remaining limitations to the canonical checkpoint.

### Task 5: Characterize receipt/effect SIGKILL boundaries

**Files:** `src/bin/mission-012-mcp-host.rs`, `tests/mission_012_mcp_lifecycle.rs`, `evidence/mission-012-live-agent-webhook-resume-010/`, canonical docs.

- [x] Add debug-feature-only crash hooks after receipt-file directory sync, after `execution_started`, after durable source replacement before its receipt, and after receipt sync before JSON-RPC response.
- [x] Exercise real SIGKILL at each boundary; verify exact receipt state, changed candidate, absent response and fail-closed restart.
- [x] Verify default/debug-feature/release-feature suites, release binary hook absence, Clippy and scoped formatting.
- [x] Record that the protocol does not yet bind request IDs or replay complete responses, so this proves fail-closed behavior rather than exactly-once recovery.
- [ ] Design and implement a versioned durable per-call intent/result contract with idempotency keys, effect reconciliation and response replay before enabling external side effects.

### Task 6: Persist idempotent workspace-write intent and result

**Files:** `src/lib.rs`, `src/agent_runtime.rs`, MCP host and lifecycle tests, architecture and checkpoint docs.

- [ ] Add mission-scoped, versioned host-only intent/result events to the existing hash-chained mission journal; keep the journal as the sole operation-state authority.
- [ ] Bind the bounded `workspace.write_text` input, before/after source hashes, stable host-derived idempotency key, request correlation ID, exact MCP result body, and authorizing lease/fence. Reject same-key/different-input and malformed transitions.
- [ ] Persist intent before the effect and result before the response. If the effect reports uncertain durability or the observed file is neither the declared pre-state nor post-state, return an actionable inconclusive error.
- [ ] On a new higher-fence lease and fresh candidate from the admitted base, reconcile prior intents by replaying only the exact fixed-path source write after a fresh red baseline; duplicate completed calls return the stored result. Keep the old candidate isolated and unused.
- [ ] Add real-process SIGKILL cases before intent, after intent, after source sync/rename before result, and after result before response; test duplicate request/result replay, conflicting key/input, malformed journal, expired/stale lease, and divergent candidate refusal.
- [ ] Re-run focused, default, release-feature, Clippy, targeted formatting and whitespace checks; record resource/version, commands and hashes. Do not claim global exactly-once delivery or recoverability for external tools.
