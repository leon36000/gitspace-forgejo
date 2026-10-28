# FORGEJO-FORK-PR-BASE-SCOPE-001

Date: 2026-10-25

## Result

The local pre-merge adapter now binds a pull request snapshot to the configured target repository through the PR URL and base repository, while allowing head.repo to identify a contributor fork.

Before the parser change, PullRequestSnapshotV1::from_forgejo_response required both base.repo.full_name and head.repo.full_name to equal the configured target. That rejected the normal fork workflow described by the architecture: an agent pushes its candidate on a fork and opens a PR against the target. No fork-specific approval or merge privilege was added; the adapter only stops rejecting the source repository because it differs.

The parser contract keeps the refusal for a base repository that differs from the configured target. Existing base SHA, head SHA, pull-request number, and canonical HTML URL checks remain in place.

## Test-first and process evidence

The parser contract was run first against the old implementation. It failed because a PR with the configured target base and a contributor fork head returned WrongScope. See red.log. After the parser change, forgejo_merge_preflight passes 13/13 and the focused contract passes 1/1 in green-final.log.

A new ignored process test exercises the real read-only forgejo-reconcile-local CLI against a loopback HTTP fixture. The runner stages the current Cargo source and locked registry in a temporary directory, starts Bubblewrap with --unshare-net and --unshare-pid, then invokes the test and CLI inside that namespace. With base.repo set to forge-admin/agent-forge-core and head.repo set to contributor/agent-forge-core, the process returns proof_current, reports merge_authorized=false and merge_attempted=false, performs exactly one GET to the configured target PR path, and sends no Authorization header. The focused sandbox run passes 1/1; exact runner output, source-stage hash, toolchain, and Bubblewrap command are in cli-process-final.log.

A second ignored process test exercises the real CLI refusal path when base.repo names a foreign repository while the head is on a contributor fork. The process returns inconclusive/wrong_scope, performs only the single GET to the configured target PR path, and sends no Authorization header; no merge authorization or attempt is reported. It passes 1/1 under the same network/PID-isolated runner; output is in cli-foreign-base-process.log. This tests the existing parser refusal at the process boundary and does not change runtime code.

Before the second process case was added, the full Rust all-target suite exited 0: 532 passed, 0 failed, 67 ignored across 34 groups. After adding that ignored process case, a fresh full-suite attempt failed in the unrelated pre-existing cancellation test `mcp_supervisor_cancellation_terminates_the_in_flight_verifier_scope` (`tests/mission_012_mcp_lifecycle.rs:1728`): 27 passed, 1 failed, 1 ignored in that test group. It observed PID 646255 in the transient verifier cgroup after its two-second cleanup grace. Immediately after the test panic, `/proc/646255` was absent. A separately isolated rerun of the exact test passed 1/1 in 15.79 seconds (`mcp-cancel-isolated.log`); the whole `mission_012_mcp_lifecycle` target then passed 28 tests with 1 ignored (`mcp-lifecycle-module.log`). A subsequent full all-target retry passed: 532 passed, 0 failed, 68 ignored across 34 groups (`all-targets-retry.log`). Earlier evidence also records this test's intermittent cgroup cleanup observation and successful isolated/full reruns (`evidence/forgejo-native-branch-create-001/diagnostics.md`). The one failed full run remains preserved as `all-targets-latest.log`; the precise cause of the intermittent cgroup observation is not established. No runtime cancellation code was changed.

Rustfmt and `git diff --check` passed for the preceding fork parser work; this turn's new test source was formatted with rustfmt before its process run. The final checks are rerun and logged after this dossier update.

## Source and evidence

The product parser is src/forgejo.rs::PullRequestSnapshotV1::from_forgejo_response. Contract: tests/forgejo_merge_preflight.rs::api_snapshot_parser_binds_target_base_and_allows_fork_head. Process test: tests/forgejo_preflight_process_e2e.rs::actual_preflight_cli_accepts_target_base_with_fork_head_without_writes. Runner: scripts/run_forgejo_preflight_cli_e2e.py.

Forgejo official v17.0.0 source was checked at commit 0d791421f85ca13a63b665053d81ea765fbbc571 in /tmp/forgejo-v17-base-cas-poc. PullRequest has Base and Head as PRBranchInfo; PRBranchInfo has Repository *Repository. The scratch checkout is dirty with unrelated experiments; declarations were read from git show HEAD. No upstream code was copied or modified.

## Scope and limits

These are real local CLI processes in a network-isolated namespace, but their Forgejo endpoint and verifier proofs are fixtures. They are not a live Forgejo PR, permission check, actual fork push, or merge. They do not prove Forgejo authorizes this contribution or that the full user/agent lifecycle works.

No LLM, AgentOS/model integration, RAG/memory service, credential, external service, or paid model was used. No new access was granted. The Cargo package remains license=null, license_file=null, publish=[], decision no_license_selected, redistribution rights not_assessed. No product license or redistribution rights were selected or inferred.
