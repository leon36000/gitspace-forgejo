use crate::{JournalEvent, forgejo::MAX_FORGEJO_LABEL_NAME_BYTES};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
};

pub const MAX_RECONCILE_BATCH_EVENTS: usize = 100;
/// Bound serialized JSON bytes accounted for across one batch cache.
/// This is a byte-accounting limit, not a Rust heap-memory limit.
pub const MAX_RECONCILE_BATCH_CACHED_JSON_BYTES: usize = 8 * 1024 * 1024;
/// Bound the projected result JSON for one reconciliation batch. Each result
/// reserves additional space for its CLI envelope and error details.
pub const MAX_RECONCILE_BATCH_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const RECONCILE_BATCH_RESULT_ENVELOPE_RESERVE_BYTES: usize = 2048;
pub const MAX_RECONCILED_REQUESTED_REVIEWERS: usize = 1024;
pub const MAX_RECONCILED_REQUESTED_REVIEWER_TEAMS: usize = 1024;
pub const MAX_RECONCILED_REVIEWER_TEAM_NAME_CHARS: usize = 255;
pub const MAX_RECONCILED_LABELS: usize = 256;
pub const MAX_RECONCILED_LABEL_NAME_BYTES: usize = MAX_FORGEJO_LABEL_NAME_BYTES;
pub const MAX_RECONCILED_RELEASE_TAG_NAME_BYTES: usize = 255;
#[deprecated(note = "use MAX_RECONCILED_LABEL_NAME_BYTES; the contract is UTF-8 bytes")]
pub const MAX_RECONCILED_LABEL_NAME_CHARS: usize = MAX_RECONCILED_LABEL_NAME_BYTES;
/// Maximum UTF-8 byte length returned for a review notification body.
/// Oversized source text is prefix-truncated on a character boundary; the
/// complete byte length and SHA-256 remain available for source comparison.
pub const MAX_REVIEW_NOTIFICATION_BODY_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Issue,
    IssueComment,
    PullRequest,
    PullRequestComment,
    Branch,
    WorkflowRun,
    WorkflowJob,
    Release,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResourceId {
    Number(u64),
    IssueComment {
        issue_number: u64,
        comment_id: u64,
        deleted_event: bool,
    },
    Branch(String),
    WorkflowRun {
        run_id: u64,
        index_in_repo: u64,
    },
    WorkflowJob {
        job_id: u64,
        run_id: u64,
        index_in_repo: u64,
        job_index: u64,
    },
    Release(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub owner: String,
    pub repository: String,
    pub id: ResourceId,
}

impl ResourceRef {
    /// Return the issue/PR number, or the parent issue number for a comment.
    pub fn number(&self) -> Option<u64> {
        match &self.id {
            ResourceId::Number(number) => Some(*number),
            ResourceId::IssueComment { issue_number, .. } => Some(*issue_number),
            ResourceId::Branch(_) => None,
            ResourceId::WorkflowRun { .. } => None,
            ResourceId::WorkflowJob { .. } => None,
            ResourceId::Release(_) => None,
        }
    }
}

pub trait ForgejoCurrentState {
    fn read_current(&self, resource: &ResourceRef) -> Result<Value, ApiReadError>;

    /// Force a source read when a prior response may now be stale.
    /// This default is correct for direct, uncached readers; implementations
    /// that cache `read_current` must override it to evict or bypass that cache.
    /// A deleted-comment 404 uses this to recheck parent visibility.
    fn read_current_fresh(&self, resource: &ResourceRef) -> Result<Value, ApiReadError> {
        self.read_current(resource)
    }

    /// Return the canonical web URL derived from trusted adapter configuration,
    /// never from a webhook or API response field.
    fn expected_html_url(&self, resource: &ResourceRef) -> String;

    /// Derive a canonical web URL for a current release tag from trusted
    /// adapter configuration. The webhook and API `html_url` are never used
    /// as the output URL.
    fn expected_release_html_url(&self, resource: &ResourceRef, tag_name: &str) -> String {
        let collection_url = self.expected_html_url(resource);
        if !collection_url.ends_with("/releases") {
            return String::new();
        }
        format!(
            "{collection_url}/tag/{}",
            crate::forgejo::encode_url_path_segments(tag_name)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiReadError {
    Unavailable,
    Unauthorized,
    Forbidden,
    NotFound,
    InvalidCredential,
    BatchCacheBudgetExceeded {
        maximum_cached_json_bytes: usize,
    },
    InconsistentPagination,
    HttpStatus(u16),
    HttpStatusWithRetryAfter {
        status: u16,
        retry_after_ms: u64,
    },
    /// The response status is retained, but Retry-After cannot safely guide a retry.
    HttpStatusWithInvalidRetryAfter {
        status: u16,
    },
    /// The adapter rejected a response component at a documented byte limit.
    ResponseTooLarge {
        component: &'static str,
        maximum_bytes: usize,
    },
    InvalidResponse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileError {
    BatchTooLarge,
    BatchOutputBudgetExceeded {
        maximum_output_bytes: usize,
    },
    InvalidEnvelope,
    InvalidEventPayloadField(&'static str),
    UnexpectedEventPayloadField(&'static str),
    UnsupportedEventPayloadField(&'static str),
    UnsupportedEvent,
    InvalidRepository,
    InvalidNumber,
    InvalidBranch,
    Source(ApiReadError),
    ParentWorkflowRunNotFound {
        run_id: u64,
        index_in_repo: u64,
    },
    ResourceMismatch,
    InvalidCurrentState,
    InvalidCurrentStateField(&'static str),
    DeliveryQuarantined {
        source_sequence: u64,
        source_event_hash: String,
        disposition_sequence: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledState {
    pub delivery_id: String,
    pub resource: ResourceRef,
    pub state: String,
    pub updated_at: String,
    pub html_url: String,
    pub head_sha: Option<String>,
    /// Current parent run for workflow-job resources. This is descriptive
    /// Forgejo state and never an independent verifier result.
    pub parent_workflow_run: Option<ReconciledWorkflowRun>,
    pub base_sha: Option<String>,
    pub merged: Option<bool>,
    /// Current draft state from Forgejo's pull-request API. This is
    /// descriptive, not merge authorization; missing/malformed values make
    /// the pull-request snapshot inconclusive.
    pub draft: Option<bool>,
    pub merge_commit_sha: Option<String>,
    /// Public identities currently requested for review. `None` means the
    /// source omitted the field; an empty list means it reported no requests.
    pub requested_reviewers: Option<Vec<ReconciledReviewer>>,
    /// Organization teams currently requested for review. `None` means the
    /// source omitted the field; an empty list means no team requests.
    pub requested_reviewers_teams: Option<Vec<ReconciledReviewerTeam>>,
    /// Current issue/PR labels. Label names are user controlled; `None` means
    /// the source omitted the field or returned null, while an empty list is
    /// a known empty set.
    pub labels: Option<Vec<ReconciledLabel>>,
    /// Current issue/PR assignees. `None` means the source omitted the field,
    /// or a pull-request response returned null. Forgejo v17's issue API
    /// returns null for an explicitly empty assignee set, which is projected
    /// as `Some(vec![])`.
    pub assignees: Option<Vec<ReconciledAssignee>>,
    /// Current milestone observation for issue/PR resources and their comment
    /// parents. The enum distinguishes an omitted field from explicit null.
    /// `None` means milestones do not apply to this resource.
    pub milestone: Option<ReconciledMilestoneObservation>,
    /// Current issue/PR title and body from Forgejo's canonical API. These
    /// fields are user controlled; absence of either field remains unknown.
    pub work_item_content: Option<ReconciledWorkItemContent>,
    pub comment: Option<ReconciledComment>,
    /// A review note copied from the signed event envelope. This is delivery
    /// context, not canonical review state; Forgejo v17 does not include a
    /// review ID in this webhook shape.
    pub review_notification: Option<ReconciledReviewNotification>,
    /// Bounded identity/status projection for a current Forgejo release.
    /// Release notes, titles, assets and publisher details are deliberately
    /// excluded from this first adapter slice.
    pub release: Option<ReconciledRelease>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledRelease {
    pub id: u64,
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledWorkItemContent {
    pub title: String,
    /// `None` means Forgejo explicitly returned JSON null. A missing body or
    /// title prevents the content projection and leaves it unknown instead.
    pub body: Option<String>,
    pub source: &'static str,
    pub trust: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledReviewer {
    pub id: u64,
    pub login: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledReviewerTeam {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledLabel {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledAssignee {
    pub id: u64,
    pub login: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconciledMilestoneObservation {
    Unknown,
    Unassigned,
    Assigned(ReconciledMilestone),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledMilestone {
    pub id: u64,
    pub title: String,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledComment {
    pub id: u64,
    pub state: String,
    pub updated_at: Option<String>,
    pub html_url: Option<String>,
    pub body: Option<String>,
    /// Comment text is user-controlled and must be handled as untrusted input.
    pub body_trust: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledReviewNotification {
    pub review_type: &'static str,
    pub body: Option<String>,
    /// True when `body` is only the bounded prefix of the webhook text.
    pub body_truncated: bool,
    /// UTF-8 byte length of the complete source text, when present.
    pub body_byte_length: Option<usize>,
    /// SHA-256 of the complete source text before any truncation. This records
    /// identity only; it does not establish correctness or approval.
    pub body_sha256: Option<String>,
    /// The PR head included in the webhook payload, if it is a valid object
    /// ID. It records the event snapshot; it does not prove which commit the
    /// review targeted.
    pub pull_request_head_sha_at_event: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciledWorkflowRun {
    pub run_id: u64,
    pub index_in_repo: u64,
    pub state: String,
    pub updated_at: String,
    pub html_url: String,
    pub head_sha: String,
}

pub fn reconciled_state_value(reconciled: &ReconciledState) -> Value {
    let resource = match (&reconciled.resource.kind, &reconciled.resource.id) {
        (ResourceKind::Issue, ResourceId::Number(number)) => json!({
            "kind": "issue",
            "owner": reconciled.resource.owner,
            "repository": reconciled.resource.repository,
            "number": number,
        }),
        (ResourceKind::PullRequest, ResourceId::Number(number)) => json!({
            "kind": "pull_request",
            "owner": reconciled.resource.owner,
            "repository": reconciled.resource.repository,
            "number": number,
        }),
        (
            ResourceKind::IssueComment,
            ResourceId::IssueComment {
                issue_number,
                comment_id,
                ..
            },
        ) => json!({
            "kind": "issue_comment",
            "owner": reconciled.resource.owner,
            "repository": reconciled.resource.repository,
            "issue_number": issue_number,
            "comment_id": comment_id,
        }),
        (ResourceKind::PullRequestComment, ResourceId::IssueComment { issue_number, .. }) => {
            json!({
                "kind": "pull_request",
                "owner": reconciled.resource.owner,
                "repository": reconciled.resource.repository,
                "number": issue_number,
            })
        }
        (ResourceKind::Branch, ResourceId::Branch(branch)) => json!({
            "kind": "branch",
            "owner": reconciled.resource.owner,
            "repository": reconciled.resource.repository,
            "branch": branch,
        }),
        (
            ResourceKind::WorkflowRun,
            ResourceId::WorkflowRun {
                run_id,
                index_in_repo,
            },
        ) => json!({
            "kind": "workflow_run",
            "owner": reconciled.resource.owner,
            "repository": reconciled.resource.repository,
            "run_id": run_id,
            "index_in_repo": index_in_repo,
        }),
        (
            ResourceKind::WorkflowJob,
            ResourceId::WorkflowJob {
                job_id,
                run_id,
                index_in_repo,
                ..
            },
        ) => json!({
            "kind": "workflow_job",
            "owner": reconciled.resource.owner,
            "repository": reconciled.resource.repository,
            "job_id": job_id,
            "run_id": run_id,
            "index_in_repo": index_in_repo,
        }),
        (ResourceKind::Release, ResourceId::Release(release_id)) => json!({
            "kind": "release",
            "owner": reconciled.resource.owner,
            "repository": reconciled.resource.repository,
            "release_id": release_id,
        }),
        _ => json!({}),
    };
    json!({
        "delivery_id": reconciled.delivery_id,
        "resource": resource,
        "review_notification": reconciled.review_notification.as_ref().map(|review| json!({
            "review_type": review.review_type,
            "body": review.body,
            "body_truncated": review.body_truncated,
            "body_byte_length": review.body_byte_length,
            "body_sha256": review.body_sha256,
            "body_trust": "untrusted_user_content",
            "source": "forgejo_webhook_delivery",
            "freshness": "event_snapshot_not_canonical",
            "review_id_binding": "unavailable_in_forgejo_v17_webhook",
            "pull_request_head_sha_at_event": review.pull_request_head_sha_at_event,
        })),
        "canonical_state": {
            "state": reconciled.state,
            "updated_at": if reconciled.release.is_some() { Value::Null } else { json!(reconciled.updated_at) },
            "html_url": reconciled.html_url,
            "head_sha": reconciled.head_sha,
            "parent_workflow_run": reconciled.parent_workflow_run.as_ref().map(|run| json!({
                "run_id": run.run_id,
                "index_in_repo": run.index_in_repo,
                "state": run.state,
                "updated_at": run.updated_at,
                "html_url": run.html_url,
                "head_sha": run.head_sha,
                "source": "current_forgejo_api",
                "freshness": "current_api_read",
                "independent_verification": false,
                "merge_authorizing": false,
            })),
            "base_sha": reconciled.base_sha,
            "merged": reconciled.merged,
            "draft": reconciled.draft,
            "merge_commit_sha": reconciled.merge_commit_sha,
            "requested_reviewers": reconciled.requested_reviewers.as_ref().map(|reviewers| {
                reviewers
                    .iter()
                    .map(|reviewer| json!({"id": reviewer.id, "login": reviewer.login}))
                    .collect::<Vec<_>>()
            }),
            "requested_reviewers_teams": reconciled
                .requested_reviewers_teams
                .as_ref()
                .map(|teams| {
                    teams
                        .iter()
                        .map(|team| json!({"id": team.id, "name": team.name}))
                        .collect::<Vec<_>>()
                }),
            "labels": reconciled.labels.as_ref().map(|labels| json!({
                "items": labels
                    .iter()
                    .map(|label| json!({"id": label.id, "name": label.name}))
                    .collect::<Vec<_>>(),
                "source": "current_forgejo_resource_api",
                "freshness": "current_api_read",
                "trust": "untrusted_user_content",
            })),
            "assignees": reconciled.assignees.as_ref().map(|assignees| json!({
                "items": assignees
                    .iter()
                    .map(|assignee| json!({"id": assignee.id, "login": assignee.login}))
                    .collect::<Vec<_>>(),
                "source": "current_forgejo_resource_api",
                "freshness": "current_api_read",
                "trust": "untrusted_forgejo_identity",
            })),
            "milestone": reconciled.milestone.as_ref().map(|milestone| {
                let (observation, value) = match milestone {
                    ReconciledMilestoneObservation::Unknown => ("unknown", Value::Null),
                    ReconciledMilestoneObservation::Unassigned => ("unassigned", Value::Null),
                    ReconciledMilestoneObservation::Assigned(milestone) => (
                        "assigned",
                        json!({
                            "id": milestone.id,
                            "title": milestone.title,
                            "state": milestone.state,
                        }),
                    ),
                };
                json!({
                    "observation": observation,
                    "value": value,
                    "source": "current_forgejo_resource_api",
                    "freshness": "current_api_read",
                    "trust": "untrusted_user_content",
                })
            }),
            "work_item_content": reconciled.work_item_content.as_ref().map(|content| json!({
                "title": content.title,
                "body": content.body,
                "source": content.source,
                "trust": content.trust,
            })),
            "comment": reconciled.comment.as_ref().map(|comment| json!({
                "id": comment.id,
                "state": comment.state,
                "updated_at": comment.updated_at,
                "html_url": comment.html_url,
                "body": comment.body,
                "body_trust": comment.body_trust,
            })),
            "release": reconciled.release.as_ref().map(|release| json!({
                "id": release.id,
                "tag_name": release.tag_name,
                "draft": release.draft,
                "prerelease": release.prerelease,
                "source": "current_forgejo_resource_api",
                "freshness": "current_api_read",
                "trust": "untrusted_user_content",
                "release_notes_included": false,
                "assets_included": false,
            })),
        }
    })
}

pub fn reconcile_inbox_event(
    event: &JournalEvent,
    source: &impl ForgejoCurrentState,
) -> Result<ReconciledState, ReconcileError> {
    let (delivery_id, event_name, resource) = parse_inbox_event(event)?;
    // Validate and project the event-sourced review note before any upstream
    // read. The canonical PR read below remains independent of this context.
    let review_notification = review_notification_from_event(&event_name, event)?;

    if matches!(
        resource.kind,
        ResourceKind::IssueComment | ResourceKind::PullRequestComment
    ) {
        return reconcile_issue_comment(delivery_id, resource, source);
    }

    if resource.kind == ResourceKind::WorkflowJob {
        let current = source
            .read_current(&resource)
            .map_err(ReconcileError::Source)?;
        return reconcile_workflow_job(delivery_id, resource, current, source);
    }

    let current = source
        .read_current(&resource)
        .map_err(ReconcileError::Source)?;
    if resource.kind == ResourceKind::Branch {
        let ResourceId::Branch(expected_branch) = &resource.id else {
            return Err(ReconcileError::InvalidCurrentStateField("resource.id"));
        };
        let current_branch = current
            .get("name")
            .and_then(Value::as_str)
            .ok_or(ReconcileError::InvalidCurrentStateField("name"))?;
        if current_branch != expected_branch {
            return Err(ReconcileError::ResourceMismatch);
        }
        let commit = current
            .get("commit")
            .and_then(Value::as_object)
            .ok_or(ReconcileError::InvalidCurrentStateField("commit"))?;
        let head_sha = commit
            .get("id")
            .and_then(Value::as_str)
            .filter(|sha| valid_nonzero_commit_sha(sha))
            .ok_or(ReconcileError::InvalidCurrentStateField("commit.id"))?;
        let updated_at = commit
            .get("timestamp")
            .and_then(Value::as_str)
            .filter(|value| valid_forgejo_timestamp(value))
            .ok_or(ReconcileError::InvalidCurrentStateField("commit.timestamp"))?;
        let html_url = source.expected_html_url(&resource);
        return Ok(ReconciledState {
            delivery_id,
            resource,
            state: "present".to_owned(),
            updated_at: updated_at.to_owned(),
            html_url,
            head_sha: Some(head_sha.to_owned()),
            parent_workflow_run: None,
            base_sha: None,
            merged: None,
            draft: None,
            merge_commit_sha: None,
            requested_reviewers: None,
            requested_reviewers_teams: None,
            labels: None,
            assignees: None,
            milestone: None,
            work_item_content: None,
            comment: None,
            review_notification: None,
            release: None,
        });
    }

    if resource.kind == ResourceKind::WorkflowRun {
        let snapshot = current_workflow_run_snapshot(&current, &resource, source)?;
        return Ok(ReconciledState {
            delivery_id,
            resource,
            state: snapshot.state,
            updated_at: snapshot.updated_at,
            html_url: snapshot.html_url,
            head_sha: Some(snapshot.head_sha),
            parent_workflow_run: None,
            base_sha: None,
            merged: None,
            draft: None,
            merge_commit_sha: None,
            requested_reviewers: None,
            requested_reviewers_teams: None,
            labels: None,
            assignees: None,
            milestone: None,
            work_item_content: None,
            comment: None,
            review_notification: None,
            release: None,
        });
    }

    if resource.kind == ResourceKind::Release {
        return reconcile_release(delivery_id, resource, current, source);
    }

    // Webhook payload fields such as state/action are deliberately ignored here.
    // They can be stale by the time this read reaches Forgejo.
    let ResourceId::Number(expected_number) = &resource.id else {
        return Err(ReconcileError::InvalidCurrentStateField("resource.id"));
    };
    let current_number = current
        .get("number")
        .and_then(Value::as_u64)
        .ok_or(ReconcileError::InvalidCurrentStateField("number"))?;
    if current_number != *expected_number {
        return Err(ReconcileError::ResourceMismatch);
    }
    if resource.kind == ResourceKind::Issue {
        validate_current_issue_repository(&current, &resource)?;
    }
    let state = current
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| matches!(*state, "open" | "closed"))
        .ok_or(ReconcileError::InvalidCurrentStateField("state"))?;
    let updated_at = current
        .get("updated_at")
        .and_then(Value::as_str)
        .filter(|value| valid_forgejo_timestamp(value))
        .ok_or(ReconcileError::InvalidCurrentStateField("updated_at"))?;
    let html_url = current
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ReconcileError::InvalidCurrentStateField("html_url"))?;
    if html_url != source.expected_html_url(&resource) {
        return Err(ReconcileError::ResourceMismatch);
    }

    let (
        head_sha,
        base_sha,
        merged,
        draft,
        merge_commit_sha,
        requested_reviewers,
        requested_reviewers_teams,
    ) = if resource.kind == ResourceKind::PullRequest {
        let (
            head_sha,
            base_sha,
            merged,
            draft,
            merge_commit_sha,
            requested_reviewers,
            requested_reviewers_teams,
        ) = current_pull_request_fields(&current, &resource, state)?;
        (
            Some(head_sha),
            Some(base_sha),
            Some(merged),
            Some(draft),
            merge_commit_sha,
            requested_reviewers,
            requested_reviewers_teams,
        )
    } else {
        (None, None, None, None, None, None, None)
    };
    let labels = current_work_item_labels(&current, &resource)?;
    let assignees = current_work_item_assignees(&current, &resource)?;
    let milestone = current_work_item_milestone(&current, &resource)?;
    let work_item_content = current_work_item_content(&current, &resource)?;

    Ok(ReconciledState {
        delivery_id: delivery_id.to_owned(),
        resource,
        state: state.to_owned(),
        updated_at: updated_at.to_owned(),
        html_url: html_url.to_owned(),
        head_sha,
        parent_workflow_run: None,
        base_sha,
        merged,
        draft,
        merge_commit_sha,
        requested_reviewers,
        requested_reviewers_teams,
        labels,
        assignees,
        milestone,
        work_item_content,
        comment: None,
        review_notification,
        release: None,
    })
}

fn review_notification_from_event(
    event_name: &str,
    event: &JournalEvent,
) -> Result<Option<ReconciledReviewNotification>, ReconcileError> {
    let review_type = match event_name {
        super::PULL_REQUEST_APPROVED_EVENT => super::PULL_REQUEST_REVIEW_APPROVED_TYPE,
        super::PULL_REQUEST_REJECTED_EVENT => super::PULL_REQUEST_REVIEW_REJECTED_TYPE,
        super::PULL_REQUEST_COMMENT_EVENT => super::PULL_REQUEST_REVIEW_COMMENT_TYPE,
        _ => return Ok(None),
    };
    let payload = event
        .payload
        .get("payload")
        .and_then(Value::as_object)
        .ok_or(ReconcileError::InvalidEnvelope)?;
    let review = payload
        .get("review")
        .and_then(Value::as_object)
        .ok_or(ReconcileError::InvalidEnvelope)?;
    if review.get("type").and_then(Value::as_str) != Some(review_type) {
        return Err(ReconcileError::InvalidEnvelope);
    }
    let (body, body_truncated, body_byte_length, body_sha256) = match review.get("content") {
        None | Some(Value::Null) => (None, false, None, None),
        Some(Value::String(source_body)) => {
            let body_byte_length = source_body.len();
            let body_sha256 = hex::encode(Sha256::digest(source_body.as_bytes()));
            if body_byte_length > MAX_REVIEW_NOTIFICATION_BODY_BYTES {
                let mut prefix_end = MAX_REVIEW_NOTIFICATION_BODY_BYTES;
                while !source_body.is_char_boundary(prefix_end) {
                    prefix_end -= 1;
                }
                (
                    Some(source_body[..prefix_end].to_owned()),
                    true,
                    Some(body_byte_length),
                    Some(body_sha256),
                )
            } else {
                (
                    Some(source_body.clone()),
                    false,
                    Some(body_byte_length),
                    Some(body_sha256),
                )
            }
        }
        Some(_) => return Err(ReconcileError::InvalidEventPayloadField("review.content")),
    };
    let pull_request_head_sha_at_event = match payload
        .get("pull_request")
        .and_then(Value::as_object)
        .and_then(|pull_request| pull_request.get("head"))
    {
        None | Some(Value::Null) => None,
        Some(Value::Object(head)) => match head.get("sha") {
            None | Some(Value::Null) => None,
            Some(Value::String(sha)) if valid_nonzero_commit_sha(sha) => Some(sha.clone()),
            Some(_) => {
                return Err(ReconcileError::InvalidEventPayloadField(
                    "pull_request.head.sha",
                ));
            }
        },
        Some(_) => {
            return Err(ReconcileError::InvalidEventPayloadField(
                "pull_request.head",
            ));
        }
    };
    Ok(Some(ReconciledReviewNotification {
        review_type,
        body,
        body_truncated,
        body_byte_length,
        body_sha256,
        pull_request_head_sha_at_event,
    }))
}

fn reconcile_release(
    delivery_id: String,
    resource: ResourceRef,
    current: Value,
    source: &impl ForgejoCurrentState,
) -> Result<ReconciledState, ReconcileError> {
    let ResourceId::Release(expected_id) = &resource.id else {
        return Err(ReconcileError::InvalidCurrentStateField("resource.id"));
    };
    let current_id = current
        .get("id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or(ReconcileError::InvalidCurrentStateField("id"))?;
    if current_id != *expected_id {
        return Err(ReconcileError::ResourceMismatch);
    }
    let tag_name = current
        .get("tag_name")
        .and_then(Value::as_str)
        .filter(|tag| {
            !tag.is_empty()
                && tag.len() <= MAX_RECONCILED_RELEASE_TAG_NAME_BYTES
                && crate::forgejo::valid_tag_ref_name(tag)
        })
        .ok_or(ReconcileError::InvalidCurrentStateField("tag_name"))?;
    let draft = current
        .get("draft")
        .and_then(Value::as_bool)
        .ok_or(ReconcileError::InvalidCurrentStateField("draft"))?;
    let prerelease = current
        .get("prerelease")
        .and_then(Value::as_bool)
        .ok_or(ReconcileError::InvalidCurrentStateField("prerelease"))?;
    let current_html_url = current
        .get("html_url")
        .and_then(Value::as_str)
        .ok_or(ReconcileError::InvalidCurrentStateField("html_url"))?;
    let html_url = source.expected_release_html_url(&resource, tag_name);
    if html_url.is_empty() || current_html_url != html_url {
        return Err(ReconcileError::ResourceMismatch);
    }

    Ok(ReconciledState {
        delivery_id,
        resource,
        state: "present".to_owned(),
        // Forgejo v17's release response has created_at/published_at but no
        // updated_at. Do not mislabel either timestamp as an update time.
        updated_at: String::new(),
        html_url,
        head_sha: None,
        parent_workflow_run: None,
        base_sha: None,
        merged: None,
        draft: None,
        merge_commit_sha: None,
        requested_reviewers: None,
        requested_reviewers_teams: None,
        labels: None,
        assignees: None,
        milestone: None,
        work_item_content: None,
        comment: None,
        review_notification: None,
        release: Some(ReconciledRelease {
            id: current_id,
            tag_name: tag_name.to_owned(),
            draft,
            prerelease,
        }),
    })
}

type CurrentWorkflowRunSnapshot = ReconciledWorkflowRun;

fn current_workflow_run_snapshot(
    current: &Value,
    resource: &ResourceRef,
    source: &impl ForgejoCurrentState,
) -> Result<CurrentWorkflowRunSnapshot, ReconcileError> {
    let ResourceId::WorkflowRun {
        run_id: expected_run_id,
        index_in_repo: expected_index,
    } = &resource.id
    else {
        return Err(ReconcileError::InvalidCurrentStateField("resource.id"));
    };
    let current_run_id = current
        .get("id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or(ReconcileError::InvalidCurrentStateField("id"))?;
    let current_index = current
        .get("index_in_repo")
        .and_then(Value::as_u64)
        .filter(|index| *index > 0)
        .ok_or(ReconcileError::InvalidCurrentStateField("index_in_repo"))?;
    if current_run_id != *expected_run_id || current_index != *expected_index {
        return Err(ReconcileError::ResourceMismatch);
    }
    validate_current_workflow_run_repository(current, resource)?;
    let state = current
        .get("status")
        .and_then(Value::as_str)
        .filter(|status| valid_workflow_run_status(status))
        .ok_or(ReconcileError::InvalidCurrentStateField("status"))?;
    let updated_at = current
        .get("updated")
        .and_then(Value::as_str)
        .filter(|value| valid_forgejo_timestamp(value))
        .ok_or(ReconcileError::InvalidCurrentStateField("updated"))?;
    let head_sha = current
        .get("commit_sha")
        .and_then(Value::as_str)
        .filter(|sha| valid_nonzero_commit_sha(sha))
        .ok_or(ReconcileError::InvalidCurrentStateField("commit_sha"))?;
    let html_url = source.expected_html_url(resource);
    if current.get("html_url").and_then(Value::as_str) != Some(html_url.as_str()) {
        return Err(ReconcileError::ResourceMismatch);
    }
    Ok(CurrentWorkflowRunSnapshot {
        run_id: *expected_run_id,
        index_in_repo: *expected_index,
        state: state.to_owned(),
        updated_at: updated_at.to_owned(),
        html_url,
        head_sha: head_sha.to_owned(),
    })
}

fn reconcile_workflow_job(
    delivery_id: String,
    resource: ResourceRef,
    current_job: Value,
    source: &impl ForgejoCurrentState,
) -> Result<ReconciledState, ReconcileError> {
    let ResourceId::WorkflowJob {
        job_id: expected_job_id,
        run_id,
        index_in_repo,
        job_index: expected_job_index,
    } = &resource.id
    else {
        return Err(ReconcileError::InvalidCurrentStateField("resource.id"));
    };
    let current_job_id = current_job
        .get("id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or(ReconcileError::InvalidCurrentStateField("id"))?;
    let current_run_id = current_job
        .get("run_id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or(ReconcileError::InvalidCurrentStateField("run_id"))?;
    if current_job_id != *expected_job_id || current_run_id != *run_id {
        return Err(ReconcileError::ResourceMismatch);
    }
    let state = current_job
        .get("status")
        .and_then(Value::as_str)
        .filter(|status| valid_workflow_run_status(status))
        .ok_or(ReconcileError::InvalidCurrentStateField("status"))?;

    let parent = ResourceRef {
        kind: ResourceKind::WorkflowRun,
        owner: resource.owner.clone(),
        repository: resource.repository.clone(),
        id: ResourceId::WorkflowRun {
            run_id: *run_id,
            index_in_repo: *index_in_repo,
        },
    };
    let current_run = source.read_current(&parent).map_err(|error| match error {
        ApiReadError::NotFound => ReconcileError::ParentWorkflowRunNotFound {
            run_id: *run_id,
            index_in_repo: *index_in_repo,
        },
        error => ReconcileError::Source(error),
    })?;
    let run = current_workflow_run_snapshot(&current_run, &parent, source)?;
    let html_url = current_job
        .get("html_url")
        .and_then(Value::as_str)
        .ok_or(ReconcileError::InvalidCurrentStateField("html_url"))?;
    let current_job_index = workflow_job_url_index_within_run(html_url, &run.html_url)
        .ok_or(ReconcileError::InvalidCurrentStateField("html_url"))?;
    if current_job_index != *expected_job_index {
        return Err(ReconcileError::InvalidCurrentStateField("html_url"));
    }

    Ok(ReconciledState {
        delivery_id,
        resource,
        state: state.to_owned(),
        // Forgejo's job response has no updated timestamp; expose the current
        // parent run timestamp and SHA, which are read independently above.
        updated_at: run.updated_at.clone(),
        html_url: html_url.to_owned(),
        head_sha: Some(run.head_sha.clone()),
        parent_workflow_run: Some(run),
        base_sha: None,
        merged: None,
        draft: None,
        merge_commit_sha: None,
        requested_reviewers: None,
        requested_reviewers_teams: None,
        labels: None,
        assignees: None,
        milestone: None,
        work_item_content: None,
        comment: None,
        review_notification: None,
        release: None,
    })
}

fn workflow_job_url_index_within_run(job_url: &str, run_url: &str) -> Option<u64> {
    let Some(suffix) = job_url.strip_prefix(run_url) else {
        return None;
    };
    let Some(suffix) = suffix.strip_prefix("/jobs/") else {
        return None;
    };
    workflow_job_index_from_suffix(suffix)
}

fn workflow_job_index_from_event_url(job_url: &str, index_in_repo: u64) -> Option<u64> {
    // Treat the signed URL as an identity hint only; callers must not follow
    // it or project its origin as a canonical link.
    let run_marker = format!("/actions/runs/{index_in_repo}/jobs/");
    let (_, suffix) = job_url.rsplit_once(&run_marker)?;
    workflow_job_index_from_suffix(suffix)
}

fn workflow_job_index_from_suffix(suffix: &str) -> Option<u64> {
    let Some((job_index, attempt)) = suffix.split_once("/attempt/") else {
        return None;
    };
    let parsed_index = job_index.parse::<u64>().ok()?;
    if parsed_index.to_string() != job_index {
        return None;
    }
    let parsed_attempt = attempt.parse::<u64>().ok()?;
    if parsed_attempt == 0 || parsed_attempt.to_string() != attempt {
        return None;
    }
    Some(parsed_index)
}

fn reconcile_issue_comment(
    delivery_id: String,
    resource: ResourceRef,
    source: &impl ForgejoCurrentState,
) -> Result<ReconciledState, ReconcileError> {
    let ResourceId::IssueComment {
        issue_number,
        comment_id,
        deleted_event,
    } = &resource.id
    else {
        return Err(ReconcileError::InvalidCurrentStateField("resource.id"));
    };
    let parent_kind = match resource.kind {
        ResourceKind::IssueComment => ResourceKind::Issue,
        ResourceKind::PullRequestComment => ResourceKind::PullRequest,
        _ => return Err(ReconcileError::InvalidCurrentStateField("resource.kind")),
    };
    let parent = ResourceRef {
        kind: parent_kind,
        owner: resource.owner.clone(),
        repository: resource.repository.clone(),
        id: ResourceId::Number(*issue_number),
    };
    let mut current_issue = source
        .read_current(&parent)
        .map_err(ReconcileError::Source)?;
    let mut parent_snapshot =
        current_comment_parent_snapshot(&current_issue, &parent, *issue_number, source)?;

    let comment = match source.read_current(&resource) {
        Err(ApiReadError::NotFound) if *deleted_event => {
            current_issue = source
                .read_current_fresh(&parent)
                .map_err(ReconcileError::Source)?;
            parent_snapshot =
                current_comment_parent_snapshot(&current_issue, &parent, *issue_number, source)?;
            ReconciledComment {
                id: *comment_id,
                state: "absent".to_owned(),
                updated_at: None,
                html_url: None,
                body: None,
                body_trust: "untrusted_user_content",
            }
        }
        Err(error) => return Err(ReconcileError::Source(error)),
        Ok(current_comment) => {
            let current_id = current_comment
                .get("id")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or(ReconcileError::InvalidCurrentStateField("comment.id"))?;
            if current_id != *comment_id {
                return Err(ReconcileError::ResourceMismatch);
            }
            let (expected_parent_field, other_parent_field) = match resource.kind {
                ResourceKind::IssueComment => ("issue_url", "pull_request_url"),
                ResourceKind::PullRequestComment => ("pull_request_url", "issue_url"),
                _ => return Err(ReconcileError::InvalidCurrentStateField("resource.kind")),
            };
            let current_parent_url = current_comment
                .get(expected_parent_field)
                .and_then(Value::as_str)
                .ok_or(ReconcileError::InvalidCurrentStateField(
                    match resource.kind {
                        ResourceKind::IssueComment => "comment.issue_url",
                        ResourceKind::PullRequestComment => "comment.pull_request_url",
                        _ => "resource.kind",
                    },
                ))?;
            if current_parent_url != parent_snapshot.html_url {
                return Err(ReconcileError::ResourceMismatch);
            }
            match current_comment.get(other_parent_field) {
                None | Some(Value::Null) => {}
                Some(Value::String(value)) if value.is_empty() => {}
                Some(Value::String(_)) => return Err(ReconcileError::ResourceMismatch),
                Some(_) => {
                    return Err(ReconcileError::InvalidCurrentStateField(
                        match resource.kind {
                            ResourceKind::IssueComment => "comment.pull_request_url",
                            ResourceKind::PullRequestComment => "comment.issue_url",
                            _ => "resource.kind",
                        },
                    ));
                }
            }
            let comment_html_url = current_comment
                .get("html_url")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or(ReconcileError::InvalidCurrentStateField("comment.html_url"))?;
            let expected_comment_html_url =
                format!("{}#issuecomment-{comment_id}", parent_snapshot.html_url);
            if comment_html_url != expected_comment_html_url {
                return Err(ReconcileError::ResourceMismatch);
            }
            let comment_updated_at = current_comment
                .get("updated_at")
                .and_then(Value::as_str)
                .filter(|value| valid_forgejo_timestamp(value))
                .ok_or(ReconcileError::InvalidCurrentStateField(
                    "comment.updated_at",
                ))?;
            let body = current_comment
                .get("body")
                .and_then(Value::as_str)
                .ok_or(ReconcileError::InvalidCurrentStateField("comment.body"))?;
            ReconciledComment {
                id: *comment_id,
                state: "present".to_owned(),
                updated_at: Some(comment_updated_at.to_owned()),
                html_url: Some(comment_html_url.to_owned()),
                body: Some(body.to_owned()),
                body_trust: "untrusted_user_content",
            }
        }
    };

    let (
        head_sha,
        base_sha,
        merged,
        draft,
        merge_commit_sha,
        requested_reviewers,
        requested_reviewers_teams,
    ) = if parent_kind == ResourceKind::PullRequest {
        let (
            head_sha,
            base_sha,
            merged,
            draft,
            merge_commit_sha,
            requested_reviewers,
            requested_reviewers_teams,
        ) = current_pull_request_fields(&current_issue, &parent, &parent_snapshot.state)?;
        (
            Some(head_sha),
            Some(base_sha),
            Some(merged),
            Some(draft),
            merge_commit_sha,
            requested_reviewers,
            requested_reviewers_teams,
        )
    } else {
        (None, None, None, None, None, None, None)
    };

    Ok(ReconciledState {
        delivery_id,
        resource,
        state: parent_snapshot.state,
        updated_at: parent_snapshot.updated_at,
        html_url: parent_snapshot.html_url,
        head_sha,
        parent_workflow_run: None,
        base_sha,
        merged,
        draft,
        merge_commit_sha,
        requested_reviewers,
        requested_reviewers_teams,
        labels: parent_snapshot.labels,
        assignees: parent_snapshot.assignees,
        milestone: parent_snapshot.milestone,
        work_item_content: parent_snapshot.work_item_content,
        comment: Some(comment),
        review_notification: None,
        release: None,
    })
}

struct CurrentCommentParentSnapshot {
    state: String,
    updated_at: String,
    html_url: String,
    labels: Option<Vec<ReconciledLabel>>,
    assignees: Option<Vec<ReconciledAssignee>>,
    milestone: Option<ReconciledMilestoneObservation>,
    work_item_content: Option<ReconciledWorkItemContent>,
}

fn current_comment_parent_snapshot(
    current: &Value,
    resource: &ResourceRef,
    expected_number: u64,
    source: &impl ForgejoCurrentState,
) -> Result<CurrentCommentParentSnapshot, ReconcileError> {
    let number = current
        .get("number")
        .and_then(Value::as_u64)
        .ok_or(ReconcileError::InvalidCurrentStateField("number"))?;
    if number != expected_number {
        return Err(ReconcileError::ResourceMismatch);
    }
    validate_current_issue_repository(current, resource)?;
    let state = current
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| matches!(*state, "open" | "closed"))
        .ok_or(ReconcileError::InvalidCurrentStateField("state"))?;
    let updated_at = current
        .get("updated_at")
        .and_then(Value::as_str)
        .filter(|value| valid_forgejo_timestamp(value))
        .ok_or(ReconcileError::InvalidCurrentStateField("updated_at"))?;
    let html_url = current
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ReconcileError::InvalidCurrentStateField("html_url"))?;
    if html_url != source.expected_html_url(resource) {
        return Err(ReconcileError::ResourceMismatch);
    }
    Ok(CurrentCommentParentSnapshot {
        state: state.to_owned(),
        updated_at: updated_at.to_owned(),
        html_url: html_url.to_owned(),
        labels: current_work_item_labels(current, resource)?,
        assignees: current_work_item_assignees(current, resource)?,
        milestone: current_work_item_milestone(current, resource)?,
        work_item_content: current_work_item_content(current, resource)?,
    })
}

fn current_work_item_labels(
    current: &Value,
    resource: &ResourceRef,
) -> Result<Option<Vec<ReconciledLabel>>, ReconcileError> {
    if !matches!(
        resource.kind,
        ResourceKind::Issue | ResourceKind::PullRequest
    ) {
        return Ok(None);
    }
    let values = match current.get("labels") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(values)) => values,
        Some(_) => return Err(ReconcileError::InvalidCurrentStateField("labels")),
    };
    if values.len() > MAX_RECONCILED_LABELS {
        return Err(ReconcileError::InvalidCurrentStateField("labels"));
    }

    let mut seen_ids = BTreeSet::new();
    let mut labels = Vec::with_capacity(values.len());
    for value in values {
        let id = value
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ReconcileError::InvalidCurrentStateField("labels"))?;
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| {
                !name.trim().is_empty()
                    && name.len() <= MAX_RECONCILED_LABEL_NAME_BYTES
                    && !name.chars().any(char::is_control)
            })
            .ok_or(ReconcileError::InvalidCurrentStateField("labels"))?;
        if !seen_ids.insert(id) {
            return Err(ReconcileError::InvalidCurrentStateField("labels"));
        }
        labels.push(ReconciledLabel {
            id,
            name: name.to_owned(),
        });
    }
    labels.sort_unstable_by_key(|label| label.id);
    Ok(Some(labels))
}

pub const MAX_RECONCILED_ASSIGNEES: usize = 256;
pub const MAX_RECONCILED_ASSIGNEE_LOGIN_BYTES: usize = 255;

fn current_work_item_assignees(
    current: &Value,
    resource: &ResourceRef,
) -> Result<Option<Vec<ReconciledAssignee>>, ReconcileError> {
    if !matches!(
        resource.kind,
        ResourceKind::Issue | ResourceKind::PullRequest
    ) {
        return Ok(None);
    }
    let values = match current.get("assignees") {
        None => return Ok(None),
        Some(Value::Null) if resource.kind == ResourceKind::Issue => {
            return Ok(Some(Vec::new()));
        }
        Some(Value::Null) => return Ok(None),
        Some(Value::Array(values)) => values,
        Some(_) => return Err(ReconcileError::InvalidCurrentStateField("assignees")),
    };
    if values.len() > MAX_RECONCILED_ASSIGNEES {
        return Err(ReconcileError::InvalidCurrentStateField("assignees"));
    }

    let mut seen_ids = BTreeSet::new();
    let mut assignees = Vec::with_capacity(values.len());
    for value in values {
        let id = value
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ReconcileError::InvalidCurrentStateField("assignees"))?;
        let login = value
            .get("login")
            .and_then(Value::as_str)
            .filter(|login| {
                !login.trim().is_empty()
                    && login.len() <= MAX_RECONCILED_ASSIGNEE_LOGIN_BYTES
                    && !login.chars().any(char::is_control)
            })
            .ok_or(ReconcileError::InvalidCurrentStateField("assignees"))?;
        if !seen_ids.insert(id) {
            return Err(ReconcileError::InvalidCurrentStateField("assignees"));
        }
        assignees.push(ReconciledAssignee {
            id,
            login: login.to_owned(),
        });
    }
    assignees.sort_unstable_by_key(|assignee| assignee.id);
    Ok(Some(assignees))
}

pub const MAX_RECONCILED_MILESTONE_TITLE_BYTES: usize = 1024;

fn current_work_item_milestone(
    current: &Value,
    resource: &ResourceRef,
) -> Result<Option<ReconciledMilestoneObservation>, ReconcileError> {
    if !matches!(
        resource.kind,
        ResourceKind::Issue | ResourceKind::PullRequest
    ) {
        return Ok(None);
    }

    let observation = match current.get("milestone") {
        None => ReconciledMilestoneObservation::Unknown,
        Some(Value::Null) => ReconciledMilestoneObservation::Unassigned,
        Some(Value::Object(milestone)) => {
            let id = milestone
                .get("id")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or(ReconcileError::InvalidCurrentStateField("milestone"))?;
            let title = milestone
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| {
                    !title.trim().is_empty()
                        && title.len() <= MAX_RECONCILED_MILESTONE_TITLE_BYTES
                        && !title.chars().any(char::is_control)
                })
                .ok_or(ReconcileError::InvalidCurrentStateField("milestone"))?;
            let state = milestone
                .get("state")
                .and_then(Value::as_str)
                .filter(|state| matches!(*state, "open" | "closed"))
                .ok_or(ReconcileError::InvalidCurrentStateField("milestone"))?;
            ReconciledMilestoneObservation::Assigned(ReconciledMilestone {
                id,
                title: title.to_owned(),
                state: state.to_owned(),
            })
        }
        Some(_) => return Err(ReconcileError::InvalidCurrentStateField("milestone")),
    };

    Ok(Some(observation))
}

pub const MAX_WORK_ITEM_TITLE_BYTES: usize = 1024;
pub const MAX_WORK_ITEM_BODY_BYTES: usize = 65_536;

fn current_work_item_content(
    current: &Value,
    resource: &ResourceRef,
) -> Result<Option<ReconciledWorkItemContent>, ReconcileError> {
    let source = match resource.kind {
        ResourceKind::Issue => "current_forgejo_issue_api",
        ResourceKind::PullRequest => "current_forgejo_pull_request_api",
        _ => return Ok(None),
    };
    let title = match current.get("title") {
        None => None,
        Some(value) => {
            let title = value
                .as_str()
                .ok_or(ReconcileError::InvalidCurrentStateField("title"))?;
            if title.len() > MAX_WORK_ITEM_TITLE_BYTES {
                return Ok(None);
            }
            if title.trim().is_empty() || title.chars().any(char::is_control) {
                return Err(ReconcileError::InvalidCurrentStateField("title"));
            }
            Some(title)
        }
    };
    let body = match current.get("body") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(body)) if body.len() <= MAX_WORK_ITEM_BODY_BYTES => {
            Some(Some(body.as_str()))
        }
        Some(Value::String(_)) => return Ok(None),
        Some(_) => return Err(ReconcileError::InvalidCurrentStateField("body")),
    };
    let (Some(title), Some(body)) = (title, body) else {
        return Ok(None);
    };
    Ok(Some(ReconciledWorkItemContent {
        title: title.to_owned(),
        body: body.map(str::to_owned),
        source,
        trust: "untrusted_user_content",
    }))
}

fn current_pull_request_fields(
    current: &Value,
    resource: &ResourceRef,
    state: &str,
) -> Result<
    (
        String,
        String,
        bool,
        bool,
        Option<String>,
        Option<Vec<ReconciledReviewer>>,
        Option<Vec<ReconciledReviewerTeam>>,
    ),
    ReconcileError,
> {
    let merged = current
        .get("merged")
        .and_then(Value::as_bool)
        .ok_or(ReconcileError::InvalidCurrentStateField("merged"))?;
    let draft = current
        .get("draft")
        .and_then(Value::as_bool)
        .ok_or(ReconcileError::InvalidCurrentStateField("draft"))?;
    if merged && state != "closed" {
        return Err(ReconcileError::InvalidCurrentStateField(
            "merged_state_consistency",
        ));
    }
    let head_sha = current
        .get("head")
        .and_then(|head| head.get("sha"))
        .and_then(Value::as_str)
        .filter(|sha| valid_nonzero_commit_sha(sha))
        .ok_or(ReconcileError::InvalidCurrentStateField("head.sha"))?;
    let base_sha = current
        .get("base")
        .and_then(|base| base.get("sha"))
        .and_then(Value::as_str)
        .filter(|sha| valid_nonzero_commit_sha(sha))
        .ok_or(ReconcileError::InvalidCurrentStateField("base.sha"))?;
    validate_current_pull_request_base_repository(current, resource)?;
    let merge_commit_sha = if merged {
        Some(
            current
                .get("merge_commit_sha")
                .and_then(Value::as_str)
                .filter(|sha| valid_nonzero_commit_sha(sha))
                .ok_or(ReconcileError::InvalidCurrentStateField("merge_commit_sha"))?
                .to_owned(),
        )
    } else {
        match current.get("merge_commit_sha") {
            None | Some(Value::Null) => None,
            Some(_) => return Err(ReconcileError::InvalidCurrentStateField("merge_commit_sha")),
        }
    };
    let requested_reviewers = current_pull_request_reviewers(current)?;
    let requested_reviewers_teams = current_pull_request_reviewer_teams(current)?;
    Ok((
        head_sha.to_owned(),
        base_sha.to_owned(),
        merged,
        draft,
        merge_commit_sha,
        requested_reviewers,
        requested_reviewers_teams,
    ))
}

fn current_pull_request_reviewer_teams(
    current: &Value,
) -> Result<Option<Vec<ReconciledReviewerTeam>>, ReconcileError> {
    let Some(value) = current.get("requested_reviewers_teams") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let teams = value
        .as_array()
        .ok_or(ReconcileError::InvalidCurrentStateField(
            "requested_reviewers_teams",
        ))?;
    if teams.len() > MAX_RECONCILED_REQUESTED_REVIEWER_TEAMS {
        return Err(ReconcileError::InvalidCurrentStateField(
            "requested_reviewers_teams",
        ));
    }
    let mut parsed = Vec::with_capacity(teams.len());
    for team in teams {
        let id = team
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ReconcileError::InvalidCurrentStateField(
                "requested_reviewers_teams",
            ))?;
        let name = team
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| {
                !name.is_empty()
                    && name.chars().count() <= MAX_RECONCILED_REVIEWER_TEAM_NAME_CHARS
                    && !name.chars().any(char::is_control)
            })
            .ok_or(ReconcileError::InvalidCurrentStateField(
                "requested_reviewers_teams",
            ))?;
        parsed.push(ReconciledReviewerTeam {
            id,
            name: name.to_owned(),
        });
    }
    parsed.sort_by_key(|team| team.id);
    if parsed.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(ReconcileError::InvalidCurrentStateField(
            "requested_reviewers_teams",
        ));
    }
    Ok(Some(parsed))
}

fn current_pull_request_reviewers(
    current: &Value,
) -> Result<Option<Vec<ReconciledReviewer>>, ReconcileError> {
    let Some(value) = current.get("requested_reviewers") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let reviewers = value
        .as_array()
        .ok_or(ReconcileError::InvalidCurrentStateField(
            "requested_reviewers",
        ))?;
    if reviewers.len() > MAX_RECONCILED_REQUESTED_REVIEWERS {
        return Err(ReconcileError::InvalidCurrentStateField(
            "requested_reviewers",
        ));
    }
    let mut parsed = Vec::with_capacity(reviewers.len());
    for reviewer in reviewers {
        let id = reviewer
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ReconcileError::InvalidCurrentStateField(
                "requested_reviewers",
            ))?;
        let login = reviewer
            .get("login")
            .and_then(Value::as_str)
            .filter(|login| !login.is_empty() && !login.chars().any(char::is_control))
            .ok_or(ReconcileError::InvalidCurrentStateField(
                "requested_reviewers",
            ))?;
        parsed.push(ReconciledReviewer {
            id,
            login: login.to_owned(),
        });
    }
    parsed.sort_by_key(|reviewer| reviewer.id);
    if parsed.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(ReconcileError::InvalidCurrentStateField(
            "requested_reviewers",
        ));
    }
    Ok(Some(parsed))
}

/// Parse and validate a journal event's webhook resource without making an API
/// request. Hosts can reject out-of-scope resources before performing I/O.
pub fn event_resource_ref(event: &JournalEvent) -> Result<ResourceRef, ReconcileError> {
    parse_inbox_event(event).map(|(_, _, resource)| resource)
}

/// Validate that a supported event header names the resource contained in its
/// signed JSON body. Ingestion calls this before appending to the durable inbox;
/// replay still validates the persisted envelope independently.
pub(crate) fn validate_event_payload(
    event_name: &str,
    payload: &Value,
) -> Result<(), ReconcileError> {
    let payload = payload.as_object().ok_or(ReconcileError::InvalidEnvelope)?;
    validate_event_payload_kind(event_name, payload)?;
    let kind =
        resource_kind_for_event(event_name, payload).ok_or(ReconcileError::UnsupportedEvent)?;
    let repository_identity_field = repository_identity_field(kind);
    validate_event_repository_identity(kind, payload, repository_identity_field)?;
    match parse_resource_ref(event_name, kind, payload) {
        Ok(_) => Ok(()),
        Err(ReconcileError::InvalidRepository) => Err(ReconcileError::InvalidEventPayloadField(
            repository_identity_field,
        )),
        Err(error) => Err(error),
    }
}

fn repository_identity_field(kind: ResourceKind) -> &'static str {
    if matches!(kind, ResourceKind::WorkflowRun | ResourceKind::WorkflowJob) {
        "run.repository.full_name"
    } else {
        "repository.full_name"
    }
}

fn validate_event_repository_identity(
    kind: ResourceKind,
    payload: &Map<String, Value>,
    field: &'static str,
) -> Result<(), ReconcileError> {
    let repository_payload =
        (if matches!(kind, ResourceKind::WorkflowRun | ResourceKind::WorkflowJob) {
            payload.get("run").and_then(|run| run.get("repository"))
        } else {
            payload.get("repository")
        })
        .and_then(Value::as_object)
        .ok_or(ReconcileError::InvalidEventPayloadField(field))?;
    let full_name = repository_payload
        .get("full_name")
        .and_then(Value::as_str)
        .ok_or(ReconcileError::InvalidEventPayloadField(field))?;
    let (owner, repository) = full_name
        .split_once('/')
        .filter(|(owner, repository)| {
            !repository.contains('/')
                && valid_repo_component(owner)
                && valid_repo_component(repository)
        })
        .ok_or(ReconcileError::InvalidEventPayloadField(field))?;

    match validate_repository_identity(repository_payload, owner, repository) {
        Ok(()) => Ok(()),
        Err(ReconcileError::InvalidRepository | ReconcileError::ResourceMismatch) => {
            Err(ReconcileError::InvalidEventPayloadField(field))
        }
        Err(error) => Err(error),
    }
}

fn parse_inbox_event(
    event: &JournalEvent,
) -> Result<(String, String, ResourceRef), ReconcileError> {
    let event_name = match event.kind.strip_prefix(super::JOURNAL_KIND_PREFIX) {
        Some(event_name) if super::SUPPORTED_EVENTS.contains(&event_name) => event_name,
        _ => return Err(ReconcileError::UnsupportedEvent),
    };
    let envelope = &event.payload;
    let delivery_id = envelope
        .get("delivery_id")
        .and_then(Value::as_str)
        .ok_or(ReconcileError::InvalidEnvelope)?;
    if envelope.get("schema_version").and_then(Value::as_u64) != Some(1)
        || envelope.get("provider").and_then(Value::as_str) != Some("forgejo")
        || envelope.get("event").and_then(Value::as_str) != Some(event_name)
        || !valid_delivery_id(delivery_id)
        || event.idempotency_key != format!("forgejo-delivery-v1:{delivery_id}")
    {
        return Err(ReconcileError::InvalidEnvelope);
    }

    let payload = envelope
        .get("payload")
        .and_then(Value::as_object)
        .ok_or(ReconcileError::InvalidEnvelope)?;
    validate_event_payload_kind(event_name, payload)?;
    let kind =
        resource_kind_for_event(event_name, payload).ok_or(ReconcileError::UnsupportedEvent)?;
    let resource = parse_resource_ref(event_name, kind, payload)?;
    Ok((delivery_id.to_owned(), event_name.to_owned(), resource))
}

fn validate_event_payload_kind(
    event_name: &str,
    payload: &Map<String, Value>,
) -> Result<(), ReconcileError> {
    if let Some(action_contract) = event_action_contract_for_event(event_name) {
        let action = payload_string_at_path(payload, action_contract.payload_field)
            .ok_or(ReconcileError::InvalidEventPayloadField("action"))?;
        if !action_contract.allowed_actions.contains(&action) {
            return Err(ReconcileError::InvalidEventPayloadField("action"));
        }
    }

    if let Some(status_contract) = workflow_status_contract_for_event(event_name, "workflow_job") {
        let action_contract = workflow_action_contract_for_resource_kind("workflow_job")
            .ok_or(ReconcileError::UnsupportedEvent)?;
        let event_action_contract =
            workflow_event_action_contract_for_event(event_name, "workflow_job")
                .ok_or(ReconcileError::UnsupportedEvent)?;
        let action = payload_string_at_path(payload, action_contract.payload_field)
            .ok_or(ReconcileError::InvalidEventPayloadField("action"))?;
        if !action_contract.allowed_actions.contains(&action)
            || !event_action_contract.allowed_actions.contains(&action)
        {
            return Err(ReconcileError::InvalidEventPayloadField("action"));
        }
        let job = payload
            .get("job")
            .and_then(Value::as_object)
            .ok_or(ReconcileError::InvalidEventPayloadField("job"))?;
        if payload_string_at_path(payload, status_contract.payload_field)
            != Some(status_contract.expected_status)
        {
            return Err(ReconcileError::InvalidEventPayloadField("job.status"));
        }
        let job_run_id = job
            .get("run_id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ReconcileError::InvalidEventPayloadField("job.run_id"))?;
        let run = payload
            .get("run")
            .and_then(Value::as_object)
            .ok_or(ReconcileError::InvalidEventPayloadField("run"))?;
        let run_id = run
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ReconcileError::InvalidEventPayloadField("run.id"))?;
        let run_index = run
            .get("index_in_repo")
            .and_then(Value::as_u64)
            .filter(|index| *index > 0)
            .ok_or(ReconcileError::InvalidEventPayloadField(
                "run.index_in_repo",
            ))?;
        if job_run_id != run_id {
            return Err(ReconcileError::ResourceMismatch);
        }
        if !job
            .get("html_url")
            .and_then(Value::as_str)
            .and_then(|url| workflow_job_index_from_event_url(url, run_index))
            .is_some()
        {
            return Err(ReconcileError::InvalidEventPayloadField("job.html_url"));
        }
        for (field, name) in [("id", "job.id"), ("index_in_repo", "run.index_in_repo")] {
            let value = if name == "job.id" { job } else { run };
            if !value
                .get(field)
                .and_then(Value::as_u64)
                .is_some_and(|id| id > 0)
            {
                return Err(ReconcileError::InvalidEventPayloadField(name));
            }
        }
        reject_unexpected_event_payload_fields(
            payload,
            &[
                "repository",
                "issue",
                "pull_request",
                "comment",
                "review",
                "ref",
            ],
        )?;
        return Ok(());
    }

    if let Some(status_contract) = workflow_status_contract_for_event(event_name, "workflow_run") {
        let action_contract = workflow_action_contract_for_resource_kind("workflow_run")
            .ok_or(ReconcileError::UnsupportedEvent)?;
        let event_action_contract =
            workflow_event_action_contract_for_event(event_name, "workflow_run")
                .ok_or(ReconcileError::UnsupportedEvent)?;
        let action = payload_string_at_path(payload, action_contract.payload_field)
            .ok_or(ReconcileError::InvalidEventPayloadField("action"))?;
        if !action_contract.allowed_actions.contains(&action)
            || !event_action_contract.allowed_actions.contains(&action)
        {
            return Err(ReconcileError::InvalidEventPayloadField("action"));
        }
        let run = payload
            .get("run")
            .and_then(Value::as_object)
            .ok_or(ReconcileError::InvalidEventPayloadField("run"))?;
        if payload_string_at_path(payload, status_contract.payload_field)
            != Some(status_contract.expected_status)
        {
            return Err(ReconcileError::InvalidEventPayloadField("run.status"));
        }
        if !run
            .get("id")
            .and_then(Value::as_u64)
            .is_some_and(|id| id > 0)
        {
            return Err(ReconcileError::InvalidEventPayloadField("run.id"));
        }
        if !run
            .get("index_in_repo")
            .and_then(Value::as_u64)
            .is_some_and(|index| index > 0)
        {
            return Err(ReconcileError::InvalidEventPayloadField(
                "run.index_in_repo",
            ));
        }
        if !run
            .get("commit_sha")
            .and_then(Value::as_str)
            .is_some_and(valid_nonzero_commit_sha)
        {
            return Err(ReconcileError::InvalidEventPayloadField("run.commit_sha"));
        }
        if !run
            .get("updated")
            .and_then(Value::as_str)
            .is_some_and(valid_forgejo_timestamp)
        {
            return Err(ReconcileError::InvalidEventPayloadField("run.updated"));
        }
        reject_unexpected_event_payload_fields(
            payload,
            &[
                "repository",
                "issue",
                "pull_request",
                "comment",
                "review",
                "ref",
            ],
        )?;
        return Ok(());
    }

    if event_name == super::CREATE_EVENT || event_name == super::DELETE_EVENT {
        // Forgejo signs the payload but not X-Forgejo-Event. The create-only
        // SHA field makes a signed create body distinguishable from delete;
        // accepting it under the delete header would reclassify the event.
        if event_name == super::DELETE_EVENT && payload.contains_key("sha") {
            return Err(ReconcileError::UnexpectedEventPayloadField("sha"));
        }
        let ref_type = payload
            .get("ref_type")
            .and_then(Value::as_str)
            .ok_or(ReconcileError::InvalidEventPayloadField("ref_type"))?;
        if ref_type != "branch" {
            return Err(ReconcileError::UnsupportedEventPayloadField("ref_type"));
        }
        let branch_name = payload
            .get("ref")
            .and_then(Value::as_str)
            .ok_or(ReconcileError::InvalidEventPayloadField("ref"))?;
        if !crate::forgejo::valid_branch_ref_name(branch_name) {
            return Err(ReconcileError::InvalidEventPayloadField("ref"));
        }
        if event_name == super::CREATE_EVENT {
            let sha = payload
                .get("sha")
                .and_then(Value::as_str)
                .ok_or(ReconcileError::InvalidEventPayloadField("sha"))?;
            if !valid_nonzero_commit_sha(sha) {
                return Err(ReconcileError::InvalidEventPayloadField("sha"));
            }
        }
        reject_unexpected_event_payload_fields(
            payload,
            &["issue", "pull_request", "comment", "review"],
        )?;
        return Ok(());
    }

    if event_name == super::PUSH_EVENT {
        let ref_name = payload
            .get("ref")
            .and_then(Value::as_str)
            .ok_or(ReconcileError::InvalidBranch)?;
        let branch = ref_name
            .strip_prefix("refs/heads/")
            .filter(|branch| crate::forgejo::valid_branch_ref_name(branch))
            .ok_or(ReconcileError::UnsupportedEvent)?;
        if branch.is_empty() {
            return Err(ReconcileError::InvalidBranch);
        }
        for field in ["before", "after"] {
            let sha = payload
                .get(field)
                .and_then(Value::as_str)
                .ok_or(ReconcileError::InvalidEnvelope)?;
            if !valid_commit_sha(sha) {
                return Err(ReconcileError::InvalidEnvelope);
            }
        }
        reject_unexpected_event_payload_fields(
            payload,
            &["issue", "pull_request", "comment", "review"],
        )?;
        return Ok(());
    }

    if event_name == super::ISSUES_EVENT {
        reject_unexpected_event_payload_fields(payload, &["pull_request", "comment", "review"])?;
    }

    if event_name == super::PULL_REQUEST_EVENT {
        reject_unexpected_event_payload_fields(payload, &["issue", "comment"])?;
        if payload
            .get("review")
            .is_some_and(|review| !review.is_null())
        {
            return Err(ReconcileError::UnexpectedEventPayloadField("review"));
        }
    }

    if event_name == super::ISSUE_COMMENT_EVENT {
        let issue_number = payload
            .get("issue")
            .and_then(|issue| issue.get("number"))
            .and_then(Value::as_u64)
            .filter(|number| *number > 0)
            .ok_or(ReconcileError::InvalidNumber)?;
        reject_unexpected_event_payload_fields(payload, &["review"])?;
        let comment = payload
            .get("comment")
            .and_then(Value::as_object)
            .ok_or(ReconcileError::InvalidEnvelope)?;
        if !comment
            .get("id")
            .and_then(Value::as_u64)
            .is_some_and(|id| id > 0)
        {
            return Err(ReconcileError::InvalidEventPayloadField("comment.id"));
        }
        let is_pull = payload
            .get("is_pull")
            .and_then(Value::as_bool)
            .ok_or(ReconcileError::InvalidEventPayloadField("is_pull"))?;
        match payload.get("pull_request") {
            None => {
                if is_pull {
                    return Err(ReconcileError::ResourceMismatch);
                }
            }
            Some(Value::Object(pull_request)) => {
                let pull_request_number = pull_request
                    .get("number")
                    .and_then(Value::as_u64)
                    .filter(|number| *number > 0)
                    .ok_or(ReconcileError::InvalidNumber)?;
                if pull_request_number != issue_number || !is_pull {
                    return Err(ReconcileError::ResourceMismatch);
                }
            }
            Some(_) => return Err(ReconcileError::InvalidEnvelope),
        }
        return Ok(());
    }

    let review_type = payload
        .get("review")
        .and_then(|review| review.get("type"))
        .and_then(Value::as_str);
    let expected_review_type = match event_name {
        super::PULL_REQUEST_APPROVED_EVENT => Some(super::PULL_REQUEST_REVIEW_APPROVED_TYPE),
        super::PULL_REQUEST_REJECTED_EVENT => Some(super::PULL_REQUEST_REVIEW_REJECTED_TYPE),
        super::PULL_REQUEST_COMMENT_EVENT => Some(super::PULL_REQUEST_REVIEW_COMMENT_TYPE),
        _ => None,
    };

    if expected_review_type.is_some() {
        let action = payload
            .get("action")
            .and_then(Value::as_str)
            .ok_or(ReconcileError::InvalidEventPayloadField("action"))?;
        if action != "reviewed" {
            return Err(ReconcileError::InvalidEventPayloadField("action"));
        }
    }

    if expected_review_type.is_some_and(|expected| review_type != Some(expected)) {
        return Err(ReconcileError::InvalidEnvelope);
    }
    Ok(())
}

fn reject_unexpected_event_payload_fields(
    payload: &Map<String, Value>,
    fields: &[&'static str],
) -> Result<(), ReconcileError> {
    if let Some(field) = fields.iter().find(|field| payload.contains_key(**field)) {
        return Err(ReconcileError::UnexpectedEventPayloadField(*field));
    }
    Ok(())
}

fn resource_kind_for_event(event_name: &str, payload: &Map<String, Value>) -> Option<ResourceKind> {
    let contract = super::SUPPORTED_EVENT_CONTRACTS
        .iter()
        .find(|contract| contract.event == event_name)?;
    resource_kind_from_contract(contract, payload)
}

fn resource_kind_from_contract(
    contract: &super::WebhookEventContractV1,
    payload: &Map<String, Value>,
) -> Option<ResourceKind> {
    match contract.resource_kind {
        "issue" => Some(ResourceKind::Issue),
        "issue_or_pull_request" => match payload.get("pull_request") {
            None => Some(ResourceKind::IssueComment),
            Some(Value::Object(_)) => Some(ResourceKind::PullRequestComment),
            Some(_) => None,
        },
        "pull_request" => Some(ResourceKind::PullRequest),
        "branch" => Some(ResourceKind::Branch),
        "workflow_run" => Some(ResourceKind::WorkflowRun),
        "workflow_job" => Some(ResourceKind::WorkflowJob),
        "release" => Some(ResourceKind::Release),
        _ => None,
    }
}

#[cfg(test)]
mod resource_kind_contract_tests {
    use super::*;

    #[test]
    fn runtime_resource_mapping_matches_every_published_event_contract() {
        for contract in &super::super::SUPPORTED_EVENT_CONTRACTS {
            let payload = Map::new();
            let actual = resource_kind_for_event(contract.event, &payload)
                .unwrap_or_else(|| panic!("{} has no runtime resource mapping", contract.event));
            if contract.resource_kind == "issue_or_pull_request" {
                assert_eq!(actual, ResourceKind::IssueComment, "{}", contract.event);
                let pull_request_payload = json!({"pull_request": {}});
                let actual = resource_kind_for_event(
                    contract.event,
                    pull_request_payload.as_object().expect("object payload"),
                );
                assert_eq!(
                    actual,
                    Some(ResourceKind::PullRequestComment),
                    "{} must preserve the issue-versus-PR comment distinction",
                    contract.event
                );
                continue;
            }

            let expected = match contract.resource_kind {
                "issue" => ResourceKind::Issue,
                "pull_request" => ResourceKind::PullRequest,
                "branch" => ResourceKind::Branch,
                "workflow_run" => ResourceKind::WorkflowRun,
                "workflow_job" => ResourceKind::WorkflowJob,
                "release" => ResourceKind::Release,
                other => panic!("unrecognized contract resource kind {other:?}"),
            };
            assert_eq!(actual, expected, "{}", contract.event);
        }
    }

    #[test]
    fn unknown_contract_resource_kind_fails_closed() {
        let contract = super::super::WebhookEventContractV1 {
            event: "future_event",
            resource_kind: "future_resource",
            required_payload_fields: &[],
            payload_constraints: &[],
            canonical_read: "",
            not_found_behavior: "",
        };

        assert_eq!(
            resource_kind_from_contract(&contract, &Map::new()),
            None,
            "an unrecognized published resource kind must not default to pull_request"
        );
    }

    #[test]
    fn workflow_action_and_status_contracts_are_published_and_drive_payload_validation() {
        let statuses = &super::super::SUPPORTED_WORKFLOW_STATUS_CONTRACTS;
        let v2 = &super::super::WEBHOOK_EVENT_CONTRACTS_V2;
        let v3 = &super::super::WEBHOOK_EVENT_CONTRACTS_V3;
        let v4 = &super::super::WEBHOOK_EVENT_CONTRACTS_V4;
        assert_eq!(v2.schema_version, 2);
        assert_eq!(v2.workflow_statuses, statuses);
        assert_eq!(v3.schema_version, 3);
        assert_eq!(v3.contracts, v2.contracts);
        assert_eq!(v3.workflow_statuses, statuses);
        assert_eq!(
            v3.workflow_actions,
            &super::super::SUPPORTED_WORKFLOW_ACTION_CONTRACTS
        );
        assert_eq!(v4.schema_version, 4);
        assert_eq!(v4.contracts, v3.contracts);
        assert_eq!(v4.workflow_statuses, statuses);
        assert_eq!(v4.workflow_actions, v3.workflow_actions);
        assert_eq!(
            v4.workflow_event_actions,
            &super::super::SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS
        );
        assert_eq!(statuses.len(), 14);
        assert_eq!(v3.workflow_actions.len(), 2);
        assert_eq!(v4.workflow_event_actions.len(), statuses.len());
        assert_eq!(v3.workflow_actions[0].resource_kind, "workflow_run");
        assert_eq!(v3.workflow_actions[1].resource_kind, "workflow_job");

        for contract in statuses {
            let event_contract = super::super::SUPPORTED_EVENT_CONTRACTS
                .iter()
                .find(|event| event.event == contract.event)
                .unwrap_or_else(|| {
                    panic!(
                        "status contract {} is not an admitted event",
                        contract.event
                    )
                });
            assert_eq!(event_contract.resource_kind, contract.resource_kind);
            let event_action_contract =
                workflow_event_action_contract_for_event(contract.event, contract.resource_kind)
                    .unwrap_or_else(|| {
                        panic!(
                            "status contract {} has no event-action contract",
                            contract.event
                        )
                    });
            assert_eq!(event_action_contract.payload_field, "action");

            let payload = match contract.resource_kind {
                "workflow_run" => {
                    assert_eq!(contract.payload_field, "run.status");
                    let required_status =
                        format!("{}={}", contract.payload_field, contract.expected_status);
                    assert!(
                        event_contract
                            .required_payload_fields
                            .contains(&required_status.as_str())
                    );
                    json!({
                        "action": event_action_contract.allowed_actions[0],
                        "run": {
                            "id": 41,
                            "index_in_repo": 2,
                            "status": contract.expected_status,
                            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            "updated": "2026-10-27T10:00:00Z"
                        }
                    })
                }
                "workflow_job" => {
                    assert_eq!(contract.payload_field, "job.status");
                    assert!(
                        event_contract
                            .required_payload_fields
                            .contains(&contract.payload_field)
                    );
                    json!({
                        "action": event_action_contract.allowed_actions[0],
                        "run": {"id": 41, "index_in_repo": 2},
                        "job": {
                            "id": 59,
                            "run_id": 41,
                            "status": contract.expected_status,
                            "html_url": "https://forgejo.invalid/owner/repository/actions/runs/2/jobs/0/attempt/1"
                        }
                    })
                }
                other => panic!("unexpected workflow status resource kind {other:?}"),
            };
            let payload = payload.as_object().expect("workflow payload object");
            let action_contract = v3
                .workflow_actions
                .iter()
                .find(|action| action.resource_kind == contract.resource_kind)
                .unwrap_or_else(|| {
                    panic!("status contract {} has no action contract", contract.event)
                });
            assert_eq!(action_contract.payload_field, "action");
            assert!(!action_contract.allowed_actions.is_empty());
            let expected_actions: &[&str] = match contract.resource_kind {
                "workflow_run" => &["new_run_attempt", "run_status_changed", "run_completed"],
                "workflow_job" => &["new_job_attempt", "job_status_changed", "job_completed"],
                other => panic!("unexpected workflow resource kind {other:?}"),
            };
            assert_eq!(action_contract.allowed_actions, expected_actions);
            for action in event_action_contract.allowed_actions {
                assert!(action_contract.allowed_actions.contains(action));
                let mut accepted_action = payload.clone();
                accepted_action.insert(
                    action_contract.payload_field.to_owned(),
                    Value::String((*action).to_owned()),
                );
                assert_eq!(
                    validate_event_payload_kind(contract.event, &accepted_action),
                    Ok(()),
                    "published action {action} must be accepted for {}",
                    contract.event
                );
            }
            let mut unknown_action = payload.clone();
            unknown_action.insert(
                action_contract.payload_field.to_owned(),
                Value::String("workflow_action_future_unknown".to_owned()),
            );
            assert_eq!(
                validate_event_payload_kind(contract.event, &unknown_action),
                Err(ReconcileError::InvalidEventPayloadField("action")),
                "an action absent from the published contract must fail closed for {}",
                contract.event
            );
            assert_eq!(
                validate_event_payload_kind(contract.event, payload),
                Ok(()),
                "the advertised expected status must be accepted for {}",
                contract.event
            );
            let terminal_status = matches!(
                contract.expected_status,
                "success" | "failure" | "cancelled" | "skipped"
            );
            let incompatible_action = match (contract.resource_kind, terminal_status) {
                ("workflow_run", true) => "run_status_changed",
                ("workflow_run", false) => "run_completed",
                ("workflow_job", true) => "job_status_changed",
                ("workflow_job", false) => "job_completed",
                other => panic!("unexpected workflow action/status pair {other:?}"),
            };
            let mut inconsistent_pair = payload.clone();
            inconsistent_pair.insert(
                action_contract.payload_field.to_owned(),
                Value::String(incompatible_action.to_owned()),
            );
            assert_eq!(
                validate_event_payload_kind(contract.event, &inconsistent_pair),
                Err(ReconcileError::InvalidEventPayloadField("action")),
                "a completion action must match Forgejo's terminal-status path for {}",
                contract.event
            );
            let mut wrong_status = payload.clone();
            let member = if contract.resource_kind == "workflow_run" {
                &mut wrong_status["run"]["status"]
            } else {
                &mut wrong_status["job"]["status"]
            };
            *member = Value::String("not-a-forgejo-status".to_owned());
            assert_eq!(
                validate_event_payload_kind(contract.event, &wrong_status),
                Err(ReconcileError::InvalidEventPayloadField(
                    contract.payload_field,
                )),
                "a status that differs from the published contract must be refused for {}",
                contract.event
            );
        }

        for event in &super::super::SUPPORTED_EVENT_CONTRACTS {
            let status_count = statuses
                .iter()
                .filter(|status| status.event == event.event)
                .count();
            let is_workflow_status = matches!(event.resource_kind, "workflow_run" | "workflow_job");
            assert_eq!(
                status_count,
                usize::from(is_workflow_status),
                "{}",
                event.event
            );
        }
    }
}

fn parse_resource_ref(
    event_name: &str,
    kind: ResourceKind,
    payload: &Map<String, Value>,
) -> Result<ResourceRef, ReconcileError> {
    let repository_payload =
        (if matches!(kind, ResourceKind::WorkflowRun | ResourceKind::WorkflowJob) {
            payload.get("run").and_then(|run| run.get("repository"))
        } else {
            payload.get("repository")
        })
        .and_then(Value::as_object)
        .ok_or(ReconcileError::InvalidRepository)?;
    let full_name = repository_payload
        .get("full_name")
        .and_then(Value::as_str)
        .ok_or(ReconcileError::InvalidRepository)?;
    let (owner, repository) = full_name
        .split_once('/')
        .filter(|(owner, repository)| {
            !repository.contains('/')
                && valid_repo_component(owner)
                && valid_repo_component(repository)
        })
        .ok_or(ReconcileError::InvalidRepository)?;
    validate_repository_identity(repository_payload, owner, repository)?;
    if matches!(
        kind,
        ResourceKind::PullRequest | ResourceKind::PullRequestComment
    ) {
        validate_pull_request_base_repository(payload, owner, repository, full_name).map_err(
            |error| match error {
                ReconcileError::InvalidRepository | ReconcileError::ResourceMismatch => {
                    ReconcileError::InvalidEventPayloadField("pull_request.base.repo")
                }
                ReconcileError::InvalidEnvelope => {
                    ReconcileError::InvalidEventPayloadField("pull_request.base")
                }
                error => error,
            },
        )?;
    }
    if matches!(kind, ResourceKind::Issue | ResourceKind::PullRequest) {
        let resource_name = if kind == ResourceKind::Issue {
            "issue"
        } else {
            "pull_request"
        };
        validate_optional_root_resource_number(payload, resource_name)?;
    }

    let id = match kind {
        ResourceKind::IssueComment | ResourceKind::PullRequestComment => {
            let issue_number = payload
                .get("issue")
                .and_then(|issue| issue.get("number"))
                .and_then(Value::as_u64)
                .filter(|number| *number > 0)
                .ok_or(ReconcileError::InvalidNumber)?;
            let comment_id = payload
                .get("comment")
                .and_then(|comment| comment.get("id"))
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or(ReconcileError::InvalidEventPayloadField("comment.id"))?;
            let deleted_event = payload.get("action").and_then(Value::as_str) == Some("deleted");
            ResourceId::IssueComment {
                issue_number,
                comment_id,
                deleted_event,
            }
        }
        ResourceKind::Branch => {
            let ref_name = payload
                .get("ref")
                .and_then(Value::as_str)
                .ok_or(ReconcileError::InvalidBranch)?;
            let branch = if event_name == super::CREATE_EVENT || event_name == super::DELETE_EVENT {
                ref_name
            } else {
                ref_name
                    .strip_prefix("refs/heads/")
                    .filter(|branch| crate::forgejo::valid_branch_ref_name(branch))
                    .ok_or(ReconcileError::UnsupportedEvent)?
            };
            if !crate::forgejo::valid_branch_ref_name(branch) {
                return Err(ReconcileError::UnsupportedEvent);
            }
            ResourceId::Branch(branch.to_owned())
        }
        ResourceKind::WorkflowRun => {
            let run = payload
                .get("run")
                .and_then(Value::as_object)
                .ok_or(ReconcileError::InvalidEventPayloadField("run"))?;
            ResourceId::WorkflowRun {
                run_id: run
                    .get("id")
                    .and_then(Value::as_u64)
                    .filter(|id| *id > 0)
                    .ok_or(ReconcileError::InvalidEventPayloadField("run.id"))?,
                index_in_repo: run
                    .get("index_in_repo")
                    .and_then(Value::as_u64)
                    .filter(|index| *index > 0)
                    .ok_or(ReconcileError::InvalidEventPayloadField(
                        "run.index_in_repo",
                    ))?,
            }
        }
        ResourceKind::WorkflowJob => {
            let run = payload
                .get("run")
                .and_then(Value::as_object)
                .ok_or(ReconcileError::InvalidEventPayloadField("run"))?;
            let job = payload
                .get("job")
                .and_then(Value::as_object)
                .ok_or(ReconcileError::InvalidEventPayloadField("job"))?;
            let run_id = run
                .get("id")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or(ReconcileError::InvalidEventPayloadField("run.id"))?;
            let index_in_repo = run
                .get("index_in_repo")
                .and_then(Value::as_u64)
                .filter(|index| *index > 0)
                .ok_or(ReconcileError::InvalidEventPayloadField(
                    "run.index_in_repo",
                ))?;
            let job_index = job
                .get("html_url")
                .and_then(Value::as_str)
                .and_then(|url| workflow_job_index_from_event_url(url, index_in_repo))
                .ok_or(ReconcileError::InvalidEventPayloadField("job.html_url"))?;
            let job_run_id = job
                .get("run_id")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or(ReconcileError::InvalidEventPayloadField("job.run_id"))?;
            if job_run_id != run_id {
                return Err(ReconcileError::ResourceMismatch);
            }
            ResourceId::WorkflowJob {
                job_id: job
                    .get("id")
                    .and_then(Value::as_u64)
                    .filter(|id| *id > 0)
                    .ok_or(ReconcileError::InvalidEventPayloadField("job.id"))?,
                run_id,
                index_in_repo,
                job_index,
            }
        }
        ResourceKind::Release => {
            let release = payload
                .get("release")
                .and_then(Value::as_object)
                .ok_or(ReconcileError::InvalidEventPayloadField("release"))?;
            ResourceId::Release(
                release
                    .get("id")
                    .and_then(Value::as_u64)
                    .filter(|id| *id > 0)
                    .ok_or(ReconcileError::InvalidEventPayloadField("release.id"))?,
            )
        }
        ResourceKind::Issue | ResourceKind::PullRequest => {
            let resource_name = match kind {
                ResourceKind::Issue => "issue",
                ResourceKind::PullRequest => "pull_request",
                ResourceKind::IssueComment
                | ResourceKind::PullRequestComment
                | ResourceKind::Branch
                | ResourceKind::WorkflowRun
                | ResourceKind::WorkflowJob
                | ResourceKind::Release => unreachable!(),
            };
            let number = payload
                .get(resource_name)
                .and_then(|resource| resource.get("number"))
                .and_then(Value::as_u64)
                .filter(|number| *number > 0)
                .ok_or(ReconcileError::InvalidNumber)?;
            ResourceId::Number(number)
        }
    };
    Ok(ResourceRef {
        kind,
        owner: owner.to_owned(),
        repository: repository.to_owned(),
        id,
    })
}

fn validate_optional_root_resource_number(
    payload: &Map<String, Value>,
    resource_name: &str,
) -> Result<(), ReconcileError> {
    let Some(root_number) = payload.get("number") else {
        return Ok(());
    };
    let root_number = root_number
        .as_u64()
        .filter(|number| *number > 0)
        .ok_or(ReconcileError::InvalidEventPayloadField("number"))?;
    let resource_number = payload
        .get(resource_name)
        .and_then(|resource| resource.get("number"))
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .ok_or(ReconcileError::InvalidNumber)?;
    if root_number != resource_number {
        return Err(ReconcileError::ResourceMismatch);
    }
    Ok(())
}

fn workflow_status_contract_for_event(
    event_name: &str,
    resource_kind: &str,
) -> Option<&'static super::WebhookWorkflowStatusContractV1> {
    super::SUPPORTED_WORKFLOW_STATUS_CONTRACTS
        .iter()
        .find(|contract| contract.event == event_name && contract.resource_kind == resource_kind)
}

fn workflow_action_contract_for_resource_kind(
    resource_kind: &str,
) -> Option<&'static super::WebhookWorkflowActionContractV1> {
    super::SUPPORTED_WORKFLOW_ACTION_CONTRACTS
        .iter()
        .find(|contract| contract.resource_kind == resource_kind)
}

fn workflow_event_action_contract_for_event(
    event_name: &str,
    resource_kind: &str,
) -> Option<&'static super::WebhookWorkflowEventActionContractV1> {
    super::SUPPORTED_WORKFLOW_EVENT_ACTION_CONTRACTS
        .iter()
        .find(|contract| contract.event == event_name && contract.resource_kind == resource_kind)
}

fn event_action_contract_for_event(
    event_name: &str,
) -> Option<&'static super::WebhookEventActionContractV1> {
    super::SUPPORTED_EVENT_ACTION_CONTRACTS
        .iter()
        .find(|contract| contract.event == event_name)
}

fn payload_string_at_path<'a>(payload: &'a Map<String, Value>, path: &str) -> Option<&'a str> {
    let mut parts = path.split('.');
    let mut value = payload.get(parts.next()?)?;
    for part in parts {
        value = value.get(part)?;
    }
    value.as_str()
}

fn valid_workflow_run_status(status: &str) -> bool {
    super::SUPPORTED_WORKFLOW_STATUS_CONTRACTS
        .iter()
        .any(|contract| {
            contract.resource_kind == "workflow_run" && contract.expected_status == status
        })
}

fn validate_current_workflow_run_repository(
    current: &Value,
    resource: &ResourceRef,
) -> Result<(), ReconcileError> {
    let repository = current
        .get("repository")
        .and_then(Value::as_object)
        .ok_or(ReconcileError::InvalidCurrentStateField("repository"))?;
    let full_name = repository.get("full_name").and_then(Value::as_str).ok_or(
        ReconcileError::InvalidCurrentStateField("repository.full_name"),
    )?;
    if full_name != format!("{}/{}", resource.owner, resource.repository) {
        return Err(ReconcileError::ResourceMismatch);
    }
    validate_repository_identity(repository, &resource.owner, &resource.repository)
        .map_err(|_| ReconcileError::ResourceMismatch)
}

fn validate_repository_identity(
    repository: &Map<String, Value>,
    expected_owner: &str,
    expected_name: &str,
) -> Result<(), ReconcileError> {
    if let Some(name) = repository.get("name") {
        let name = name.as_str().ok_or(ReconcileError::InvalidRepository)?;
        if name != expected_name {
            return Err(ReconcileError::ResourceMismatch);
        }
    }

    match repository.get("owner") {
        None | Some(Value::Null) => {}
        Some(Value::Object(owner)) => {
            if let Some(login) = owner.get("login") {
                let login = login.as_str().ok_or(ReconcileError::InvalidRepository)?;
                if login != expected_owner {
                    return Err(ReconcileError::ResourceMismatch);
                }
            }
        }
        Some(_) => return Err(ReconcileError::InvalidRepository),
    }

    Ok(())
}

fn validate_pull_request_base_repository(
    payload: &Map<String, Value>,
    expected_owner: &str,
    expected_repository: &str,
    expected_full_name: &str,
) -> Result<(), ReconcileError> {
    let Some(pull_request) = payload.get("pull_request").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(base_value) = pull_request.get("base") else {
        return Ok(());
    };
    let base = base_value
        .as_object()
        .ok_or(ReconcileError::InvalidEnvelope)?;
    let Some(repository_value) = base.get("repo") else {
        return Ok(());
    };
    let repository = repository_value
        .as_object()
        .ok_or(ReconcileError::InvalidRepository)?;

    if let Some(full_name) = repository.get("full_name") {
        let full_name = full_name
            .as_str()
            .ok_or(ReconcileError::InvalidRepository)?;
        if full_name != expected_full_name {
            return Err(ReconcileError::ResourceMismatch);
        }
    }
    validate_repository_identity(repository, expected_owner, expected_repository)
}

fn validate_current_pull_request_base_repository(
    current: &Value,
    resource: &ResourceRef,
) -> Result<(), ReconcileError> {
    let Some(repository_value) = current
        .get("base")
        .and_then(Value::as_object)
        .and_then(|base| base.get("repo"))
    else {
        return Ok(());
    };
    if repository_value.is_null() {
        return Ok(());
    }
    let repository = repository_value
        .as_object()
        .ok_or(ReconcileError::InvalidCurrentStateField("base.repo"))?;
    let has_identity_field = repository.contains_key("full_name")
        || repository.contains_key("name")
        || repository
            .get("owner")
            .and_then(Value::as_object)
            .is_some_and(|owner| owner.contains_key("login"));
    if !has_identity_field {
        return Err(ReconcileError::InvalidCurrentStateField("base.repo"));
    }

    if let Some(full_name) = repository.get("full_name") {
        let full_name = full_name
            .as_str()
            .ok_or(ReconcileError::InvalidCurrentStateField(
                "base.repo.full_name",
            ))?;
        if full_name != format!("{}/{}", resource.owner, resource.repository) {
            return Err(ReconcileError::ResourceMismatch);
        }
    }
    match validate_repository_identity(repository, &resource.owner, &resource.repository) {
        Err(ReconcileError::InvalidRepository) => {
            Err(ReconcileError::InvalidCurrentStateField("base.repo"))
        }
        result => result,
    }
}

/// Reconcile up to `MAX_RECONCILE_BATCH_EVENTS` notifications while reading
/// each unique resource at most once. Replayed deliveries can have new delivery
/// IDs, so deduplicating only by delivery ID is insufficient for API efficiency.
pub fn reconcile_inbox_events(
    events: &[JournalEvent],
    source: &impl ForgejoCurrentState,
) -> Result<Vec<Result<ReconciledState, ReconcileError>>, ReconcileError> {
    if events.len() > MAX_RECONCILE_BATCH_EVENTS {
        return Err(ReconcileError::BatchTooLarge);
    }

    let cached = CachedCurrentState {
        source,
        responses: RefCell::new(HashMap::new()),
        cached_json_bytes: Cell::new(0),
        cache_budget_exhausted: Cell::new(false),
    };
    let mut results = Vec::with_capacity(events.len());
    let mut projected_output_bytes = 0usize;
    for event in events {
        let result = reconcile_inbox_event(event, &cached);
        let state_bytes = match &result {
            Ok(state) => serde_json::to_vec(&reconciled_state_value(state))
                .expect("JSON state projection is serializable")
                .len(),
            Err(_) => 0,
        };
        projected_output_bytes = projected_output_bytes
            .saturating_add(state_bytes)
            .saturating_add(RECONCILE_BATCH_RESULT_ENVELOPE_RESERVE_BYTES);
        if projected_output_bytes > MAX_RECONCILE_BATCH_OUTPUT_BYTES {
            return Err(ReconcileError::BatchOutputBudgetExceeded {
                maximum_output_bytes: MAX_RECONCILE_BATCH_OUTPUT_BYTES,
            });
        }
        results.push(result);
    }
    Ok(results)
}

struct CachedCurrentState<'a, S> {
    source: &'a S,
    responses: RefCell<HashMap<ResourceRef, Result<Value, ApiReadError>>>,
    cached_json_bytes: Cell<usize>,
    cache_budget_exhausted: Cell<bool>,
}

fn cache_resource_identity(resource: &ResourceRef) -> ResourceRef {
    let mut identity = resource.clone();
    if let ResourceId::IssueComment { deleted_event, .. } = &mut identity.id {
        // Deletion changes how a 404 is interpreted by the event consumer,
        // not which Forgejo comment endpoint is read.
        *deleted_event = false;
    }
    identity
}

impl<S: ForgejoCurrentState> ForgejoCurrentState for CachedCurrentState<'_, S> {
    fn read_current(&self, resource: &ResourceRef) -> Result<Value, ApiReadError> {
        let identity = cache_resource_identity(resource);
        if let Some(cached) = self.responses.borrow().get(&identity).cloned() {
            return cached;
        }

        let response = if self.cache_budget_exhausted.get() {
            Err(ApiReadError::BatchCacheBudgetExceeded {
                maximum_cached_json_bytes: MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
            })
        } else {
            self.source.read_current(&identity).and_then(|value| {
                let serialized_json_bytes = serde_json::to_vec(&value)
                    .map_err(|_| ApiReadError::InvalidResponse)?
                    .len();
                let combined_cached_json_bytes = self
                    .cached_json_bytes
                    .get()
                    .saturating_add(serialized_json_bytes);
                if combined_cached_json_bytes > MAX_RECONCILE_BATCH_CACHED_JSON_BYTES {
                    self.cache_budget_exhausted.set(true);
                    Err(ApiReadError::BatchCacheBudgetExceeded {
                        maximum_cached_json_bytes: MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
                    })
                } else {
                    self.cached_json_bytes.set(combined_cached_json_bytes);
                    Ok(value)
                }
            })
        };
        self.responses
            .borrow_mut()
            .insert(identity, response.clone());
        response
    }

    fn read_current_fresh(&self, resource: &ResourceRef) -> Result<Value, ApiReadError> {
        let identity = cache_resource_identity(resource);
        let previous = self.responses.borrow_mut().remove(&identity);
        let previous_bytes = previous
            .and_then(Result::ok)
            .and_then(|value| serde_json::to_vec(&value).ok())
            .map_or(0, |bytes| bytes.len());
        self.cached_json_bytes
            .set(self.cached_json_bytes.get().saturating_sub(previous_bytes));

        let response = if self.cache_budget_exhausted.get() {
            Err(ApiReadError::BatchCacheBudgetExceeded {
                maximum_cached_json_bytes: MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
            })
        } else {
            self.source.read_current_fresh(&identity).and_then(|value| {
                let serialized_json_bytes = serde_json::to_vec(&value)
                    .map_err(|_| ApiReadError::InvalidResponse)?
                    .len();
                let combined_cached_json_bytes = self
                    .cached_json_bytes
                    .get()
                    .saturating_add(serialized_json_bytes);
                if combined_cached_json_bytes > MAX_RECONCILE_BATCH_CACHED_JSON_BYTES {
                    self.cache_budget_exhausted.set(true);
                    Err(ApiReadError::BatchCacheBudgetExceeded {
                        maximum_cached_json_bytes: MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
                    })
                } else {
                    self.cached_json_bytes.set(combined_cached_json_bytes);
                    Ok(value)
                }
            })
        };
        self.responses
            .borrow_mut()
            .insert(identity, response.clone());
        response
    }

    fn expected_html_url(&self, resource: &ResourceRef) -> String {
        self.source.expected_html_url(resource)
    }
}

/// When Forgejo includes issue repository metadata, bind it to the requested
/// repository as well as checking the endpoint number and canonical HTML URL.
/// Missing or null metadata remains compatible with partial API responses.
fn validate_current_issue_repository(
    current: &Value,
    resource: &ResourceRef,
) -> Result<(), ReconcileError> {
    let Some(repository_value) = current.get("repository") else {
        return Ok(());
    };
    if repository_value.is_null() {
        return Ok(());
    }
    let repository = repository_value
        .as_object()
        .ok_or(ReconcileError::InvalidCurrentStateField("repository"))?;
    let mut has_identity_field = false;

    if let Some(full_name) = repository.get("full_name") {
        has_identity_field = true;
        let full_name = full_name
            .as_str()
            .ok_or(ReconcileError::InvalidCurrentStateField(
                "repository.full_name",
            ))?;
        if full_name != format!("{}/{}", resource.owner, resource.repository) {
            return Err(ReconcileError::ResourceMismatch);
        }
    }
    if let Some(name) = repository.get("name") {
        has_identity_field = true;
        let name = name
            .as_str()
            .ok_or(ReconcileError::InvalidCurrentStateField("repository.name"))?;
        if name != resource.repository {
            return Err(ReconcileError::ResourceMismatch);
        }
    }
    if let Some(owner) = repository.get("owner") {
        has_identity_field = true;
        let owner = owner
            .as_str()
            .ok_or(ReconcileError::InvalidCurrentStateField("repository.owner"))?;
        if owner != resource.owner {
            return Err(ReconcileError::ResourceMismatch);
        }
    }
    if !has_identity_field {
        return Err(ReconcileError::InvalidCurrentStateField("repository"));
    }

    Ok(())
}

fn valid_delivery_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_repo_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_commit_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_nonzero_commit_sha(value: &str) -> bool {
    valid_commit_sha(value) && value.bytes().any(|byte| byte != b'0')
}

pub fn valid_forgejo_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }

    let Some(year) = decimal(&bytes[0..4]) else {
        return false;
    };
    let Some(month) = decimal(&bytes[5..7]) else {
        return false;
    };
    let Some(day) = decimal(&bytes[8..10]) else {
        return false;
    };
    let Some(hour) = decimal(&bytes[11..13]) else {
        return false;
    };
    let Some(minute) = decimal(&bytes[14..16]) else {
        return false;
    };
    let Some(second) = decimal(&bytes[17..19]) else {
        return false;
    };
    // Go's time.Time cannot represent leap seconds, so reject every :60 value.
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }

    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > last_day {
        return false;
    }

    let mut index = 19;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }

    match bytes.get(index) {
        Some(b'Z' | b'z') => index + 1 == bytes.len(),
        Some(b'+' | b'-') => {
            if index + 6 != bytes.len() || bytes[index + 3] != b':' {
                return false;
            }
            matches!(decimal(&bytes[index + 1..index + 3]), Some(0..=23))
                && matches!(decimal(&bytes[index + 4..index + 6]), Some(0..=59))
        }
        _ => false,
    }
}

fn decimal(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    bytes.iter().try_fold(0_u32, |value, byte| {
        value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))
    })
}
