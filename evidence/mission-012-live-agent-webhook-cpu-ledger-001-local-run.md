# MISSION-012 CPU ledger binding — local run report

Date: 24 October 2026  
Workspace HEAD observed: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`  
Scope: bind the final PR verifier CPU evidence to the admitted mission, reject incomplete or internally inconsistent accounting, and preserve the exact expected acceptance failure.

## Changes

- Admission validation now checks that `host_admission.allocation.mission_id` has the host-generated `mission-v1-<sha256>` form and equals `host_admission.request.mission_id`. The normalized mission identity is carried into both successful and refused PR results.
- The composed validator requires the final journal ledger to use that same mission ID, contain all three PR verification steps in order, and bind each step to its base and candidate commits, tested commit, reservation ID, policy, and admission-built MCP host hash.
- It independently recomputes the conservative CPU charge as `min(reservation, ceil((accounting_elapsed_ms + 1000) * 200 / 100))`. It rejects a sandbox duration above the parent accounting duration, any unresolved ledger entry, fractional/bool totals, and a stale-base readback whose CPU evidence differs from the validated result.
- The base-red receipt now identifies the expected failing acceptance test and assertion (`-2` observed, `10` expected, `1 passed; 1 failed`). The runner checks this output and its SHA-256; green steps must show both acceptance tests passing.
- Concurrent advance is reported as `refused_after_concurrent_base_advance_before_merge`, matching the observed state. Forgejo’s Rust authority returned `invalid_review_base_binding` after the approved competing PR advanced `main`.

The relevant implementation is in [the composed E2E validator](../scripts/run_mission_012_live_composed_e2e.py) and [the PR driver](../scripts/run_mission_012_live_agent_pr_e2e.py). Both files are untracked relative to the observed HEAD; the run records their SHA-256 values in `composition_source_sha256`.

## Verification

Fresh command:

```text
python3 -m unittest tests.test_mission_012_live_agent_pr_e2e tests.test_mission_012_live_composed_e2e
```

Result: 66 tests passed. The saved output is [the test log](mission-012-live-agent-webhook-cpu-ledger-tests.log). `py_compile` also passed for both runners and both test files.

### Positive merge — run 050

Command:

```text
python3 scripts/run_mission_012_live_composed_e2e.py --state-dir /tmp/mission012-composed-cpu-050 --forgejo-binary /tmp/forgejo-17.0.0 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/mission-012-live-agent-webhook-cpu-050 --execution-id live-mcp-050 --allocation-id mission012-local-adapter-050 --budget-cap-usd 0.20 --agent-adapter local-test-adapter
```

On local Forgejo v17, issue #1 was admitted from a real `issues/opened` webhook. The deterministic local adapter used the MCP context and tools to correct `src/lib.rs`; only that file changed. The base acceptance failed on the intended upper-bound regression (`-2`, expected `10`), candidate acceptance passed, and an independent pre-merge run passed. Forgejo refused merge before review and from the unauthorized reviewer (HTTP 405 each); the Rust merge authority then merged PR #2 and read back merge `08e8da450c6d39b0756bdea564581dade705f612`, with the issue closed.

The ledger has five resolved reservations (two agent-tool invocations and three PR checks): 12,776 ms charged of 1,200,000 ms. Mission ID: `mission-v1-94c26001d4370e63d3637fd5c06ef13f0496424a39a74bf498b7512b7d2b37b5`. No model was invoked; reported local-adapter cost is $0. The composed run SHA-256 is `9565bc82547812d5e735276ac86f2576b4cecd19fd09f88371a7c87e2b22b7a6`.

### Concurrent base advance refusal — run 049

Command: python3 scripts/run_mission_012_live_composed_e2e.py --state-dir /tmp/mission012-composed-cpu-049 --forgejo-binary /tmp/forgejo-17.0.0 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/mission-012-live-agent-webhook-cpu-049 --execution-id live-mcp-049 --allocation-id mission012-local-adapter-049 --budget-cap-usd 0.20 --agent-adapter local-test-adapter --inject-concurrent-base-advance.

Approved competing PR #3 advanced `main` from `3913109befe490dc3d221b0ae90e372404943f0b` to `83e6a8bac9849edaead35a086ab1f19a0d7790d7`. The target PR was not merged or authorized; the issue remained open and the broken base source remained unchanged. The authority returned `invalid_review_base_binding`. All five reservations resolved; 12,658 ms of the 1,200,000 ms mission budget was charged. No model was invoked. Composed-run SHA-256: `331e91e23965b9ea619938fc23dd5a752d64ce8f1bbca9d3a8530f2d0ae88dd7`.

All four internal manifests for runs 049/050 and their admission evidence passed `sha256sum -c`. The selected composed and admission provenance files are included in the top-level [evidence manifest](SHA256SUMS).

## Limits and continuation

These are two runs of one small fixture with a deterministic local adapter. They do not demonstrate an LLM-backed path or improved project success, cost, tokens, or human intervention. Claude PID `1947069` remained present (`Ssl+`) in the last metadata-only observation; CPU rose by 2 seconds over 6 minutes 56 seconds. These metrics do not establish progress or blockage. No prompt, argv, environment, log, or transcript was read; the process was not contacted, stopped, or restarted. No new model call was launched because the previous reported Claude spend had exceeded its estimate and the mission requires confirmation before new spend. The existing 30-minute heartbeat remains the automatic continuation mechanism; no duplicate was created.

The local sandbox and services share the host UID, and Forgejo’s merge API still lacks an atomic expected-base compare-and-swap. These runs demonstrate refusal when the competing advance is visible before authority evaluation, not protection against every non-cooperating same-UID process or a writer racing inside the final API window.
