use crate::{
    AppendResult, EventJournal, JournalAttestationError, JournalError, JournalEvent,
    verify_journal_event_attestation,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hmac::{Hmac, Mac};
use serde::Serialize;
use serde_json::json;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub mod http;
pub mod reconcile;

pub const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;

const ISSUES_EVENT: &str = "issues";
const ISSUE_COMMENT_EVENT: &str = "issue_comment";
const PULL_REQUEST_EVENT: &str = "pull_request";
const PUSH_EVENT: &str = "push";
const CREATE_EVENT: &str = "create";
const DELETE_EVENT: &str = "delete";
const RELEASE_EVENT: &str = "release";
const PULL_REQUEST_APPROVED_EVENT: &str = "pull_request_approved";
const PULL_REQUEST_REJECTED_EVENT: &str = "pull_request_rejected";
const PULL_REQUEST_COMMENT_EVENT: &str = "pull_request_comment";
const PULL_REQUEST_REVIEW_APPROVED_TYPE: &str = "pull_request_review_approved";
const PULL_REQUEST_REVIEW_REJECTED_TYPE: &str = "pull_request_review_rejected";
const PULL_REQUEST_REVIEW_COMMENT_TYPE: &str = "pull_request_review_comment";
const WORKFLOW_RUN_BLOCKED_EVENT: &str = "workflow_run_blocked";
const WORKFLOW_RUN_CANCELLED_EVENT: &str = "workflow_run_cancelled";
const WORKFLOW_RUN_FAILURE_EVENT: &str = "workflow_run_failure";
const WORKFLOW_RUN_RUNNING_EVENT: &str = "workflow_run_running";
const WORKFLOW_RUN_SKIPPED_EVENT: &str = "workflow_run_skipped";
const WORKFLOW_RUN_SUCCESS_EVENT: &str = "workflow_run_success";
const WORKFLOW_RUN_WAITING_EVENT: &str = "workflow_run_waiting";
const WORKFLOW_JOB_BLOCKED_EVENT: &str = "workflow_job_blocked";
const WORKFLOW_JOB_CANCELLED_EVENT: &str = "workflow_job_cancelled";
const WORKFLOW_JOB_FAILURE_EVENT: &str = "workflow_job_failure";
const WORKFLOW_JOB_RUNNING_EVENT: &str = "workflow_job_running";
const WORKFLOW_JOB_SKIPPED_EVENT: &str = "workflow_job_skipped";
const WORKFLOW_JOB_SUCCESS_EVENT: &str = "workflow_job_success";
const WORKFLOW_JOB_WAITING_EVENT: &str = "workflow_job_waiting";
const WORKFLOW_JOB_REQUIRED_FIELDS: &[&str] = &[
    "action",
    "job.id",
    "job.run_id",
    "job.status",
    "job.html_url",
    "run.id",
    "run.index_in_repo",
    "run.repository.full_name",
];
const WORKFLOW_JOB_PAYLOAD_CONSTRAINTS: &[&str] = &[
    "the event header status must match the signed job.status",
    "job.run_id must equal run.id; the current job and parent run API reads are canonical",
    "job.html_url supplies only the job index binding; the current job and run API URLs are canonical, and the retry attempt may advance",
    "job names, workflow payloads, steps, and logs are not returned",
    "a workflow job status is an observation, not an independent verification verdict",
];
/// Static, versioned wire-contract details published by `/forgejo/capabilities`.
/// These describe admission and reconciliation expectations; they do not make
/// webhook payloads authoritative state. The contract event names are the
/// canonical Forgejo wire-event subset: `issue_comment` covers issue and PR
/// comments, `push` uses `refs/heads/<branch>`, `create`/`delete` use a short
/// branch name with `ref_type=branch`, and review headers use
/// `pull_request_approved`, `pull_request_rejected`, and
/// `pull_request_comment` while their body `review.type` uses the longer
/// `pull_request_review_*` values. The seven `workflow_run_*` headers bind
/// to the signed `run.status`; the seven `workflow_job_*` headers bind to the
/// signed `job.status`. Actions reads are canonical observations, never proofs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractV1 {
    pub event: &'static str,
    pub resource_kind: &'static str,
    pub required_payload_fields: &'static [&'static str],
    pub payload_constraints: &'static [&'static str],
    pub canonical_read: &'static str,
    pub not_found_behavior: &'static str,
}

/// Machine-readable allowed values for event-specific action fields on
/// non-workflow webhook events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventActionContractV1 {
    pub event: &'static str,
    pub payload_field: &'static str,
    pub allowed_actions: &'static [&'static str],
}

// Frozen action list for the published v5 contract. Keep this separate from
// the current contract so an additive compatibility fix does not silently
// change the meaning of an already versioned discovery document.
const SUPPORTED_EVENT_ACTION_CONTRACTS_V5: [WebhookEventActionContractV1; 2] = [
    WebhookEventActionContractV1 {
        event: ISSUES_EVENT,
        payload_field: "action",
        allowed_actions: &["opened", "closed", "reopened", "edited"],
    },
    WebhookEventActionContractV1 {
        event: ISSUE_COMMENT_EVENT,
        payload_field: "action",
        allowed_actions: &["created", "edited", "deleted"],
    },
];

/// Frozen action list for the published v6 contract. Keep it unchanged when
/// extending the runtime list so older discovery clients receive the same
/// values they were written against.
const SUPPORTED_EVENT_ACTION_CONTRACTS_V6: [WebhookEventActionContractV1; 2] = [
    WebhookEventActionContractV1 {
        event: ISSUES_EVENT,
        payload_field: "action",
        allowed_actions: &[
            "opened",
            "closed",
            "reopened",
            "edited",
            "assigned",
            "unassigned",
            "label_updated",
            "label_cleared",
            "milestoned",
            "demilestoned",
        ],
    },
    WebhookEventActionContractV1 {
        event: ISSUE_COMMENT_EVENT,
        payload_field: "action",
        allowed_actions: &["created", "edited", "deleted"],
    },
];

/// Current non-workflow action allowlist. Forgejo v17 maps multiple internal
/// pull-request hook types to the same `pull_request` wire header, so every
/// action from those notifier paths belongs to this contract.
const SUPPORTED_EVENT_ACTION_CONTRACTS_V7: [WebhookEventActionContractV1; 3] = [
    SUPPORTED_EVENT_ACTION_CONTRACTS_V6[0],
    SUPPORTED_EVENT_ACTION_CONTRACTS_V6[1],
    WebhookEventActionContractV1 {
        event: PULL_REQUEST_EVENT,
        payload_field: "action",
        allowed_actions: &[
            "opened",
            "closed",
            "reopened",
            "edited",
            "assigned",
            "unassigned",
            "label_updated",
            "label_cleared",
            "milestoned",
            "demilestoned",
            "synchronized",
            "review_requested",
            "review_request_removed",
        ],
    },
];

/// v9 adds the native Forgejo release actions and freezes the v7 action list.
pub const SUPPORTED_EVENT_ACTION_CONTRACTS: [WebhookEventActionContractV1; 4] = [
    SUPPORTED_EVENT_ACTION_CONTRACTS_V7[0],
    SUPPORTED_EVENT_ACTION_CONTRACTS_V7[1],
    SUPPORTED_EVENT_ACTION_CONTRACTS_V7[2],
    WebhookEventActionContractV1 {
        event: RELEASE_EVENT,
        payload_field: "action",
        allowed_actions: &["published", "updated", "deleted"],
    },
];

/// Machine-readable mapping between a workflow webhook header and the status
/// field/value the signed payload must carry. These mappings are also used by
/// the runtime reconciler; they are observations, never verification verdicts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookWorkflowStatusContractV1 {
    pub event: &'static str,
    pub resource_kind: &'static str,
    pub payload_field: &'static str,
    pub expected_status: &'static str,
}

/// Machine-readable allowed values for the generic `action` field in
/// workflow-run and workflow-job webhook payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookWorkflowActionContractV1 {
    pub resource_kind: &'static str,
    pub payload_field: &'static str,
    pub allowed_actions: &'static [&'static str],
}

/// Event-specific action values observed in Forgejo's notifier paths. Status
/// change actions are nonterminal; completion actions are terminal. New-attempt
/// notifications are emitted through a separate path and are valid for either.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookWorkflowEventActionContractV1 {
    pub event: &'static str,
    pub resource_kind: &'static str,
    pub payload_field: &'static str,
    pub allowed_actions: &'static [&'static str],
}

macro_rules! workflow_status_contract {
    ($event:expr, $resource_kind:literal, $payload_field:literal, $status:literal) => {
        WebhookWorkflowStatusContractV1 {
            event: $event,
            resource_kind: $resource_kind,
            payload_field: $payload_field,
            expected_status: $status,
        }
    };
}

pub const SUPPORTED_WORKFLOW_STATUS_CONTRACTS: [WebhookWorkflowStatusContractV1; 14] = [
    workflow_status_contract!(
        WORKFLOW_RUN_BLOCKED_EVENT,
        "workflow_run",
        "run.status",
        "blocked"
    ),
    workflow_status_contract!(
        WORKFLOW_RUN_CANCELLED_EVENT,
        "workflow_run",
        "run.status",
        "cancelled"
    ),
    workflow_status_contract!(
        WORKFLOW_RUN_FAILURE_EVENT,
        "workflow_run",
        "run.status",
        "failure"
    ),
    workflow_status_contract!(
        WORKFLOW_RUN_RUNNING_EVENT,
        "workflow_run",
        "run.status",
        "running"
    ),
    workflow_status_contract!(
        WORKFLOW_RUN_SKIPPED_EVENT,
        "workflow_run",
        "run.status",
        "skipped"
    ),
    workflow_status_contract!(
        WORKFLOW_RUN_SUCCESS_EVENT,
        "workflow_run",
        "run.status",
        "success"
    ),
    workflow_status_contract!(
        WORKFLOW_RUN_WAITING_EVENT,
        "workflow_run",
        "run.status",
        "waiting"
    ),
    workflow_status_contract!(
        WORKFLOW_JOB_BLOCKED_EVENT,
        "workflow_job",
        "job.status",
        "blocked"
    ),
    workflow_status_contract!(
        WORKFLOW_JOB_CANCELLED_EVENT,
        "workflow_job",
        "job.status",
        "cancelled"
    ),
    workflow_status_contract!(
        WORKFLOW_JOB_FAILURE_EVENT,
        "workflow_job",
        "job.status",
        "failure"
    ),
    workflow_status_contract!(
        WORKFLOW_JOB_RUNNING_EVENT,
        "workflow_job",
        "job.status",
        "running"
    ),
    workflow_status_contract!(
        WORKFLOW_JOB_SKIPPED_EVENT,
        "workflow_job",
        "job.status",
        "skipped"
    ),
    workflow_status_contract!(
        WORKFLOW_JOB_SUCCESS_EVENT,
        "workflow_job",
        "job.status",
        "success"
    ),
    workflow_status_contract!(
        WORKFLOW_JOB_WAITING_EVENT,
        "workflow_job",
        "job.status",
        "waiting"
    ),
];

pub const SUPPORTED_WORKFLOW_ACTION_CONTRACTS: [WebhookWorkflowActionContractV1; 2] = [
    WebhookWorkflowActionContractV1 {
        resource_kind: "workflow_run",
        payload_field: "action",
        allowed_actions: &["new_run_attempt", "run_status_changed", "run_completed"],
    },
    WebhookWorkflowActionContractV1 {
        resource_kind: "workflow_job",
        payload_field: "action",
        allowed_actions: &["new_job_attempt", "job_status_changed", "job_completed"],
    },
];

const WORKFLOW_RUN_NONTERMINAL_ACTIONS: [&str; 2] = ["new_run_attempt", "run_status_changed"];
const WORKFLOW_RUN_TERMINAL_ACTIONS: [&str; 2] = ["new_run_attempt", "run_completed"];
const WORKFLOW_JOB_NONTERMINAL_ACTIONS: [&str; 2] = ["new_job_attempt", "job_status_changed"];
const WORKFLOW_JOB_TERMINAL_ACTIONS: [&str; 2] = ["new_job_attempt", "job_completed"];

macro_rules! workflow_event_action_contract {
    ($event:expr, $resource_kind:literal, $actions:expr) => {
        WebhookWorkflowEventActionContractV1 {
            event: $event,
            resource_kind: $resource_kind,
            payload_field: "action",
            allowed_actions: $actions,
        }
    };
}

/// Versioned event/action compatibility derived from Forgejo v17's notifier
/// dispatch: `*_completed` runs only for terminal status; `*_status_changed`
/// runs only for a nonterminal status. New-attempt notifications use a separate
/// path and remain permitted for either status class.
pub const SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS: [WebhookWorkflowEventActionContractV1; 14] = [
    workflow_event_action_contract!(
        WORKFLOW_RUN_BLOCKED_EVENT,
        "workflow_run",
        &WORKFLOW_RUN_NONTERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_RUN_CANCELLED_EVENT,
        "workflow_run",
        &WORKFLOW_RUN_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_RUN_FAILURE_EVENT,
        "workflow_run",
        &WORKFLOW_RUN_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_RUN_RUNNING_EVENT,
        "workflow_run",
        &WORKFLOW_RUN_NONTERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_RUN_SKIPPED_EVENT,
        "workflow_run",
        &WORKFLOW_RUN_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_RUN_SUCCESS_EVENT,
        "workflow_run",
        &WORKFLOW_RUN_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_RUN_WAITING_EVENT,
        "workflow_run",
        &WORKFLOW_RUN_NONTERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_JOB_BLOCKED_EVENT,
        "workflow_job",
        &WORKFLOW_JOB_NONTERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_JOB_CANCELLED_EVENT,
        "workflow_job",
        &WORKFLOW_JOB_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_JOB_FAILURE_EVENT,
        "workflow_job",
        &WORKFLOW_JOB_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_JOB_RUNNING_EVENT,
        "workflow_job",
        &WORKFLOW_JOB_NONTERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_JOB_SKIPPED_EVENT,
        "workflow_job",
        &WORKFLOW_JOB_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_JOB_SUCCESS_EVENT,
        "workflow_job",
        &WORKFLOW_JOB_TERMINAL_ACTIONS
    ),
    workflow_event_action_contract!(
        WORKFLOW_JOB_WAITING_EVENT,
        "workflow_job",
        &WORKFLOW_JOB_NONTERMINAL_ACTIONS
    ),
];

/// Frozen v7 event contracts. Later revisions use a separate current array so
/// discovery clients can retain the exact older required-field list.
pub const SUPPORTED_EVENT_CONTRACTS_V7: [WebhookEventContractV1; 23] = [
    WebhookEventContractV1 {
        event: ISSUES_EVENT,
        resource_kind: "issue",
        required_payload_fields: &["repository.full_name", "issue.number", "action"],
        payload_constraints: &[
            "action is one of opened, closed, reopened, or edited",
            "top-level pull_request, comment, and review fields are rejected",
            "the current Forgejo issue read, not the webhook payload, is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/issues/{number}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: ISSUE_COMMENT_EVENT,
        resource_kind: "issue_or_pull_request",
        required_payload_fields: &[
            "repository.full_name",
            "issue.number",
            "action",
            "comment (object)",
        ],
        payload_constraints: &[
            "action is one of created, edited, or deleted",
            "pull_request.number, when present, equals issue.number",
            "is_pull, when present, is boolean; true requires pull_request and false forbids it",
            "the current Forgejo issue or pull request read, not the webhook payload, is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/issues/{number} or /pulls/{number}, selected by signed pull_request, then GET /issues/comments/{comment_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: PULL_REQUEST_EVENT,
        resource_kind: "pull_request",
        required_payload_fields: &["repository.full_name", "pull_request.number"],
        payload_constraints: &[
            "top-level issue and comment fields are rejected; review may be absent or null, but a non-null review field is rejected",
            "the current Forgejo pull request read, not the webhook payload, is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: PUSH_EVENT,
        resource_kind: "branch",
        required_payload_fields: &["repository.full_name", "ref", "before", "after"],
        payload_constraints: &[
            "ref uses refs/heads/<branch>",
            "before and after are 40- or 64-character hexadecimal object IDs",
            "the current Forgejo branch read, not payload.after, is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/branches/{encoded_branch}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: CREATE_EVENT,
        resource_kind: "branch",
        required_payload_fields: &["repository.full_name", "ref_type", "ref", "sha"],
        payload_constraints: &[
            "ref_type is branch",
            "ref is a short branch name",
            "sha is a nonzero 40- or 64-character hexadecimal object ID",
            "the current Forgejo branch read, not payload.sha, is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/branches/{encoded_branch}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: DELETE_EVENT,
        resource_kind: "branch",
        required_payload_fields: &["repository.full_name", "ref_type", "ref"],
        payload_constraints: &[
            "ref_type is branch",
            "ref is a short branch name",
            "sha is create-only and forbidden for delete; this cross-checks payload shape against the unsigned event header",
            "a branch lookup HTTP 404 remains inconclusive because private visibility can be masked",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/branches/{encoded_branch}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: PULL_REQUEST_APPROVED_EVENT,
        resource_kind: "pull_request",
        required_payload_fields: &[
            "repository.full_name",
            "pull_request.number",
            "action=reviewed",
            "review.type=pull_request_review_approved",
        ],
        payload_constraints: &[
            "action is reviewed",
            "the current Forgejo pull request read is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: PULL_REQUEST_REJECTED_EVENT,
        resource_kind: "pull_request",
        required_payload_fields: &[
            "repository.full_name",
            "pull_request.number",
            "action=reviewed",
            "review.type=pull_request_review_rejected",
        ],
        payload_constraints: &[
            "action is reviewed",
            "the current Forgejo pull request read is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: PULL_REQUEST_COMMENT_EVENT,
        resource_kind: "pull_request",
        required_payload_fields: &[
            "repository.full_name",
            "pull_request.number",
            "action=reviewed",
            "review.type=pull_request_review_comment",
        ],
        payload_constraints: &[
            "action is reviewed",
            "the current Forgejo pull request read is canonical",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_RUN_BLOCKED_EVENT,
        resource_kind: "workflow_run",
        required_payload_fields: &[
            "action",
            "run.id",
            "run.index_in_repo",
            "run.repository.full_name",
            "run.commit_sha",
            "run.updated",
            "run.status=blocked",
        ],
        payload_constraints: &[
            "the event header status must match the signed run.status",
            "the API read is canonical; workflow payload and logs are not returned",
            "a workflow status is an observation, not an independent verification verdict",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_RUN_CANCELLED_EVENT,
        resource_kind: "workflow_run",
        required_payload_fields: &[
            "action",
            "run.id",
            "run.index_in_repo",
            "run.repository.full_name",
            "run.commit_sha",
            "run.updated",
            "run.status=cancelled",
        ],
        payload_constraints: &[
            "the event header status must match the signed run.status",
            "the API read is canonical; workflow payload and logs are not returned",
            "a workflow status is an observation, not an independent verification verdict",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_RUN_FAILURE_EVENT,
        resource_kind: "workflow_run",
        required_payload_fields: &[
            "action",
            "run.id",
            "run.index_in_repo",
            "run.repository.full_name",
            "run.commit_sha",
            "run.updated",
            "run.status=failure",
        ],
        payload_constraints: &[
            "the event header status must match the signed run.status",
            "the API read is canonical; workflow payload and logs are not returned",
            "a workflow status is an observation, not an independent verification verdict",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_RUN_RUNNING_EVENT,
        resource_kind: "workflow_run",
        required_payload_fields: &[
            "action",
            "run.id",
            "run.index_in_repo",
            "run.repository.full_name",
            "run.commit_sha",
            "run.updated",
            "run.status=running",
        ],
        payload_constraints: &[
            "the event header status must match the signed run.status",
            "the API read is canonical; workflow payload and logs are not returned",
            "a workflow status is an observation, not an independent verification verdict",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_RUN_SKIPPED_EVENT,
        resource_kind: "workflow_run",
        required_payload_fields: &[
            "action",
            "run.id",
            "run.index_in_repo",
            "run.repository.full_name",
            "run.commit_sha",
            "run.updated",
            "run.status=skipped",
        ],
        payload_constraints: &[
            "the event header status must match the signed run.status",
            "the API read is canonical; workflow payload and logs are not returned",
            "a workflow status is an observation, not an independent verification verdict",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_RUN_SUCCESS_EVENT,
        resource_kind: "workflow_run",
        required_payload_fields: &[
            "action",
            "run.id",
            "run.index_in_repo",
            "run.repository.full_name",
            "run.commit_sha",
            "run.updated",
            "run.status=success",
        ],
        payload_constraints: &[
            "the event header status must match the signed run.status",
            "the API read is canonical; workflow payload and logs are not returned",
            "a workflow status is an observation, not an independent verification verdict",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_RUN_WAITING_EVENT,
        resource_kind: "workflow_run",
        required_payload_fields: &[
            "action",
            "run.id",
            "run.index_in_repo",
            "run.repository.full_name",
            "run.commit_sha",
            "run.updated",
            "run.status=waiting",
        ],
        payload_constraints: &[
            "the event header status must match the signed run.status",
            "the API read is canonical; workflow payload and logs are not returned",
            "a workflow status is an observation, not an independent verification verdict",
        ],
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_JOB_BLOCKED_EVENT,
        resource_kind: "workflow_job",
        required_payload_fields: WORKFLOW_JOB_REQUIRED_FIELDS,
        payload_constraints: WORKFLOW_JOB_PAYLOAD_CONSTRAINTS,
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_JOB_CANCELLED_EVENT,
        resource_kind: "workflow_job",
        required_payload_fields: WORKFLOW_JOB_REQUIRED_FIELDS,
        payload_constraints: WORKFLOW_JOB_PAYLOAD_CONSTRAINTS,
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_JOB_FAILURE_EVENT,
        resource_kind: "workflow_job",
        required_payload_fields: WORKFLOW_JOB_REQUIRED_FIELDS,
        payload_constraints: WORKFLOW_JOB_PAYLOAD_CONSTRAINTS,
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_JOB_RUNNING_EVENT,
        resource_kind: "workflow_job",
        required_payload_fields: WORKFLOW_JOB_REQUIRED_FIELDS,
        payload_constraints: WORKFLOW_JOB_PAYLOAD_CONSTRAINTS,
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_JOB_SKIPPED_EVENT,
        resource_kind: "workflow_job",
        required_payload_fields: WORKFLOW_JOB_REQUIRED_FIELDS,
        payload_constraints: WORKFLOW_JOB_PAYLOAD_CONSTRAINTS,
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_JOB_SUCCESS_EVENT,
        resource_kind: "workflow_job",
        required_payload_fields: WORKFLOW_JOB_REQUIRED_FIELDS,
        payload_constraints: WORKFLOW_JOB_PAYLOAD_CONSTRAINTS,
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
    WebhookEventContractV1 {
        event: WORKFLOW_JOB_WAITING_EVENT,
        resource_kind: "workflow_job",
        required_payload_fields: WORKFLOW_JOB_REQUIRED_FIELDS,
        payload_constraints: WORKFLOW_JOB_PAYLOAD_CONSTRAINTS,
        canonical_read: "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        not_found_behavior: "inconclusive_resource_not_found",
    },
];

const ISSUE_COMMENT_REQUIRED_PAYLOAD_FIELDS_V8: &[&str] = &[
    "repository.full_name",
    "issue.number",
    "action",
    "comment (object)",
    "is_pull (boolean)",
];

const ISSUE_COMMENT_PAYLOAD_CONSTRAINTS_V8: &[&str] = &[
    "action is one of created, edited, or deleted",
    "is_pull is required and boolean; true requires pull_request and false forbids it",
    "pull_request.number, when present, equals issue.number",
    "the current Forgejo issue or pull request read, not the webhook payload, is canonical",
];

const fn event_contracts_v8() -> [WebhookEventContractV1; 23] {
    let mut contracts = SUPPORTED_EVENT_CONTRACTS_V7;
    // issue_comment is the second row in the frozen, alphabetically grouped
    // contract table; all other v7 rows remain byte-for-byte equivalent.
    contracts[1].required_payload_fields = ISSUE_COMMENT_REQUIRED_PAYLOAD_FIELDS_V8;
    contracts[1].payload_constraints = ISSUE_COMMENT_PAYLOAD_CONSTRAINTS_V8;
    contracts
}

pub const SUPPORTED_EVENT_CONTRACTS_V8: [WebhookEventContractV1; 23] = event_contracts_v8();

/// Runtime admission uses the newest contract; prior discovery envelopes keep
/// their frozen v7 constraints.
const RELEASE_EVENT_CONTRACT: WebhookEventContractV1 = WebhookEventContractV1 {
    event: RELEASE_EVENT,
    resource_kind: "release",
    required_payload_fields: &["repository.full_name", "release.id", "action"],
    payload_constraints: &[
        "action is one of published, updated, or deleted",
        "release.id is a positive repository-scoped identifier",
        "the current Forgejo release API read is canonical; webhook release notes and assets are not projected",
    ],
    canonical_read: "GET /api/v1/repos/{owner}/{repository}/releases/{release_id}",
    not_found_behavior: "inconclusive_resource_not_found",
};

const fn event_contracts_v9() -> [WebhookEventContractV1; 24] {
    let mut contracts = [SUPPORTED_EVENT_CONTRACTS_V8[0]; 24];
    let mut index = 0;
    while index < SUPPORTED_EVENT_CONTRACTS_V8.len() {
        contracts[index] = SUPPORTED_EVENT_CONTRACTS_V8[index];
        index += 1;
    }
    contracts[23] = RELEASE_EVENT_CONTRACT;
    contracts
}

pub const SUPPORTED_EVENT_CONTRACTS_V9: [WebhookEventContractV1; 24] = event_contracts_v9();

/// Runtime admission uses the newest contract; v1-v8 discovery envelopes
/// retain their frozen event/action lists.
pub const SUPPORTED_EVENT_CONTRACTS: [WebhookEventContractV1; 24] = SUPPORTED_EVENT_CONTRACTS_V9;

/// Shared, versioned envelope exposed by the receiver and reconciliation CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV1 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V1: WebhookEventContractsV1 = WebhookEventContractsV1 {
    schema_version: 1,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V7,
};

/// Additive v2 discovery envelope with the structured status mapping that v1
/// could only describe in prose. The v1 event rows remain embedded unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV2 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V2: WebhookEventContractsV2 = WebhookEventContractsV2 {
    schema_version: 2,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V7,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
};

/// Additive v3 envelope publishes both workflow status and payload-action
/// constraints while retaining the v1 and v2 discovery shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV3 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
    pub workflow_actions: &'static [WebhookWorkflowActionContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V3: WebhookEventContractsV3 = WebhookEventContractsV3 {
    schema_version: 3,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V7,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
    workflow_actions: &SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
};

/// Additive v4 discovery envelope publishes event-specific action/status
/// compatibility while retaining the v1-v3 wire shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV4 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
    pub workflow_actions: &'static [WebhookWorkflowActionContractV1],
    pub workflow_event_actions: &'static [WebhookWorkflowEventActionContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V4: WebhookEventContractsV4 = WebhookEventContractsV4 {
    schema_version: 4,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V7,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
    workflow_actions: &SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
    workflow_event_actions: &SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS,
};

/// Additive v5 discovery envelope publishes action constraints for non-workflow
/// events while retaining all v1-v4 contract shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV5 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
    pub workflow_actions: &'static [WebhookWorkflowActionContractV1],
    pub workflow_event_actions: &'static [WebhookWorkflowEventActionContractV1],
    pub event_actions: &'static [WebhookEventActionContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V5: WebhookEventContractsV5 = WebhookEventContractsV5 {
    schema_version: 5,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V7,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
    workflow_actions: &SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
    workflow_event_actions: &SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS,
    event_actions: &SUPPORTED_EVENT_ACTION_CONTRACTS_V5,
};

/// Additive v6 contract publishes the complete Forgejo `issues` action set
/// while preserving the exact v5 discovery shape and values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV6 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
    pub workflow_actions: &'static [WebhookWorkflowActionContractV1],
    pub workflow_event_actions: &'static [WebhookWorkflowEventActionContractV1],
    pub event_actions: &'static [WebhookEventActionContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V6: WebhookEventContractsV6 = WebhookEventContractsV6 {
    schema_version: 6,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V7,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
    workflow_actions: &SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
    workflow_event_actions: &SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS,
    event_actions: &SUPPORTED_EVENT_ACTION_CONTRACTS_V6,
};

/// Additive v7 discovery adds the Forgejo `pull_request` action set while
/// preserving the v6 contract and its exact event-action values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV7 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
    pub workflow_actions: &'static [WebhookWorkflowActionContractV1],
    pub workflow_event_actions: &'static [WebhookWorkflowEventActionContractV1],
    pub event_actions: &'static [WebhookEventActionContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V7: WebhookEventContractsV7 = WebhookEventContractsV7 {
    schema_version: 7,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V7,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
    workflow_actions: &SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
    workflow_event_actions: &SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS,
    event_actions: &SUPPORTED_EVENT_ACTION_CONTRACTS_V7,
};

/// v8 requires the non-optional Forgejo `IssueCommentPayload.IsPull` bool and
/// preserves all previous action/status discovery fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV8 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
    pub workflow_actions: &'static [WebhookWorkflowActionContractV1],
    pub workflow_event_actions: &'static [WebhookWorkflowEventActionContractV1],
    pub event_actions: &'static [WebhookEventActionContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V8: WebhookEventContractsV8 = WebhookEventContractsV8 {
    schema_version: 8,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V8,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
    workflow_actions: &SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
    workflow_event_actions: &SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS,
    event_actions: &SUPPORTED_EVENT_ACTION_CONTRACTS_V7,
};

/// v9 adds the native Forgejo `release` event and its published/updated/deleted
/// action contract while keeping the prior v8 event subset unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WebhookEventContractsV9 {
    pub schema_version: u32,
    pub webhook_payload_is_authoritative: bool,
    pub contracts: &'static [WebhookEventContractV1],
    pub workflow_statuses: &'static [WebhookWorkflowStatusContractV1],
    pub workflow_actions: &'static [WebhookWorkflowActionContractV1],
    pub workflow_event_actions: &'static [WebhookWorkflowEventActionContractV1],
    pub event_actions: &'static [WebhookEventActionContractV1],
}

pub const WEBHOOK_EVENT_CONTRACTS_V9: WebhookEventContractsV9 = WebhookEventContractsV9 {
    schema_version: 9,
    webhook_payload_is_authoritative: false,
    contracts: &SUPPORTED_EVENT_CONTRACTS_V9,
    workflow_statuses: &SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
    workflow_actions: &SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
    workflow_event_actions: &SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS,
    event_actions: &SUPPORTED_EVENT_ACTION_CONTRACTS,
};

const fn supported_event_names() -> [&'static str; SUPPORTED_EVENT_CONTRACTS.len()] {
    let mut names = [""; SUPPORTED_EVENT_CONTRACTS.len()];
    let mut index = 0;
    while index < SUPPORTED_EVENT_CONTRACTS.len() {
        names[index] = SUPPORTED_EVENT_CONTRACTS[index].event;
        index += 1;
    }
    names
}

/// Backward-compatible event-header allowlist derived from the structured
/// contracts so ingestion, reconciliation, and capabilities cannot drift.
pub const SUPPORTED_EVENTS: [&str; SUPPORTED_EVENT_CONTRACTS.len()] = supported_event_names();

const JOURNAL_KIND_PREFIX: &str = "forgejo.webhook.v1.";
pub(crate) const WEBHOOK_QUARANTINE_JOURNAL_KIND: &str = "forgejo.webhook.quarantine.v1";

/// Host-authored append-only disposition of a persisted webhook that cannot be
/// interpreted under the current event contract. This records no canonical
/// state and does not repair or remove the source delivery.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct WebhookQuarantineDispositionV1 {
    pub schema_version: u32,
    pub delivery_id: String,
    pub source_sequence: u64,
    pub source_event_id: String,
    pub source_event_hash: String,
    pub reason_code: String,
    pub reason_field: Option<String>,
    pub disposition_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct WebhookQuarantineResultV1 {
    pub schema_version: u32,
    pub delivery_id: String,
    pub source_sequence: u64,
    pub source_event_hash: String,
    pub reason_code: String,
    pub reason_field: Option<String>,
    pub disposition_sequence: u64,
    pub duplicate: bool,
}

pub(crate) fn valid_delivery_id_v1(value: &str) -> bool {
    valid_delivery_id(value)
}

/// Reserved webhook journal kinds include both authenticated source events
/// and the narrowly validated host quarantine disposition.
pub(crate) fn is_reserved_journal_kind(kind: &str) -> bool {
    is_supported_journal_kind(kind) || kind == WEBHOOK_QUARANTINE_JOURNAL_KIND
}

pub(crate) fn quarantine_reason_for_invalid_delivery(
    event: &JournalEvent,
) -> Option<(&'static str, Option<&'static str>)> {
    match reconcile::event_resource_ref(event) {
        Err(reconcile::ReconcileError::InvalidEnvelope) => Some(("invalid_envelope", None)),
        Err(reconcile::ReconcileError::UnexpectedEventPayloadField(field)) => {
            Some(("unexpected_event_payload_field", Some(field)))
        }
        Err(reconcile::ReconcileError::InvalidEventPayloadField(field)) => {
            Some(("invalid_event_payload_field", Some(field)))
        }
        Err(reconcile::ReconcileError::UnsupportedEventPayloadField(field)) => {
            Some(("unsupported_event_payload_field", Some(field)))
        }
        Err(reconcile::ReconcileError::UnsupportedEvent) => Some(("unsupported_event", None)),
        Err(reconcile::ReconcileError::InvalidRepository) => Some(("invalid_repository", None)),
        Err(reconcile::ReconcileError::InvalidNumber) => Some(("invalid_resource_number", None)),
        Err(reconcile::ReconcileError::InvalidBranch) => Some(("invalid_branch", None)),
        _ => None,
    }
}

/// Append a durable quarantine disposition only when the operator-supplied
/// sequence and hash still identify the exact invalid source row.
pub fn quarantine_invalid_webhook_delivery(
    journal: &EventJournal,
    delivery_id: &str,
    source_sequence: u64,
    source_event_hash: &str,
) -> Result<WebhookQuarantineResultV1, WebhookError> {
    if !valid_delivery_id(delivery_id) {
        return Err(WebhookError::InvalidDeliveryId);
    }
    let result = journal
        .append_webhook_quarantine(delivery_id, source_sequence, source_event_hash)
        .map_err(WebhookError::Journal)?;
    let (disposition, duplicate) = match result {
        crate::AppendResult::Appended(event) => (event, false),
        crate::AppendResult::Duplicate(event) => (event, true),
    };
    let disposition = parse_quarantine_disposition(&disposition)?;
    Ok(WebhookQuarantineResultV1 {
        schema_version: 1,
        delivery_id: disposition.delivery_id,
        source_sequence: disposition.source_sequence,
        source_event_hash: disposition.source_event_hash,
        reason_code: disposition.reason_code,
        reason_field: disposition.reason_field,
        disposition_sequence: disposition.disposition_sequence,
        duplicate,
    })
}

/// Find a validated quarantine disposition in an already-read journal
/// snapshot. Used by consumers that must make the decision from one snapshot.
pub fn quarantine_for_source(
    events: &[JournalEvent],
    source: &JournalEvent,
) -> Result<Option<WebhookQuarantineDispositionV1>, WebhookError> {
    let mut matching = None;
    for event in events
        .iter()
        .filter(|event| event.kind == WEBHOOK_QUARANTINE_JOURNAL_KIND)
    {
        let disposition = parse_quarantine_disposition(event)?;
        let bound_source = events
            .get(
                usize::try_from(disposition.source_sequence.saturating_sub(1))
                    .unwrap_or(usize::MAX),
            )
            .filter(|candidate| {
                candidate.sequence == disposition.source_sequence
                    && candidate.event_id == disposition.source_event_id
                    && candidate.hash == disposition.source_event_hash
                    && candidate.idempotency_key
                        == format!("forgejo-delivery-v1:{}", disposition.delivery_id)
                    && is_supported_journal_kind(&candidate.kind)
            })
            .ok_or(WebhookError::InvalidJournaledEvent)?;
        let (reason_code, reason_field) = quarantine_reason_for_invalid_delivery(bound_source)
            .ok_or(WebhookError::InvalidJournaledEvent)?;
        if disposition.reason_code != reason_code
            || disposition.reason_field.as_deref() != reason_field
        {
            return Err(WebhookError::InvalidJournaledEvent);
        }
        if disposition.source_sequence == source.sequence
            && disposition.source_event_id == source.event_id
            && disposition.source_event_hash == source.hash
            && disposition.delivery_id
                == source
                    .payload
                    .get("delivery_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
        {
            if matching.replace(disposition).is_some() {
                return Err(WebhookError::InvalidJournaledEvent);
            }
        }
    }
    Ok(matching)
}

fn parse_quarantine_disposition(
    event: &JournalEvent,
) -> Result<WebhookQuarantineDispositionV1, WebhookError> {
    if event.kind != WEBHOOK_QUARANTINE_JOURNAL_KIND {
        return Err(WebhookError::InvalidJournaledEvent);
    }
    let payload = event
        .payload
        .as_object()
        .ok_or(WebhookError::InvalidJournaledEvent)?;
    let schema_version = payload
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or(WebhookError::InvalidJournaledEvent)?;
    let delivery_id = payload
        .get("delivery_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(WebhookError::InvalidJournaledEvent)?;
    let source_sequence = payload
        .get("source_sequence")
        .and_then(serde_json::Value::as_u64)
        .ok_or(WebhookError::InvalidJournaledEvent)?;
    let source_event_id = payload
        .get("source_event_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(WebhookError::InvalidJournaledEvent)?;
    let source_event_hash = payload
        .get("source_event_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or(WebhookError::InvalidJournaledEvent)?;
    let reason_code = payload
        .get("reason_code")
        .and_then(serde_json::Value::as_str)
        .ok_or(WebhookError::InvalidJournaledEvent)?;
    let reason_field = match payload.get("reason_field") {
        Some(serde_json::Value::Null) | None => None,
        Some(serde_json::Value::String(value)) => Some(value.as_str()),
        _ => return Err(WebhookError::InvalidJournaledEvent),
    };
    if schema_version != 1
        || payload.keys().any(|key| {
            !matches!(
                key.as_str(),
                "schema_version"
                    | "delivery_id"
                    | "source_sequence"
                    | "source_event_id"
                    | "source_event_hash"
                    | "reason_code"
                    | "reason_field"
            )
        })
        || !valid_delivery_id(delivery_id)
        || source_sequence == 0
        || source_sequence >= event.sequence
        || source_event_id.is_empty()
        || !is_lower_hex_sha256(source_event_hash)
        || event.idempotency_key
            != format!("forgejo-webhook-quarantine-v1:{delivery_id}:{source_event_hash}")
    {
        return Err(WebhookError::InvalidJournaledEvent);
    }
    Ok(WebhookQuarantineDispositionV1 {
        schema_version: 1,
        delivery_id: delivery_id.to_owned(),
        source_sequence,
        source_event_id: source_event_id.to_owned(),
        source_event_hash: source_event_hash.to_owned(),
        reason_code: reason_code.to_owned(),
        reason_field: reason_field.map(str::to_owned),
        disposition_sequence: event.sequence,
    })
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether `kind` is the journal kind of one of [`SUPPORTED_EVENTS`].
pub fn is_supported_journal_kind(kind: &str) -> bool {
    kind.strip_prefix(JOURNAL_KIND_PREFIX)
        .is_some_and(|event| SUPPORTED_EVENTS.contains(&event))
}

#[derive(Debug, PartialEq, Eq)]
pub enum WebhookError {
    EmptySecret,
    InvalidDeliveryId,
    UnsupportedEvent,
    BodyTooLarge,
    InvalidSignature,
    InvalidJson,
    InvalidJsonAt {
        line: usize,
        column: usize,
    },
    InvalidEventPayload,
    InvalidEventPayloadField(&'static str),
    UnsupportedEventPayloadField(&'static str),
    IdempotencyConflict,
    DeliveryNotFound,
    QuarantinedDelivery {
        source_sequence: u64,
        disposition_sequence: u64,
    },
    InvalidJournaledEvent,
    Attestation(JournalAttestationError),
    Journal(JournalError),
}

/// A webhook event read back from the append-only journal after envelope and
/// journal-chain validation. Its private field prevents callers from wrapping
/// an arbitrary public `JournalEvent` and passing it to admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournaledWebhookEventV1 {
    event: JournalEvent,
}

impl JournaledWebhookEventV1 {
    pub fn event(&self) -> &JournalEvent {
        &self.event
    }
}

/// Verify a Forgejo delivery and append it to the durable inbox.
///
/// The payload is only a notification. Callers must reconcile the affected
/// issue, pull request, or branch against Forgejo before changing canonical state.
pub fn ingest_forgejo_delivery(
    journal: &EventJournal,
    secret: &[u8],
    delivery_id: &str,
    event_name: &str,
    signature_hex: &str,
    raw_body: &[u8],
) -> Result<AppendResult, WebhookError> {
    ingest_forgejo_delivery_inner(
        journal,
        secret,
        delivery_id,
        event_name,
        signature_hex,
        raw_body,
        None,
    )
}

/// Verify a Forgejo HMAC and append the event with a receiver signature.
/// The signing key must be a separate host-held key; this API never derives it
/// from or forwards the Forgejo webhook secret.
pub fn ingest_forgejo_delivery_attested(
    journal: &EventJournal,
    secret: &[u8],
    signing_key: &SigningKey,
    delivery_id: &str,
    event_name: &str,
    signature_hex: &str,
    raw_body: &[u8],
) -> Result<AppendResult, WebhookError> {
    ingest_forgejo_delivery_inner(
        journal,
        secret,
        delivery_id,
        event_name,
        signature_hex,
        raw_body,
        Some(signing_key),
    )
}

fn ingest_forgejo_delivery_inner(
    journal: &EventJournal,
    secret: &[u8],
    delivery_id: &str,
    event_name: &str,
    signature_hex: &str,
    raw_body: &[u8],
    attestation_signer: Option<&SigningKey>,
) -> Result<AppendResult, WebhookError> {
    if secret.is_empty() {
        return Err(WebhookError::EmptySecret);
    }
    if delivery_id.is_empty()
        || delivery_id.len() > 128
        || !delivery_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(WebhookError::InvalidDeliveryId);
    }
    if !SUPPORTED_EVENTS.contains(&event_name) {
        return Err(WebhookError::UnsupportedEvent);
    }
    if raw_body.len() > MAX_WEBHOOK_BODY_BYTES {
        return Err(WebhookError::BodyTooLarge);
    }

    let signature = hex::decode(signature_hex).map_err(|_| WebhookError::InvalidSignature)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| WebhookError::InvalidSignature)?;
    mac.update(raw_body);
    mac.verify_slice(&signature)
        .map_err(|_| WebhookError::InvalidSignature)?;

    let payload =
        crate::strict_json::parse_value_rejecting_duplicate_members(raw_body).map_err(|error| {
            WebhookError::InvalidJsonAt {
                line: error.line(),
                column: error.column(),
            }
        })?;
    if !payload.is_object() {
        return Err(WebhookError::InvalidJson);
    }
    match reconcile::validate_event_payload(event_name, &payload) {
        Ok(()) => {}
        Err(
            reconcile::ReconcileError::InvalidEventPayloadField(field)
            | reconcile::ReconcileError::UnexpectedEventPayloadField(field),
        ) => {
            return Err(WebhookError::InvalidEventPayloadField(field));
        }
        Err(reconcile::ReconcileError::UnsupportedEventPayloadField(field)) => {
            return Err(WebhookError::UnsupportedEventPayloadField(field));
        }
        Err(_) => return Err(WebhookError::InvalidEventPayload),
    }
    let journal_payload = json!({
        "schema_version": 1,
        "provider": "forgejo",
        "delivery_id": delivery_id,
        "event": event_name,
        "payload": payload,
    });
    let idempotency_key = format!("forgejo-delivery-v1:{delivery_id}");
    let kind = format!("{JOURNAL_KIND_PREFIX}{event_name}");

    let append_result = match attestation_signer {
        Some(signing_key) => journal.append_validated_webhook_attested(
            &idempotency_key,
            &kind,
            journal_payload,
            signing_key,
        ),
        None => journal.append_validated_webhook(&idempotency_key, &kind, journal_payload),
    };
    match append_result {
        Ok(result) => Ok(result),
        Err(JournalError::IdempotencyConflict) => Err(WebhookError::IdempotencyConflict),
        Err(error) => Err(WebhookError::Journal(error)),
    }
}

/// Read an authenticated-ingestion event by its Forgejo delivery ID without
/// changing the inbox. A partial final record makes the lookup inconclusive.
///
/// Replay verifies the journal hash chain and the envelope is checked against
/// its event kind and idempotency key. The raw HMAC is not persisted, so this
/// does not re-verify an HMAC at read time; the local filesystem writer remains
/// inside the prototype's trusted boundary.
pub fn read_journaled_webhook_event(
    journal: &EventJournal,
    delivery_id: &str,
) -> Result<JournaledWebhookEventV1, WebhookError> {
    if !valid_delivery_id(delivery_id) {
        return Err(WebhookError::InvalidDeliveryId);
    }
    let idempotency_key = format!("forgejo-delivery-v1:{delivery_id}");
    let events = journal.replay_read_only().map_err(WebhookError::Journal)?;
    let event = events
        .iter()
        .find(|event| event.idempotency_key == idempotency_key)
        .cloned()
        .ok_or(WebhookError::DeliveryNotFound)?;

    if let Some(disposition) = quarantine_for_source(&events, &event)? {
        return Err(WebhookError::QuarantinedDelivery {
            source_sequence: disposition.source_sequence,
            disposition_sequence: disposition.disposition_sequence,
        });
    }

    if !is_supported_journal_kind(&event.kind) || reconcile::event_resource_ref(&event).is_err() {
        return Err(WebhookError::InvalidJournaledEvent);
    }
    Ok(JournaledWebhookEventV1 { event })
}

/// Read and verify a webhook row using a host-supplied trusted key set. Legacy
/// rows without an attestation are refused; callers that intentionally support
/// legacy replay must use the explicitly unauthenticated reader above.
pub fn read_journaled_webhook_event_verified(
    journal: &EventJournal,
    delivery_id: &str,
    trusted_keys: &[VerifyingKey],
) -> Result<JournaledWebhookEventV1, WebhookError> {
    let event = read_journaled_webhook_event(journal, delivery_id)?;
    verify_journal_event_attestation(event.event(), trusted_keys)
        .map_err(WebhookError::Attestation)?;
    Ok(event)
}

fn valid_delivery_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod quarantine_tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is after UNIX_EPOCH")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "agent-forge-webhook-quarantine-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create scratch directory");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("restrict scratch directory");
            Self(path)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn append_hash_valid_invalid_review(path: &std::path::Path, delivery_id: &str) -> JournalEvent {
        let event_name = "pull_request_approved";
        let payload = serde_json::json!({
            "repository": {"full_name":"forge-admin/agent-forge-core"},
            "pull_request": {"number": 17},
            "review": {"type":"pull_request_review_approved"}
        });
        let fields = serde_json::json!({
            "sequence": 1,
            "event_id": "evt-1",
            "idempotency_key": format!("forgejo-delivery-v1:{delivery_id}"),
            "kind": format!("{JOURNAL_KIND_PREFIX}{event_name}"),
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "delivery_id": delivery_id,
                "event": event_name,
                "payload": payload,
            },
            "previous_hash": "0".repeat(64),
        });
        let hash = crate::sha256(&serde_json::to_vec(&fields).expect("serialize event hash"));
        let mut value = fields;
        value["hash"] = serde_json::Value::String(hash);
        let event: JournalEvent = serde_json::from_value(value.clone()).expect("parse event");
        let mut bytes = serde_json::to_vec(&value).expect("serialize event");
        bytes.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .expect("create synthetic source journal");
        file.write_all(&bytes).expect("write source event");
        file.sync_all().expect("sync source event");
        event
    }

    #[test]
    fn trusted_receiver_attestation_rejects_semantically_valid_rehashed_payload_tampering() {
        use ed25519_dalek::SigningKey;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let scratch = ScratchDir::new();
        let path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open_webhook_inbox(&path);
        let secret = b"synthetic-webhook-secret-for-attestation-test";
        let signing_key = SigningKey::from_bytes(&[0x41; 32]);
        let delivery_id = "attestation-semantic-tamper-41";
        let body = br#"{"action":"opened","repository":{"full_name":"forge-admin/agent-forge-core"},"issue":{"number":41,"state":"open"}}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts fixture key");
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());

        ingest_forgejo_delivery_attested(
            &journal,
            secret,
            &signing_key,
            delivery_id,
            "issues",
            &signature,
            body,
        )
        .expect("append HMAC-verified and receiver-attested event");
        let trusted_keys = [signing_key.verifying_key()];
        assert_eq!(
            read_journaled_webhook_event_verified(&journal, delivery_id, &trusted_keys)
                .expect("untampered event verifies")
                .event()
                .payload["payload"]["issue"]["number"],
            41
        );

        let mut event: JournalEvent = serde_json::from_slice(
            fs::read(&path)
                .expect("read private test journal")
                .strip_suffix(b"\n")
                .expect("journal row has its newline"),
        )
        .expect("parse journal event");
        event.payload["payload"]["issue"]["number"] = serde_json::json!(42);
        // Model an attacker who can recompute the unkeyed journal chain but
        // cannot produce a new signature from the host-held receiver key.
        event.hash = crate::event_hash(&event);
        let mut rewritten = serde_json::to_vec(&event).expect("serialize rewritten journal row");
        rewritten.push(b'\n');
        fs::write(&path, rewritten).expect("rewrite fixture journal with a valid chain hash");

        assert_eq!(
            read_journaled_webhook_event(&journal, delivery_id)
                .expect("legacy reader checks the chain and event contract")
                .event()
                .payload["payload"]["issue"]["number"],
            42,
            "the unhashed trust path accepts this structurally valid rewritten event"
        );
        assert!(matches!(
            read_journaled_webhook_event_verified(&journal, delivery_id, &trusted_keys),
            Err(WebhookError::Attestation(
                JournalAttestationError::InvalidSignature
            ))
        ));
    }

    #[test]
    fn quarantine_is_bound_idempotent_and_never_reconciles_invalid_source() {
        let scratch = ScratchDir::new();
        let path = scratch.0.join("inbox.jsonl");
        let delivery_id = "quarantine-review-17";
        let source = append_hash_valid_invalid_review(&path, delivery_id);
        let original_source_bytes = fs::read(&path).expect("read source bytes");
        let journal = EventJournal::open_webhook_inbox(&path);

        assert!(matches!(
            read_journaled_webhook_event(&journal, delivery_id),
            Err(WebhookError::InvalidJournaledEvent)
        ));
        assert_eq!(
            journal.append(
                "forged-quarantine",
                WEBHOOK_QUARANTINE_JOURNAL_KIND,
                serde_json::json!({}),
            ),
            Err(JournalError::InvalidEvent),
            "generic writers cannot claim the reserved disposition kind"
        );
        assert!(matches!(
            quarantine_invalid_webhook_delivery(&journal, delivery_id, 1, &"f".repeat(64)),
            Err(WebhookError::Journal(JournalError::InvalidEvent))
        ));

        let first = quarantine_invalid_webhook_delivery(&journal, delivery_id, 1, &source.hash)
            .expect("append quarantine disposition");
        assert_eq!(first.reason_code, "invalid_event_payload_field");
        assert_eq!(first.reason_field.as_deref(), Some("action"));
        assert_eq!(first.source_sequence, 1);
        assert_eq!(first.source_event_hash, source.hash);
        assert_eq!(first.disposition_sequence, 2);
        assert!(!first.duplicate);

        let replayed = journal
            .replay_read_only()
            .expect("read append-only journal");
        assert_eq!(replayed.len(), 2);
        assert_eq!(replayed[0], source, "source row is unchanged");
        assert_eq!(
            fs::read(&path)
                .expect("read source plus disposition")
                .split(|byte| *byte == b'\n')
                .next()
                .expect("source line"),
            original_source_bytes
                .split(|byte| *byte == b'\n')
                .next()
                .expect("original source line"),
            "quarantine appends without rewriting source bytes"
        );
        assert!(matches!(
            read_journaled_webhook_event(&journal, delivery_id),
            Err(WebhookError::QuarantinedDelivery {
                source_sequence: 1,
                disposition_sequence: 2,
            })
        ));
        let mut changed_snapshot = replayed.clone();
        changed_snapshot[0].hash = "a".repeat(64);
        assert!(matches!(
            quarantine_for_source(&changed_snapshot, &changed_snapshot[0]),
            Err(WebhookError::InvalidJournaledEvent)
        ));

        let duplicate = quarantine_invalid_webhook_delivery(&journal, delivery_id, 1, &source.hash)
            .expect("same host disposition is idempotent");
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.disposition_sequence, first.disposition_sequence);
        assert_eq!(journal.replay_read_only().unwrap().len(), 2);
    }

    #[test]
    fn quarantine_refuses_a_torn_journal_tail_without_repairing_it() {
        let scratch = ScratchDir::new();
        let path = scratch.0.join("inbox.jsonl");
        let delivery_id = "quarantine-torn-review-17";
        let source = append_hash_valid_invalid_review(&path, delivery_id);
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open journal for crash-tail fixture");
        file.write_all(b"{\"sequence\":").expect("append torn tail");
        file.sync_all().expect("sync torn tail");
        drop(file);
        let before = fs::read(&path).expect("read journal before refusal");

        assert!(matches!(
            quarantine_invalid_webhook_delivery(
                &EventJournal::open_webhook_inbox(&path),
                delivery_id,
                1,
                &source.hash,
            ),
            Err(WebhookError::Journal(JournalError::Corrupt { line: 2 }))
        ));
        assert_eq!(fs::read(&path).expect("read journal after refusal"), before);
    }
}
