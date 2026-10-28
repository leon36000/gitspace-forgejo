# MISSION-012 Runner Boundary Hardening Implementation Plan

> **For agentic workers:** preserve existing dirty work. Use TDD and the bounded Claude Code/Ralph workflow described in the task prompt.

**Goal:** Close five review-confirmed resource and trust-boundary defects in the local composed MISSION-012 harness.

**Architecture:** Keep the Python harness and current Forgejo/MCP flow. Make each build input and child-process boundary explicit, and verify each fix with focused tests before integration.

**Tech Stack:** Python 3.12, `unittest`, existing Forgejo/Rust runners. Claude Code CLI was attempted in a sanitized staging copy.

**Spec:** `docs/superpowers/specs/2026-10-23-mission012-runner-boundaries-design.md`.

## Global Constraints

- No production changes, publication, commits, dependency installation, credential use, or deletion of existing state.
- The Claude model sees only selected source/tests in a private staging copy; never include `.runtime`, secrets, evidence logs, or user data.
- The Claude Code CLI attempt was requested with a USD 0.20 cap, but its final event reported USD 0.5258432 and `error_max_budget_usd`. Do not make further model CLI calls in this tranche.
- No live LLM E2E rerun.
- Do not describe same-UID process cleanup as a security sandbox or the Forgejo merge API as an atomic base/head gate.

## Review Focus

- Builder credential exposure: a `.runtime` sentinel must not appear in the build snapshot or sandbox mount.
- Direct pushes: a protection with `enable_push=true` or a missing value must fail.
- Descendants on timeout: stage-owned services and same-stage descendants must not survive cancellation.
- EOF plus blocked parent: a process that closes stdout and keeps running must time out and be terminated.
- Claude output: output above the combined cap must be stopped without unbounded in-memory or disk capture.
- Stage subprocesses: pass only an explicit environment allowlist; do not forward arbitrary host environment variables.

### Task 1: Exclude credentials from the Cargo build sandbox

**Files:** `scripts/run_mission_012_live_admission_e2e.py`; `tests/test_mission_012_live_admission_e2e.py`.

- [x] Add a regression test for an explicit build snapshot containing manifest/lock/source but excluding a `.runtime` sentinel.
- [ ] Run that test red.
- [x] Build and mount only the explicit snapshot (including `build.rs` or Cargo config only if the crate actually needs it).
- [x] Run focused admission tests and verify green.

### Task 2: Enforce the fixture's branch-push policy

**Files:** `scripts/run_mission_012_live_agent_pr_e2e.py`; `tests/test_mission_012_live_agent_pr_e2e.py`.

- [x] Add cases for missing and true `enable_push`.
- [x] Require `enable_push is false` and verify green.

### Task 3: Keep local services inside the stage lifecycle

**Files:** the admission, composed, PR, and MCP supervisor scripts; their focused tests.

- [x] Add tests that a stage child closing stdout while remaining alive returns timeout and is cleaned up.
- [x] Ensure `TimeoutExpired` takes the process-group termination path.
- [x] Remove unnecessary new-session escapes for receiver/Forgejo/MCP host; add a direct process-group inheritance assertion.
- [x] Run the focused lifecycle tests green.

### Task 4: Bound Claude CLI output while it is produced

**Files:** `scripts/run_mission_012_live_mcp_agent_e2e.py`; `tests/test_mission_012_mcp_supervisor.py` or a focused sibling test.

- [x] Add streaming-output/timeout tests with noisy or pipe-closing children.
- [x] Use a streaming capture with one combined byte budget and process-group cleanup on overflow, timeout, and parent termination.
- [x] Preserve bounded private output logs and existing result/token accounting.
- [x] Run focused output and supervisor tests green.
- [x] Reproduce the cleanup-entry SIGTERM failure against the throwing-handler variant, then make the handler non-throwing and drain the CLI process group after normal completion as well as cancellation.
- [x] Make the regression wait for a live redirected-output descendant, complete the CLI normally, inject SIGTERM at cleanup entry, and verify no descendant marker appears.

### Task 5: Integrate and record evidence

- [x] Run the four target Python unittest modules through focused discovery, `py_compile`, `git diff --check`, whitespace check, and copy-project smoke test in the real worktree.
- [x] Sonnet 5.5 review not run after the CLI budget overrun; do not attribute any findings to Sonnet.
- [x] Complete final native read-only review of cleanup-entry signal handling; it found no residual issue and did not execute tests.
- [x] Update the MISSION-012 report/checkpoint with exact verification, hashes, model-reported cost, and the fact that the full LLM E2E was not rerun.
- [x] Preserve all prior state and do not commit.

## Outcome (2026-10-23)

The four target Python unittest modules pass locally (34 tests, 20.349 seconds); `py_compile`, `git diff --check`, the untracked-source whitespace scan, and the copy-project smoke check also pass. A cleanup-entry SIGTERM regression was reproduced against a staging variant of the throwing handler, then fixed with a non-throwing handler and unconditional same-group child cleanup; the test waits for a live redirected-output descendant before injecting the signal. The final native read-only review found no residual issue and did not run tests. The full composed LLM/Forgejo E2E was not rerun after hardening, and no Rust build was run for this Python-only slice. The requested Claude Code attempt ran in the sanitized staging copy with `--max-budget-usd 0.20`, but returned CLI-reported `total_cost_usd=0.5258432`, `subtype=error_max_budget_usd`, exit code 1, before code or test changes. No further paid model call was made. See `evidence/mission-012-runner-boundary-hardening-001-local-run.md`.
