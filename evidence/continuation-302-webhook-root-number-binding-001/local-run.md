# CONTINUATION-302 — Issue/PR root-number binding

Date: 2026-10-27. Product worktree base: f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0, already heavily modified before this lot. Rust and Cargo: 1.95.0, Linux/aarch64. No commit, reset, cleanup, external service, production change, model call, or new access was used.

## Finding and upstream reference

The event parser derived an issue or pull request resource identifier from only the nested object's number. Forgejo v17.0.0 also emits a root-level number on IssuePayload and PullRequestPayload. A signed payload with inconsistent duplicate identifiers was therefore accepted, with the parser silently ignoring one value.

The inspected upstream checkout is /tmp/forgejo-v17-base-cas-poc, exactly tag v17.0.0, commit 0d791421f85ca13a63b665053d81ea765fbbc571. This is source inspection only; the scratch tree is dirty from unrelated experiments. Targeted source references:

| Upstream file and symbol | Relevant source | Git blob | SHA-256 |
|---|---|---|---|
| modules/structs/hook.go, IssuePayload | lines 322–332: root Index tagged json:number and nested Issue | 9c7f1deed1c38eeb435ab976bb8429c5458c7f2b | 42f951d98dbea6b78e70e595b8c2cc0f5931bee2d96b837d8d4e8f034b015a12 |
| modules/structs/hook.go, PullRequestPayload | lines 358–370: root Index tagged json:number and nested PullRequest | same file/blob | same file hash |
| modules/structs/issue.go, Issue | lines 46–54: nested Index tagged json:number | 07ff3079de602403c23dfb2530ff5ee46fb0e747 | e6cb7890f1d7045b2423465d1e452b6f91c4d71c746957aefc9bade286d10006 |
| modules/structs/pull.go, PullRequest | lines 11–18: nested Index tagged json:number | 1ce7550e197752d5065283bab01bc7da98dfb901 | 1472f689d9e80da512cc8edafa5d647daafccdf060dfba0d4a74d3be5cfe06e3 |
| services/webhook/notifier.go | issue assignment/label/milestone constructors populate Index from issue.Index and the converted nested issue/PR | 5aea521a0f2cb5d7513c98e9805d10e6a6b2901b | a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f |

This establishes the declared wire fields and inspected constructors, not a live upstream webhook delivery. No Forgejo code was copied.

## Change and red/green evidence

src/webhook/reconcile.rs, parse_resource_ref now checks the optional root number for issues and pull_request. A present root value must be a positive unsigned integer and equal the nested number; zero or a wrong type returns InvalidEventPayloadField("number"), and disagreement returns ResourceMismatch. If the root key is absent, legacy payloads remain accepted. This avoids turning a compatibility observation into a newly required field.

The new regression issue_and_pull_request_root_numbers_must_match_nested_numbers first failed before the fix: root 42 plus nested issue 41 returned an Ok ResourceRef with Number(41). The captured assertion is red.log. After the fix, the same test passes for equal and conflicting issue/PR pairs and rejects root zero.

The signed webhook library-handler test issues_ingestion_requires_a_supported_forgejo_action_without_journaling_invalid_payloads now also sends root 42 / issue 41. It observes HTTP 400, an actionable “not journaled” response, and an empty journal. This invokes the actual signature verification and ingestion handler function, but not the standalone receiver executable.

## Verification

Final post-format command: cargo test --locked --offline --test webhook_reconcile_contract --test webhook_handler_contract — 110 passed, 0 failed, 0 ignored (81 reconcile-contract tests, 29 handler-contract tests). Full output: targeted-tests.log.

- cargo fmt --all -- --check: pass, rustfmt.log
- git diff --check: pass, diff-check.log
- python3 scripts/verify_cargo_license_state.py: pass; the readback remains license=null, license_file=null, publish=[], redistribution rights not_assessed, SPDX NOASSERTION; license-state.json

## Scope and limits

This is parser plus in-process HTTP-handler coverage with a temporary journal, synthetic body, and synthetic HMAC secret. No binary receiver process, actual Forgejo instance/delivery, LLM/AgentOS, task execution, PR, verification/merge, or comparative baseline ran here. Historical payloads that omit the root field remain accepted by design. A single contradictory-payload bug fix is not evidence of general project success or productivity gain.
