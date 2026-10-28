# FORGEJO-RECONCILE-CURRENT-BASE-REPOSITORY-001

Date: 2026-10-25

## Result

The reconciler now checks the optional repository identity nested in a current Forgejo pull-request response at base.repo. When full_name, name, or owner.login is present and conflicts with the configured PR repository, reconciliation returns ResourceMismatch rather than a canonical snapshot. A malformed non-object base.repo returns InvalidCurrentStateField("base.repo"). The check does not compare head.repo, so a normal pull request from a contributor fork remains valid. Missing or null base.repo metadata stays compatible with existing/partial snapshots; base SHA remains independently required.

## Test-first evidence

The new contract test first failed on the intended behavior: with the requested PR endpoint, number, html_url and SHAs all matching, the fixture returned a foreign base repository and the reconciler returned Ok(ReconciledState) instead of ResourceMismatch. The red output is in red.log.

After the runtime change, the final focused test passes and covers six response variants:
1. foreign base full_name is rejected;
2. foreign owner.login without full_name is rejected;
3. conflicting name despite a matching full_name is rejected;
4. malformed base.repo shape is inconclusive/actionable;
5. matching configured base with a contributor fork head is accepted;
6. null base.repo remains compatible.

Each variant also asserts that the adapter reads only the configured repository's PR route. See focused-final.log.

The full Rust target suite was run with Cargo offline/locked: 532 passed, 0 failed, 66 ignored, 34 test groups. The ignored tests are explicitly marked for specialized Bubblewrap/endpoint execution and were not run by this command. Full output is in all-targets.log. Rustfmt, git diff --check, and the Cargo-license state tests (7/7) pass.

## Source basis and scope

Forgejo official source was checked from the pinned HEAD in /tmp/forgejo-v17-base-cas-poc:
- remote https://codeberg.org/forgejo/forgejo.git, HEAD 0d791421f85ca13a63b665053d81ea765fbbc571 (v17.0.0);
- modules/structs/pull.go:46-47 declares Base and Head as *PRBranchInfo;
- modules/structs/pull.go:64-71 declares PRBranchInfo.Repository as *Repository.

The source checkout is dirty with separate experimental POCs; the cited source was read through git show HEAD. No Forgejo code was copied or modified by this product change. See source-review.txt for blob IDs and commands.

This is runtime behavior in the local agent-forge Rust reconciler, tested against a deterministic FixtureApi. It does not exercise a live Forgejo instance or a real network response. No LLM, AgentOS adapter, RAG/memory service, credential, paid model, or external service was used. The earlier process-level ingress test separately verifies a foreign webhook base repository is rejected before durable inbox append; this tranche adds current-state response verification after the configured API read.

Cargo metadata was freshly read and remains license=null, license_file=null, publish=[], decision no_license_selected, redistribution rights not_assessed. No license or redistribution permission was selected or inferred. This is one local adapter/reconciliation slice, not a complete agent-to-controlled-merge path or a comparative value result.
