# CONTINUATION-049 — CLI merge gate refuses a competing PR before API reads

Date: 2026-10-26. Product worktree HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (`local/WEBHOOK-005-canonical-links`), still dirty. No commit was created.

## Contract exercised

Seed the durable journal with a nonterminal merge intent for PR 24 from a separately admitted issue mission on `forge-admin/mission-012-clamp:main`. Then launch the actual `forgejo-reconcile-local --merge-local-test` process for PR 23 with its own valid admission, policy, proof, review binding and synthetic host credential pipe. The test uses distinct mission and PR identities on the same configured target branch. The gate must identify PR 24, return a non-success decision, and make no Forgejo API or merge request.

## Result

The ignored process integration test `unresolved_merge_on_same_base_refuses_another_pr_before_forgejo_reads` passes 1/1 through `scripts/run_forgejo_mission_delivery_process_e2e.py` under Bubblewrap 0.9.0 with `--unshare-net` and `--unshare-pid`. Assertions verify `verdict=inconclusive`, `retryable=true`, code `execution_merge_target_unresolved`, requested PR 23, blocking PR 24, and an action requesting authenticated readback. The fixture observes zero requests; no merge occurs. The journal retains one started blocker intent and zero resolution events.

The end-to-end test exposed a status-mapping defect in the new scope fence: an unresolved competing PR was reported as terminal `rejected`. The three scope-fence errors now set `retryable=true`, which the real CLI maps to `inconclusive`. The recovery action now explicitly says to use authenticated readback. The journal contract suite passes 66/66 with 2 intentionally ignored; formatting, tracked diff whitespace, Python syntax, Cargo metadata and the Cargo license guard all pass. Cargo metadata still reports no package license, no `license_file`, `publish=[]`, `no_license_selected`, `NOASSERTION`, and redistribution rights `not_assessed`.

## Evidence and source anchors

- `final-run.log`: the dedicated staged, offline Bubblewrap runner, including source tree, Cargo lock, runner and registry hashes, toolchain versions, and test output.
- `journal-suite.log`, `cargo-fmt-check.log`, `diff-check.log`, `untracked-source-whitespace.log`, `py-compile.log`, `cargo-license-verify.log`, and `cargo-metadata.json`: scoped final checks.
- `first-attempt.log`: the first fixture setup was rejected because requirement `source_issue_number=1` did not match blocker issue 2. The final fixture generates a separate requirement with the correct source issue and digest; no product failure was inferred from that setup error.
- `src/agent_runtime.rs`: the scope error classification and actionable readback text, around `with_merge_attempt` (lines 2375–2428) and `recover_merge_attempt_if_present` (around 2590).
- `tests/forgejo_merge_post_crash_recovery_process_e2e.rs`: the real-process refusal test around line 1434.
- `scripts/run_forgejo_mission_delivery_process_e2e.py`: the new named test registration; the runner stages only Cargo inputs and uses a network-isolated namespace.

## Limits

Forgejo in this test is an HTTP listener inside the isolated test namespace, not a Forgejo server instance. This proves the product CLI reaches the local journal fence before sending requests; it does not prove server-side CAS, exclusion of direct external writers, real Forgejo compatibility, LLM/AgentOS behavior, or comparative value. The process test remains ignored by generic Cargo runs and is covered only when the dedicated runner selects it. No upstream Forgejo or GitSpace code was copied, no product license was selected, and no external service, paid model, production instance or user secret was used.
