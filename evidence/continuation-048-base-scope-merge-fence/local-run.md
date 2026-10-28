# CONTINUATION-048 — unresolved merge intents fence a base-branch scope

Date: 2026-10-26. Worktree HEAD: `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0` (`local/WEBHOOK-005-canonical-links`). The relevant runtime and journal contract test are untracked files in the already-dirty worktree; they were preserved and not committed.

## Question and finding

Does an inconclusive merge intent for one pull request prevent a second mission from merging a different pull request into the same configured base branch? The pre-change test answered no: PR 27 had an unresolved intent, yet PR 28 returned a merge resolution and both callbacks were reached. The pre-fix test command and observed result are recorded in `commands.txt`; its raw console log was not saved at the time.

The existing merge scope digest binds origin, owner, repository and branch in `src/agent_runtime.rs` (`merge_scope_sha256`, around line 2700). `unresolved_merge_attempt_for_scope` now finds a nonterminal started attempt for that digest. `with_merge_attempt` checks it before preparation and repeats the check in the same journal transaction that appends a new started intent. `recover_merge_attempt_if_present` rejects attempts for another PR when no matching durable intent exists. The error is `execution_merge_target_unresolved` and identifies both the requested and blocking PR numbers. A readback that remains inconclusive keeps the scope fenced; a terminal readback lets a later attempt proceed.

The second transactional check covers competing missions that both observed an empty scope before preparing: read-only preparation may overlap, but only one can append an intent and enter its merge effect. The competing caller receives an actionable refusal before its effect. The deterministic barrier test asserts one started record, one effect, one refusal, and matching blocking/requested PR numbers.

## Evidence

- `journal-suite.log`: the whole `mission_execution_journal_contract` integration test, 66 passed, 0 failed, 2 ignored.
- `concurrency-repeat.log`: the competing-PR barrier test passed 20/20 executions.
- `cargo-fmt-check.log`, `diff-check.log`, `untracked-source-whitespace.log`, and `cargo-license-verify.log`: all checks passed. The metadata guard reports `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, and `NOASSERTION`.
- `cargo-metadata.json` and `toolchain.txt`: offline metadata and local toolchain details.
- Source anchors: `src/agent_runtime.rs` lines 2363–2444 (precheck and atomic append), 2580–2603 (cross-PR recovery refusal), 2700–2708 (scope digest), 3007–3034 (scope intent scan); `tests/mission_execution_journal_contract.rs` lines 3310 onward (serial recovery lifecycle) and 3493 onward (concurrent competing PRs).

## Limits

This proves behavior of the local journal and trusted callback boundary. It does not prove an instance-level Forgejo lock, block direct Forgejo API/Web/receive-pack writers, or provide atomic expected-base semantics at the server. It uses fixture callbacks, not Forgejo or an LLM. The logical branch fence lasts only while a nonterminal intent remains in this journal; external writers remain outside its authority. No upstream Forgejo/GitSpace code was copied or modified, no license was selected, and no external service, paid model, production instance, secret or privileged credential was used.
