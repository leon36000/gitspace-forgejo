# CONTINUATION-347.1 — Forgejo issue label events live

Date: 2026-10-28. Local source worktree: `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branch `local/WEBHOOK-005-canonical-links`, base commit `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Forgejo PC4 migration remains independently blocked; these tests ran only against a disposable loopback Forgejo instance.

## Change

The emitted-webhook runner now accepts an issue’s native `label_updated` and `label_cleared` actions, validates the bounded label id/name snapshot, binds Forgejo task type `issue_label` to the `issues` wire header, and offers `--change-issue-labels`. The live case creates one disposable issue label, adds it, removes all issue labels, confirms the current issue API after each mutation, and reconciles both journaled deliveries independently. Event-count offsets now account for the two extra issue events throughout the rest of the scenario.

## Evidence

Forgejo binary `/tmp/forgejo-17.0.0` reports `17.0.0+gitea-1.22.0`, built with Go `1.26.9`; SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

- `forgejo-run-002/result.json` has verdict `passed` and nine delivered webhook events/task rows. The two issue label tasks are `issue_label`, each delivered with HTTP 202.
- Sequence 2 is `issues/label_updated`: its payload and current API both contain the disposable label. Sequence 3 is `issues/label_cleared`: its payload still contains the prior label, while the current issue API returns no labels.
- Both label deliveries reconcile with exit status 0, `current_state_read`, one upstream read, resource `issue #1`, and canonical labels `{items: [], source: current_forgejo_resource_api, freshness: current_api_read, trust: untrusted_user_content}`.
- The issue is later closed and final-read-back with labels still empty. The run confirms webhook deactivation, Forgejo and receiver process exit, and absence of their checked listeners.
- `forgejo-run-001` is retained as a diagnostic failure. Its issue label operations emitted the expected six issue events; the runner then failed before PR reconciliation because a hard-coded PR event count still assumed no issue labels. The corrected count offsets pass in run 002. This failure is not reported as a successful end-to-end run.

## Tests

- TDD red: `issue-label-python-red.log` recorded the missing parser actions, `issue_label` task binding, clear-snapshot validation, and CLI flag.
- Focused Python contracts and CLI checks: 29/29 passed after implementation.
- Full `tests.test_forgejo_emitted_webhook_e2e`: 143/143 passed after the event-count fix.
- Python AST parsing of the runner and test module passed; `git diff --check` passed.
- The Rust source did not change; no Rust suite was rerun for this Python harness slice.

## Limits

This is one local Forgejo 17.0.0 run (`n=1`) with a synthetic repository and disposable data. It establishes live emission and current-state reconciliation for these two issue label transitions only. It is not a GitLab migration, an AgentOS/LLM run, a code-change PR/merge, a benchmark, or evidence of general improvement.
