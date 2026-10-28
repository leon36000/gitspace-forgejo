# Forgejo v17 writer-fence and LFS recovery audit — 008

Date: 25 October 2026. All experiments remained in /tmp/forgejo-v17-base-cas-poc, whose Git base is the official Forgejo v17.0.0 commit recorded in source-and-toolchain.txt. This scratch is dirty and aggregates unrelated prior POCs; the tests therefore characterize that experimental tree, not pristine upstream behavior. No Forgejo code was copied into the product and no fork was adopted.

## Question and result

Can an opt-in expected-base receive hook enforce a required compare-and-swap precondition for every writer, while staged LFS metadata remains safely recoverable?

No. The receive policy is a branch pre-receive check. The route tests show its compatibility and coverage boundaries: API branch creation without a source-base contract fails closed with an opaque 500; the browser create flow displays a generic refresh/merge-path message. A stale API branch deletion succeeds despite an expected_base_commit_id query parameter, and Web branch rename/delete succeed while the policy is enabled. An earlier scratch integration test POC-076 additionally showed API branch rename bypassing the receive hook. The hook source does not call the assertion for tag updates; the wiki path is explicitly exempt; GITEA_INTERNAL_PUSH=true skips ordinary hooks. Generic content writers call PushingEnvironment without the expected-base token, while merge/rebase server paths explicitly attach the trusted trigger and expected base. These source observations are scoped to the named paths; they are not an exhaustive inventory of every writer.

The recovery helper is also not yet safe to promote from a read-only ref observation. The barrier test TestPOCRecoveryPromotesFromARefSnapshotThatIsStaleBeforeDBPromotion moves the target ref after the helper observes candidate reachability and before database promotion. It passes by asserting that the stale observation still causes promotion after the ref no longer contains the candidate. Static call-site search finds only the recovery helper definitions in non-test Go code; no startup/periodic caller is registered in this scratch. The checkpoint/page helpers support bounded manual replay, not demonstrated autonomous recovery.

## Commands and evidence

Both fresh commands exited 0; full output, exit codes, scratch source hashes and toolchain are beside this report:

- Four integration tests: API/Web branch creation, stale API deletion, and Web rename/delete under the opt-in policy.
- One tagged services/pull test: stale reachability observation followed by target-ref movement before promotion.
- GOTOOLCHAIN=local, GOPROXY=off; pinned Go go1.27.3 linux/arm64.

The positive Go test result for the recovery race is an adverse system result: it confirms the helper promotes after its ref observation has gone stale. Passing means the test reproduced the hazard; it does not mean the behavior is acceptable.

## LFS publication implications

The earlier POC-071 exercised only generic ChangeRepoFiles HTTP acknowledgement and direct manual recovery after an injected SQLite insert failure. POC-072 reproduced unconditional service-level promotion after a stale hook observation. In this scratch, post-receive generic promotion performs a ref read and then a database promotion; the service-level caller can promote after Push, and PR merge/rebase add their own promotion paths. A read-then-promote check alone is not a fence shared with Git, API/Web branch writers, internal pushes, SQL PR state and LFS metadata. Pending rows can be retained and replayed idempotently, but neither the recovery worker nor safe discard/publication coordination is established here.

## Compatibility and maintenance decision

A mandatory server precondition would require a versioned contract for every write surface, deliberate handling for create/delete/rename, file APIs and Web edits, tags/wiki, receive-pack clients, internal jobs, merge/rebase/AutoMerge and mirrors/imports, plus a shared serialization/fencing point for ref and SQL/LFS side effects. The current receive hook is insufficient. The combined experimental diff in eight relevant tracked files is +552/-35 lines. This is an aggregate of multiple scratch POCs, not an attributable patch-size estimate. It gives a concrete review surface but not a reliable maintenance-hours estimate; callers span HTTP routes, repository services, private hooks, pull services, Git environment, and LFS model/storage. Compatibility cost is material and not yet measured quantitatively.

Decision: keep Forgejo as the local Git/PR substrate and keep the agent layer outside its source. The current interface-based pilot is valid only behind the existing constrained local merge gate; it is not a multi-writer safety guarantee. Do not adopt this scratch fork. A future server-side capability must start with an inventory of all ref writers and one fencing/intent protocol whose recovery is tested across process interruption and concurrent updates; otherwise direct writer access must be excluded by a demonstrated boundary.

## Limits

This is a targeted, source-level and integration-test audit against a globally modified scratch tree based at v17.0.0. It does not establish pristine-upstream behavior, exhaustive writers, production isolation, multi-database/storage behavior, automatic restart recovery, or a safe shared ref/SQL/LFS transaction. No LLM/AgentOS, user-level full mission, or comparative benchmark is covered. Exact sources, hashes, commands and raw results are in this directory.
