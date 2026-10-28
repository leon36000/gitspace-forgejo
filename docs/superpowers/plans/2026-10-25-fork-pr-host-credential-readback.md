# Fork PR Host-Credential Readback Implementation Plan

> **For agentic workers:** This plan is executed natively in the current session. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that the real webhook receiver and reconciliation CLI can read a private target repository's fork PR using only the host-provided credential pipe.

**Architecture:** Reuse the signed receiver, durable inbox, real `forgejo-reconcile-local` process, host credential FD, and existing authenticated loopback fixture. The fixture accepts the request only when the synthetic token is present; the test binds the API path and returned base to the configured target while allowing a foreign head repository.

**Tech Stack:** Rust integration test, Python Bubblewrap runner, loopback HTTP fixture, Cargo offline/locked.

**Spec:** [`docs/architecture.md`](../../architecture.md), fork-PR/current-state tests in [`tests/webhook_receiver_process_e2e.rs`](../../../tests/webhook_receiver_process_e2e.rs), and credential-boundary tests in the same file.

## Global Constraints

- Keep all test traffic in the Bubblewrap loopback fixture; no live Forgejo instance or external network.
- Use a synthetic test token only; assert it appears in neither CLI output stream.
- Do not expose the credential to the receiver or webhook payload.
- Keep the event, API path, and canonical base scoped to the configured target repository; the head may be a contributor fork.
- Do not change production code, Cargo license fields, or publish policy.

## Review Focus

- Missing or wrong host token must not be accepted by the API fixture; the selected scenario requires the token.
- A fork head must not redirect the API request away from the configured target.
- A response with a different base repository must not become canonical; the test fixture supplies the configured base.
- Stale webhook SHAs must not override the current API SHAs.
- Read-only reconciliation must leave the durable inbox byte-identical and keep the token out of stdout/stderr.

---

### Task 1: Compose fork PR readback with host credential delivery

**Files:**
- Modify: `tests/webhook_receiver_process_e2e.rs`
- Modify: `scripts/run_webhook_reconcile_cli_e2e.py`
- Create: `evidence/webhook-fork-pr-host-credential-001/`
- Modify: `docs/checkpoint.md`

**Interfaces:**
- Reuse `start_authenticated_cli_http_fixture(response_body, expected_authorization) -> JoinHandle<(String, bool)>`.
- Reuse the `--host-credential-fd` CLI contract and the existing HMAC receiver helpers.
- The runner adds the exact test name to `TEST_NAMES` and runs it inside the existing no-network Bubblewrap harness.

- [x] Add `actual_reconcile_cli_process_reads_fork_pull_request_with_host_credential_fd`; require the fixture to observe `Authorization: token <synthetic-token>`, assert exactly one GET to the configured target PR path, current API SHAs, no token in output, and unchanged inbox bytes.
- [x] Register the test in `scripts/run_webhook_reconcile_cli_e2e.py` and run only this scenario through Bubblewrap.
- [x] Run targeted Rust formatting, runner syntax, whitespace, Cargo-license guard/readback, and evidence/index verification.
- [x] Record exact commands, versions, exit statuses, source hashes, test scope, and limitations in the evidence dossier and checkpoint; update append-only SHA indexes without rewriting historical rows.

### Task 2: Refuse a fork PR read when the host credential is invalid

**Files:**
- Modify: `tests/webhook_receiver_process_e2e.rs`
- Modify: `scripts/run_webhook_reconcile_cli_e2e.py`
- Modify: `evidence/webhook-fork-pr-host-credential-001/`
- Modify: `docs/checkpoint.md`

- [x] Compose a signed fork PR delivery with the real receiver and CLI, then make the loopback API fixture reject an invalid synthetic host credential.
- [x] Assert exactly one request to the configured target PR route, `upstream_unauthorized`, no canonical state, no credential leakage, and byte-identical inbox.
- [x] Run the targeted Bubblewrap scenario (1/1, 72 filtered), formatting, syntax, whitespace, Cargo license checks, and refresh evidence manifests/indexes.

### Task 3: Keep a private fork PR inconclusive without a host credential

- [x] Add a process scenario that invokes the real reconciler without `--host-credential-fd` and has the private API fixture reject the anonymous read.
- [x] Require no Authorization header, one configured-target GET, a terminal `upstream_unauthorized` result without canonical state, and an unchanged inbox.
- [x] Run the isolated scenario (1/1, 73 filtered), formatting, syntax, whitespace, Cargo license checks, and refresh the evidence indexes.
