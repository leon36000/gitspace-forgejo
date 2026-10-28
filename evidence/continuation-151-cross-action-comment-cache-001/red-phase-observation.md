Red phase observed during this turn (terminal output, not a separately captured raw log):

Command: `cargo test --offline --locked --test webhook_reconcile_contract batch_reconciliation_reuses_comment_read_across_create_delete_and_parent_issue -- --exact`

Exit 101. The assertion observed three upstream reads: Issue #11, IssueComment #29 with `deleted_event=false`, and the same IssueComment #29 with `deleted_event=true`. The expected calls were only the Issue and one comment endpoint. After key normalization, the exact test passed in both event orders.
