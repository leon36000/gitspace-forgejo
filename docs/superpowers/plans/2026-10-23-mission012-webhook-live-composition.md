# MISSION-012 Webhook-to-PR Composition Plan

> **For agentic workers:** execute this plan task by task with TDD and preserve all existing local changes.

**Goal:** Make the isolated MISSION-012 live run admit the task from an HMAC-signed event received by the real local webhook process, then pass that same host admission through MCP, the authorized Claude CLI, independent PR verification, and controlled sandbox merge.

**Architecture:** Keep Forgejo as the Git/issue/PR authority and reuse the existing Rust receiver, admission CLI, MCP host, Claude adapter, and PR verifier. Extend the existing live admission runner with an explicit opt-in signed-delivery mode; keep its default direct-local path and historic evidence intact. Run the three existing stages against one fresh owner-only state directory and bind stage outputs by the admission, receipt, candidate, PR, and merge hashes.

**Tech Stack:** Rust 1.95/Cargo offline locked; Python 3; Forgejo 17 loopback fixture; Bubblewrap verifier; Claude Code Opus 5.5 through the configured MCP-only adapter with its existing $0.20 cap.

**Spec:** `docs/implementation-plan.md` sections `MISSION-012-LIVE-AGENT-E2E` and `WEBHOOK-INBOX-CAPACITY-001`; project mission: end-to-end local request → requirements → mission → context → code change → PR → independent verification → motivated controlled merge.

## Global Constraints

- Preserve dirty and untracked work; do not commit, publish, touch production, delete state, or expand access.
- Keep synthetic Forgejo/webhook state under a fresh owner-only direct child of `/tmp`; use loopback only.
- Do not send credentials, secrets, or private runtime artifacts to Claude; pass only the existing MCP tools and synthetic task context.
- Keep the model cap at or below USD 0.20; do not increase it on refusal or budget exhaustion.
- Report signing as provenance/integrity, not correctness; the same-UID and non-atomic base/head merge limits remain.

## Review Focus

- Invalid webhook signature and replayed delivery ID: reject or accept idempotently before claim; exercise the actual receiver.
- Anonymous Forgejo admission: refuse before durable claim/DTO; retain the current refusal check.
- Admission DTO, MCP receipts, and PR candidate mismatch: refuse on binding/hash mismatch; use the existing checks and record the handoff SHA.
- Candidate regression or protected acceptance changes: independent verifier must refuse; retain the base-red/candidate-green check.
- Base/head or approval movement before merge: fail closed on final readback; preserve and report the residual non-atomic API window.

---

### Task 1: Add opt-in real signed-delivery admission

**Files:**
- Modify: `scripts/run_mission_012_live_admission_e2e.py`
- Test: add focused tests under `tests/test_mission_012_live_admission_e2e.py`

**Interfaces:**
- Consumes: the existing private disposable Forgejo state and `forgejo-mission-local` admission DTO contract.
- Produces: optional `--signed-webhook-delivery`, which starts the real `forgejo-webhook-local` on an ephemeral loopback port, signs a synthetic issue event, stores it in `.runtime/forgejo-webhook-inbox.jsonl`, then invokes admission with the exact delivery ID and requirement digest. Default mode remains `direct_local`.

- [x] Add tests for signature construction; the live receiver run checks wrong-signature refusal, accepted delivery binding, and duplicate delivery behavior.
- [x] Run the focused Python tests and observe the expected failure before implementation.
- [x] Implement bounded receiver startup/shutdown, private synthetic secret handling, signed event send/readback, and isolated offline build of the receiver.
- [x] Run focused tests and the actual signed-delivery admission stage; verify `admission_mode=webhook_delivery`, one journal event, and DTO scope/issue/base/hash.

### Task 2: Compose the existing MCP, PR, verifier, and merge stages

**Files:**
- Create: `scripts/run_mission_012_live_composed_e2e.py`
- Test: focused Python tests for stage ordering, argument bindings, and fail-closed child failure.

**Interfaces:**
- Consumes: Task 1 runner output; `run_mission_012_live_mcp_agent_e2e.py`; `run_mission_012_live_agent_pr_e2e.py`.
- Produces: one reproducible command that runs admission, MCP/Claude, and PR verification/merge against a single new `/tmp` state, with explicit execution ID, evidence directory, and existing maximum model budget.

- [x] Add tests that assert a failed admission stops before the model, a failed model stops before PR, and the same state/admission binding is passed between stages.
- [x] Run focused tests and observe expected failures.
- [x] Implement bounded subprocess calls and write only sanitized stage summaries to the evidence directory; never copy tokens or private runtime state there.
- [x] Run the full composed local chain with Opus 5.5 under the existing USD 0.20 cap; capture model-reported cost as unverified.
- [x] Verify the candidate commit, independent verifier decision, approval binding, merge commit/tree, and issue state by readback.

### Task 3: Record evidence and reconcile canonical status

**Files:**
- Modify: `docs/implementation-plan.md`
- Modify: `docs/checkpoint.md`
- Create: `evidence/mission-012-live-agent-webhook-compose-005-local-run.md` and its SHA-256 manifest.

- [x] Update obsolete historical claims: distinguish the already implemented DTO/process handoff from the new receiver-to-LLM/PR composition.
- [x] Record exact commands, source commit, toolchain, run IDs, hashes, pass/refusal results, and which evidence is fixture, process, live Forgejo, or model-reported.
- [x] Preserve caveats: synthetic test accounts, same-UID handoff integrity, non-atomic merge base/head, no AgentOS/RAG/memory, n=1, and no comparative value claim.
- [x] Run targeted Python tests, the composed E2E if the existing configured model is available within the cap, `git diff --check`, and verify the report manifest. The post-run subprocess cleanup hardening is unit-tested; the LLM path was not rerun after that change to avoid a third model call.
- [x] Inspect the complete diff and checkpoint the next blocked boundary without committing.
