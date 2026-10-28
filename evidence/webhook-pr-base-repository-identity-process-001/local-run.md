# WEBHOOK-PR-BASE-REPOSITORY-IDENTITY-PROCESS-001

Date: 2026-10-25

## Result

The webhook parser now binds a pull request's declared base repository to the delivery's configured root repository. It still accepts a foreign head.repo, as required for a normal fork contribution. Where the older/partial payload omits pull_request.base or base.repo, this check remains backward-compatible and does not reject solely for that omission.

The check runs in parse_resource_ref before a webhook is appended to the durable inbox. The same parser is used during reconciliation/replay, so conflicting base identity is rejected there as well. No Forgejo source was copied.

## Upstream contract inspected

The official repository remote is https://codeberg.org/forgejo/forgejo.git. Its scratch checkout HEAD is Forgejo v17.0.0 commit 0d791421f85ca13a63b665053d81ea765fbbc571. This is the source revision inspected; the scratch checkout is dirty with unrelated experimental POC changes, so source claims below were read with git show HEAD:<path>.

At that commit:
- modules/structs/hook.go:358-365 defines PullRequestPayload with both PullRequest *PullRequest and root Repository *Repository.
- modules/structs/pull.go:46-47 defines PullRequest.Base and PullRequest.Head as *PRBranchInfo.
- modules/structs/pull.go:64-71 defines PRBranchInfo.Repository *Repository.
- The upstream repository LICENSE begins with GNU GPL version 3. These facts describe the inspected upstream source; no upstream code was incorporated into this product change.

Commands and captured source/toolchain state are in commands.txt, source-review.txt, source-state.txt, and toolchain.txt.

## Test-first evidence

Before the runtime validation was added, the new true-receiver process test failed as expected: the signed payload with the configured root repository but a different pull_request.base.repo received HTTP 202 instead of the expected HTTP 400. See red.log.

After adding the parser check, the focused process test passed 1/1. It sends a matching base repository with a contributor fork as head.repo and confirms acceptance (HTTP 202). It then sends a signed payload with a conflicting base repository and confirms HTTP 400, event_payload_invalid, retryable=false, no inbox append, and replay containing only the accepted event. See focused-green.log.

Relevant suites after the runtime change:
- webhook_contract: 9 passed, 0 failed.
- webhook_reconcile_contract: 35 passed, 0 failed.
- webhook_receiver_process_e2e: 18 passed, 0 failed, 51 ignored. The ignored tests require their own fixed-endpoint Bubblewrap invocation; this ordinary suite does not claim they ran.

The current focused run uses Cargo offline/locked. rustfmt --check, git diff --check, Cargo license-state tests 7/7, and a fresh license metadata readback passed. Full outputs are preserved in this directory.

## Scope and limits

This is an actual runtime validation change in the local agent-forge Rust receiver/parser and an actual receiver-process test, not a live Forgejo webhook run. Requests are synthetic HMAC-signed fixtures sent to a loopback process. No Forgejo instance, LLM, AgentOS adapter, RAG/memory service, external network, credential, or paid model was used. The test checks durable inbox non-mutation on rejection and a permitted fork head; it does not prove the behavior of every webhook shape or a full agent-to-merge lifecycle.

The Cargo package remains license=null, license_file=null, publish=[], with no_license_selected and redistribution rights not_assessed. No rights are inferred. The project remains incomplete; this tranche does not establish comparative project success, cost, or cycle-time improvement.
