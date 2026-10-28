# MISSION-012 Runner Boundary Hardening 001 — Local Verification

Date: 2026-10-23 (UTC)
Repository HEAD inspected: f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0
Python: 3.12.3

## Scope

This tranche hardened the local composed webhook → admission → MCP/Claude → Forgejo PR harness after a read-only review found credential exposure during Cargo builds, process cleanup gaps, a missing direct-push policy check, and unbounded child output capture. Follow-up read-only reviews identified signal-handling and snapshot-copy races; the final concrete race was fixed and the final read-only pass found no residual issue in the reviewed boundary.

Changes are in [the admission runner](../scripts/run_mission_012_live_admission_e2e.py), [the composed runner](../scripts/run_mission_012_live_composed_e2e.py), [the PR runner](../scripts/run_mission_012_live_agent_pr_e2e.py), [the MCP agent runner](../scripts/run_mission_012_live_mcp_agent_e2e.py), [the MCP supervisor](../scripts/mission_012_mcp_supervisor.py), and their four focused test files under tests/.

## Implemented

- Cargo builds receive an explicit source snapshot: manifests, lockfile, README/build inputs, source, tests and policy. The examined project has no .cargo directory, so Cargo config/credentials are excluded. Descriptor-relative opens refuse symlinks, hard links and special files, anchor writes to opened destination directories, and reject a destination entry replaced during copy. A before/after source-tree metadata snapshot rejects a mixed revision if any approved input changes during copying. .runtime is omitted from the build snapshot.
- Stage processes receive a small environment allowlist (HOME, fixed PATH, locale, terminal and color settings); a test injects a sentinel secret and private host PATH and confirms neither is inherited.
- The webhook receiver and PR fixture stay in the composing stage group. The MCP relay handles SIGTERM before launching its host; the trusted host runs in a separate process group so cancellation of Claude/relay cannot kill it before finalization. The relay closes host stdin, drains its final output, waits for finalization and kills the host group if its bounded grace expires. The stage runner handles a child that closes stdout but remains alive and terminates its process group on timeout.
- The Forgejo branch protection read-back requires enable_push is False.
- Claude stdout and stderr are streamed under one combined byte limit, with bounded in-memory captures. A signal-unblocking shim protects the process-creation race. The temporary SIGTERM handler only records cancellation and remains non-throwing through cleanup; the CLI process group is always drained after normal exit, timeout, overflow, parent cancellation, or another exception. The trusted MCP host stays in its own process group and finalizes through the relay. Tests prove host finalization and exercise SIGTERM exactly as group cleanup begins, with a live redirected-output descendant.

## Verification

On the dirty local worktree at the HEAD above:

- python3 -m unittest discover -s tests -p 'test_mission_012_*.py' -v — 34 tests passed, 20.349 seconds. The retained transcript is [the focused test log](mission-012-runner-boundary-hardening-001-focused.log).
- python3 -m py_compile on the five changed scripts and four test files — passed.
- git diff --check — passed for tracked changes; a separate newline/trailing-whitespace scan passed on all nine runner/test files.
- A disposable copy_project smoke check against the current repository copied the approved source/policy inputs, omitted .cargo, and created an empty .runtime directory — passed; [transcript](mission-012-runner-boundary-hardening-001-copy-smoke.log).
- The cleanup-entry SIGTERM regression fails against a staged throwing-handler variant and passes against the current non-throwing handler; [red transcript](mission-012-runner-boundary-hardening-001-cleanup-sigterm-red.log) and green result in the focused test log.
- Earlier red evidence for destination replacement, source mutation and parent cancellation: [snapshot red](mission-012-runner-boundary-hardening-001-snapshot-red.log), [host-group red](mission-012-runner-boundary-hardening-001-host-group-red.log), [parent-cancellation red](mission-012-runner-boundary-hardening-001-parent-cancel-red.log). Final green results are in the focused test log.
- Final source, test, report and log hashes are listed in [the SHA-256 manifest](mission-012-runner-boundary-hardening-001.sha256).

These are focused local checks, not a rerun of the composed Forgejo/LLM E2E. No Rust build, live Forgejo request, Claude task, PR or merge was run after these hardening edits. The earlier full LLM/Forgejo run remains separate evidence in [the prior composed-run report](mission-012-live-agent-webhook-compose-005-local-run.md); it does not validate this new code. Test children are controlled fixtures and do not demonstrate isolation from a hostile same-UID process.

## Claude CLI attempt and independent review

The configured claude CLI (2.1.309) ran the official Ralph Loop plugin (1.0.0) against a sanitized staging copy containing selected code/tests, with Opus 5.5, --effort max, and --max-budget-usd 0.20. It stopped before reading or changing the code and before running tests or a review. Its final JSON event reported subtype=error_max_budget_usd, exit code 1 and total_cost_usd=0.5258432 (USD 0.3258432 above the requested cap). This amount is CLI-reported and is not independently attested by billing. No subsequent paid model call was made; a Sonnet 5.5 CLI review was not run.

A native read-only reviewer initially found four additional P2 issues: group cancellation could kill the MCP host before it finalized; a previously blocked SIGTERM mask could prevent cancellation; destination writes could follow a replaced directory; and the copied files could combine different source revisions. Tests and fixes followed. Later passes found a cleanup-entry SIGTERM exception race and then a signal-disposition gap plus a regression test that did not guarantee a live child. The final change keeps the handler non-throwing during cleanup, always terminates the CLI group, and restores the caller's mask and handler after process and stream cleanup. The revised test proves its descendant is already running and uses a late marker to demonstrate it cannot outlive cleanup. The final native read-only pass found no residual issue in this scope; it did not execute tests.

## Limits and next work

- Process groups and descriptor-based snapshots do not isolate hostile code sharing the same UID. A child can deliberately create another session; a same-UID writer may replace paths after snapshot creation or mutate the finished snapshot before Bubblewrap opens it.
- The runner's filtered environment does not prevent processes running as the same user from reading files that user can access. It is not a secret vault or general execution sandbox.
- Forgejo's merge API still lacks an atomic base/head precondition; the read-back gate has a race against a non-cooperating writer. This tranche does not change that behavior.
- AgentOS, Forge Code, RAG and memory are still not integrated, and no comparative productivity, regression, intervention, latency or token study is established.

Next: test the known Forgejo base/head freshness race around merge, interruption and resumption mid-pipeline, and semantic conflicts under concurrent changes. A comparable Forgejo+agent+CI baseline is still needed before claiming value. Do not treat the earlier n=1 LLM success as a general gain or rerun a paid model task under the overshot cap.
