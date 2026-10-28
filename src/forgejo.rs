//! Small local coordination primitives for Forgejo adapters.
//!
//! The merge lock is cooperative and scoped to one Forgejo base branch. A
//! future merge gate must hold it across its final base/head reread, proof
//! validation, merge request, and post-merge readback. It does not prevent
//! direct Forgejo API writes, coordinate separate hosts, or replace Forgejo's
//! own authorization and branch protections.

use crate::{
    ExpectedBinding, GateError, ProofTargetV2, SignedProof, TargetBoundProofV2, verify_proof,
    verify_target_bound_proof_v2,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// Shared UTF-8 byte limit for a Forgejo label name at API and agent-context boundaries.
pub const MAX_FORGEJO_LABEL_NAME_BYTES: usize = 255;

pub const MAX_MERGE_APPROVAL_POLICY_BYTES: usize = 64 * 1024;
pub const MAX_MERGE_APPROVAL_REVIEWERS: usize = 1024;
pub const MAX_MERGE_APPROVAL_REVIEW_ENTRIES: usize = 10_000;
pub const MAX_MERGE_APPROVAL_REVIEW_STATE_BYTES: usize = 64;
pub const MAX_MERGE_APPROVAL_REVIEW_COMMIT_ID_BYTES: usize = 64;
pub const MAX_MERGE_APPROVAL_REVIEW_PULL_REQUEST_URL_BYTES: usize = 4096;
pub const MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_MERGE_APPROVAL_REVIEW_TOTAL_BODY_BYTES: usize = 16 * 1024 * 1024;

/// A Forgejo repository's base branch, used as the serialization boundary for
/// merge operations. Credentials are not accepted in the origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseBranchScope {
    origin: String,
    owner: String,
    repository: String,
    branch: String,
}

impl BaseBranchScope {
    pub fn new(
        origin: &str,
        owner: &str,
        repository: &str,
        branch: &str,
    ) -> Result<Self, MergeLockError> {
        let origin = normalize_origin(origin).ok_or(MergeLockError::InvalidScope)?;
        if !valid_segment(owner) || !valid_segment(repository) || !valid_branch(branch) {
            return Err(MergeLockError::InvalidScope);
        }
        Ok(Self {
            origin,
            owner: owner.to_owned(),
            repository: repository.to_owned(),
            branch: branch.to_owned(),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    fn lock_key(&self) -> String {
        let mut digest = Sha256::new();
        for part in [
            self.origin.as_bytes(),
            self.owner.as_bytes(),
            self.repository.as_bytes(),
            self.branch.as_bytes(),
        ] {
            digest.update((part.len() as u64).to_be_bytes());
            digest.update(part);
        }
        hex::encode(digest.finalize())
    }
}

/// Current fields read by a trusted Forgejo adapter for one pull request.
/// This is a snapshot only; it cannot make a later server-side merge atomic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestSnapshotV1 {
    pub origin: String,
    pub owner: String,
    pub repository: String,
    pub number: u64,
    pub state: String,
    pub merged: bool,
    pub draft: bool,
    pub mergeable: Option<bool>,
    pub base_branch: String,
    pub base_sha: String,
    pub head_sha: String,
}

impl PullRequestSnapshotV1 {
    /// Parse only the fields needed from a Forgejo PR response and bind them to
    /// the configured origin, repository, number, base URL, and both PR refs.
    pub fn from_forgejo_response(
        origin: &str,
        owner: &str,
        repository: &str,
        expected_number: u64,
        response: &serde_json::Value,
    ) -> Result<Self, MergePreflightError> {
        let origin = normalize_origin(origin).ok_or(MergePreflightError::WrongScope)?;
        if !valid_segment(owner) || !valid_segment(repository) {
            return Err(MergePreflightError::WrongScope);
        }
        if expected_number == 0
            || response.get("number").and_then(serde_json::Value::as_u64) != Some(expected_number)
        {
            return Err(MergePreflightError::WrongPullRequest);
        }
        let expected_repo = format!("{owner}/{repository}");
        let expected_url = format!("{origin}/{owner}/{repository}/pulls/{expected_number}");
        if response.get("html_url").and_then(serde_json::Value::as_str)
            != Some(expected_url.as_str())
        {
            return Err(MergePreflightError::WrongScope);
        }

        let base = response
            .get("base")
            .and_then(serde_json::Value::as_object)
            .ok_or(MergePreflightError::InvalidSnapshot)?;
        let head = response
            .get("head")
            .and_then(serde_json::Value::as_object)
            .ok_or(MergePreflightError::InvalidSnapshot)?;
        let base_repo = base
            .get("repo")
            .and_then(|value| value.get("full_name"))
            .and_then(serde_json::Value::as_str);
        if base_repo != Some(expected_repo.as_str()) {
            return Err(MergePreflightError::WrongScope);
        }

        let state = response
            .get("state")
            .and_then(serde_json::Value::as_str)
            .ok_or(MergePreflightError::InvalidSnapshot)?;
        let merged = response
            .get("merged")
            .and_then(serde_json::Value::as_bool)
            .ok_or(MergePreflightError::InvalidSnapshot)?;
        let draft = response
            .get("draft")
            .and_then(serde_json::Value::as_bool)
            .ok_or(MergePreflightError::InvalidSnapshot)?;
        let mergeable = match response.get("mergeable") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_bool()
                    .ok_or(MergePreflightError::InvalidSnapshot)?,
            ),
        };
        let base_branch = base
            .get("ref")
            .and_then(serde_json::Value::as_str)
            .ok_or(MergePreflightError::InvalidSnapshot)?;
        let base_sha = base
            .get("sha")
            .and_then(serde_json::Value::as_str)
            .ok_or(MergePreflightError::InvalidSnapshot)?;
        let head_sha = head
            .get("sha")
            .and_then(serde_json::Value::as_str)
            .ok_or(MergePreflightError::InvalidSnapshot)?;

        Ok(Self {
            origin,
            owner: owner.to_owned(),
            repository: repository.to_owned(),
            number: expected_number,
            state: state.to_owned(),
            merged,
            draft,
            mergeable,
            base_branch: base_branch.to_owned(),
            base_sha: base_sha.to_owned(),
            head_sha: head_sha.to_owned(),
        })
    }
}

/// Minimal Forgejo review API fields used to evaluate a current approval.
/// Unknown response fields are ignored for forward compatibility; fields
/// whose absence could authorize a review are required during deserialization.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PullRequestReviewV1 {
    pub id: u64,
    #[serde(default)]
    pub user: Option<ReviewUserV1>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub commit_id: String,
    pub pull_request_url: String,
    pub stale: bool,
    #[serde(default)]
    pub official: bool,
    pub dismissed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReviewUserV1 {
    pub id: u64,
}

fn review_fields_within_limits(
    review: &PullRequestReviewV1,
    expected_pull_request_url: &str,
) -> bool {
    review.state.len() <= MAX_MERGE_APPROVAL_REVIEW_STATE_BYTES
        && review.commit_id.len() <= MAX_MERGE_APPROVAL_REVIEW_COMMIT_ID_BYTES
        && review.pull_request_url.len() <= MAX_MERGE_APPROVAL_REVIEW_PULL_REQUEST_URL_BYTES
        && review.pull_request_url == expected_pull_request_url
}

/// Trusted request scope attached by the host adapter to a Forgejo API read.
/// It records the canonical origin and endpoint path used to fetch the PR.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PullRequestReviewScopeV1 {
    origin: String,
    owner: String,
    repository: String,
    pull_request_number: u64,
}

impl PullRequestReviewScopeV1 {
    pub fn new(
        origin: &str,
        owner: &str,
        repository: &str,
        pull_request_number: u64,
    ) -> Result<Self, MergeApprovalError> {
        let origin = normalize_origin(origin).ok_or(MergeApprovalError::InvalidBinding)?;
        if !valid_segment(owner) || !valid_segment(repository) || pull_request_number == 0 {
            return Err(MergeApprovalError::InvalidBinding);
        }
        Ok(Self {
            origin,
            owner: owner.to_owned(),
            repository: repository.to_owned(),
            pull_request_number,
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn pull_request_number(&self) -> u64 {
        self.pull_request_number
    }

    pub fn pull_request_url(&self) -> String {
        format!(
            "{}/{}/{}/pulls/{}",
            self.origin,
            encode_url_path_segment(&self.owner),
            encode_url_path_segment(&self.repository),
            self.pull_request_number
        )
    }
}

/// One page returned by Forgejo's `GET .../pulls/{index}/reviews` endpoint.
/// `scope` is set by the trusted adapter from the configured origin and
/// requested path; `total_count` comes from `X-Total-Count` and `page` is the
/// requested page number. The list is not an atomic Forgejo snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestReviewPageV1 {
    pub scope: PullRequestReviewScopeV1,
    pub page: u32,
    pub total_count: u64,
    pub reviews: Vec<PullRequestReviewV1>,
}

/// An untrusted JSON response body and its request metadata from the adapter.
/// `scope` and `page` must be bound to the request; `total_count` must come
/// from the response header. The body is read only through the byte budgets
/// before it is parsed into review DTOs.
pub struct PullRequestReviewPageBodyV1<R> {
    pub scope: PullRequestReviewScopeV1,
    pub page: u32,
    pub total_count: u64,
    pub body: R,
}

/// Reviews read from every page of one Forgejo pull request.
///
/// The adapter must authenticate the response and set scope and `total_count`
/// from the trusted request and Forgejo response. This DTO cannot establish
/// API provenance or atomic consistency across pages by itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestReviewsSnapshotV1 {
    pub scope: PullRequestReviewScopeV1,
    pub total_count: u64,
    pub reviews: Vec<PullRequestReviewV1>,
}

/// Request-bound Forgejo PR fields needed to authenticate the provenance of a
/// protected requirements revision. The source adapter must read this from
/// the configured repository and re-read it after paginating reviews.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequirementSourcePullRequestV1 {
    pub scope: PullRequestReviewScopeV1,
    pub merged: bool,
    pub base_branch: String,
    pub head_sha: String,
    pub merge_commit_sha: String,
}

/// Host-recorded provenance for one accepted requirements arbitration. This
/// records what the host evaluated; it is not a signature or proof that the
/// listed Forgejo accounts are natural persons.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequirementArbitrationDecisionV1 {
    pub schema_version: u32,
    pub source_pull_request_number: u64,
    pub source_merge_commit_sha: String,
    pub source_head_sha: String,
    pub requirements_sha256: String,
    pub policy_revision: u64,
    pub policy_sha256: String,
    pub review_snapshot_sha256: String,
    pub approved_review_ids: Vec<u64>,
    pub reviewer_ids: Vec<u64>,
}

/// Host-attested binding between one accepted Forgejo approval and the exact
/// base branch/ref observed when that approval was recorded. Forgejo's review
/// API does not expose this binding; a production adapter must obtain it from
/// an authoritative source and must fail closed when it is absent.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PullRequestReviewBaseBindingV1 {
    pub review_id: u64,
    pub base_branch: String,
    pub base_commit: String,
}

/// Base-ref attestations for the accepted reviews in one pull request.
/// There must be exactly one entry for each review selected by the independent
/// approval policy. This object is trustworthy only when supplied by a trusted
/// host adapter; the type itself does not authenticate its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestReviewBaseBindingsSnapshotV1 {
    pub scope: PullRequestReviewScopeV1,
    pub bindings: Vec<PullRequestReviewBaseBindingV1>,
}

/// Read and deserialize paged review responses under per-page and aggregate
/// byte limits, then apply the normal completeness, scope and review checks.
/// The adapter should pass a streaming response body when available; timeouts,
/// cancellation and authentication remain responsibilities of that transport.
pub fn collect_pull_request_review_json_pages<R: Read>(
    expected_scope: &PullRequestReviewScopeV1,
    page_bodies: impl IntoIterator<Item = PullRequestReviewPageBodyV1<R>>,
) -> Result<PullRequestReviewsSnapshotV1, MergeApprovalError> {
    let mut pages = Vec::new();
    let mut total_body_bytes = 0usize;
    let mut expected_total_count = None;

    for (index, response) in page_bodies.into_iter().enumerate() {
        let page_number = index
            .checked_add(1)
            .and_then(|number| u32::try_from(number).ok())
            .ok_or(MergeApprovalError::TooManyReviews)?;
        if pages.len() > MAX_MERGE_APPROVAL_REVIEW_ENTRIES {
            return Err(MergeApprovalError::TooManyReviews);
        }
        if response.scope != *expected_scope {
            return Err(MergeApprovalError::WrongScope);
        }
        if response.page != page_number {
            return Err(MergeApprovalError::IncompleteReviewList);
        }
        match expected_total_count {
            Some(total_count) if total_count != response.total_count => {
                return Err(MergeApprovalError::IncompleteReviewList);
            }
            None => {
                if response.total_count > MAX_MERGE_APPROVAL_REVIEW_ENTRIES as u64 {
                    return Err(MergeApprovalError::TooManyReviews);
                }
                expected_total_count = Some(response.total_count);
            }
            _ => {}
        }
        if u64::from(page_number) > response.total_count.saturating_add(1) {
            return Err(MergeApprovalError::IncompleteReviewList);
        }

        let total_body_bytes_remaining = MAX_MERGE_APPROVAL_REVIEW_TOTAL_BODY_BYTES
            .checked_sub(total_body_bytes)
            .ok_or(MergeApprovalError::ReviewSnapshotBodyTooLarge)?;
        let read_limit = MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES.min(total_body_bytes_remaining);
        let read_limit_plus_one = read_limit
            .checked_add(1)
            .ok_or(MergeApprovalError::ReviewSnapshotBodyTooLarge)?;
        let mut body = Vec::with_capacity(read_limit.min(16 * 1024));
        response
            .body
            .take(read_limit_plus_one as u64)
            .read_to_end(&mut body)
            .map_err(|_| MergeApprovalError::ReviewResponseReadFailed)?;
        if body.len() > MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES {
            return Err(MergeApprovalError::ReviewPageBodyTooLarge);
        }
        if body.len() > total_body_bytes_remaining {
            return Err(MergeApprovalError::ReviewSnapshotBodyTooLarge);
        }
        total_body_bytes += body.len();

        let value = crate::strict_json::parse_value_rejecting_duplicate_members(&body)
            .map_err(|_| MergeApprovalError::InvalidReviewResponse)?;
        let reviews =
            serde_json::from_value(value).map_err(|_| MergeApprovalError::InvalidReviewResponse)?;
        pages.push(PullRequestReviewPageV1 {
            scope: response.scope,
            page: response.page,
            total_count: response.total_count,
            reviews,
        });
    }

    collect_pull_request_review_pages(expected_scope, pages)
}

/// Combine a complete, ordered Forgejo review pagination read into one
/// snapshot. This detects omitted/repeated pages, inconsistent count headers,
/// duplicate review records, and lists beyond the processing bound.
pub fn collect_pull_request_review_pages(
    expected_scope: &PullRequestReviewScopeV1,
    pages: Vec<PullRequestReviewPageV1>,
) -> Result<PullRequestReviewsSnapshotV1, MergeApprovalError> {
    if pages.is_empty() {
        return Err(MergeApprovalError::IncompleteReviewList);
    }
    if pages[0].scope != *expected_scope {
        return Err(MergeApprovalError::WrongScope);
    }
    let total_count = pages[0].total_count;
    if total_count > MAX_MERGE_APPROVAL_REVIEW_ENTRIES as u64 {
        return Err(MergeApprovalError::TooManyReviews);
    }
    if pages.len() as u64 > total_count.saturating_add(1)
        || pages.len() > MAX_MERGE_APPROVAL_REVIEW_ENTRIES + 1
        || (total_count == 0 && pages.len() != 1)
    {
        return Err(MergeApprovalError::IncompleteReviewList);
    }

    let mut reviews = Vec::new();
    let mut review_ids = HashSet::new();
    let expected_pull_request_url = expected_scope.pull_request_url();
    for (index, page) in pages.iter().enumerate() {
        let expected_page =
            u32::try_from(index + 1).map_err(|_| MergeApprovalError::TooManyReviews)?;
        if page.scope != *expected_scope {
            return Err(MergeApprovalError::WrongScope);
        }
        if page.page != expected_page || page.total_count != total_count {
            return Err(MergeApprovalError::IncompleteReviewList);
        }
        if page.reviews.is_empty() && (total_count != 0 || index + 1 != pages.len()) {
            return Err(MergeApprovalError::IncompleteReviewList);
        }
        for review in &page.reviews {
            if review.id == 0
                || !review_ids.insert(review.id)
                || !review_fields_within_limits(review, &expected_pull_request_url)
            {
                return Err(MergeApprovalError::InvalidReviewSet);
            }
            reviews.push(review.clone());
            if reviews.len() > MAX_MERGE_APPROVAL_REVIEW_ENTRIES {
                return Err(MergeApprovalError::TooManyReviews);
            }
        }
    }
    if reviews.len() as u64 != total_count {
        return Err(MergeApprovalError::IncompleteReviewList);
    }

    Ok(PullRequestReviewsSnapshotV1 {
        scope: expected_scope.clone(),
        total_count,
        reviews,
    })
}

/// Versioned-policy projection used only by the trusted merge gate.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MergeApprovalPolicyV1 {
    pub schema_version: u32,
    pub required_approvals: u32,
    pub authorized_reviewer_ids: Vec<u64>,
}

/// Ephemeral evaluation output. It is not a signed proof or merge authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeApprovalDecisionV1 {
    pub scope: PullRequestReviewScopeV1,
    /// Verifier identity declared by the host-loaded, proof-bound policy.
    pub verifier_id: String,
    pub candidate_commit: String,
    /// SHA-256 fingerprint of the complete typed review snapshot, canonicalized by review ID.
    /// This records which DTO was evaluated; it does not prove API provenance, freshness, or correctness.
    pub review_snapshot_sha256: String,
    pub approved_review_ids: Vec<u64>,
    pub reviewer_ids: Vec<u64>,
}

#[derive(Serialize)]
struct CanonicalPullRequestReviewBaseBindingsV1<'a> {
    schema_version: u32,
    scope: &'a PullRequestReviewScopeV1,
    bindings: Vec<&'a PullRequestReviewBaseBindingV1>,
}

#[derive(Serialize)]
struct CanonicalPullRequestReviewsV2<'a> {
    // Canonical bytes are compact serde_json for this v2 field order, with reviews sorted by ID.
    // The golden hash test pins the shape; any shape change requires a new canonical schema version.
    schema_version: u32,
    scope: &'a PullRequestReviewScopeV1,
    total_count: u64,
    reviews: Vec<&'a PullRequestReviewV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeApprovalError {
    WrongScope,
    WrongPullRequest,
    InvalidBinding,
    InvalidPolicy,
    VerifierMismatch,
    NoApprovalPolicy,
    StalePolicy,
    IncompleteReviewList,
    TooManyReviews,
    InvalidReviewSet,
    ReviewResponseReadFailed,
    ReviewPageBodyTooLarge,
    ReviewSnapshotBodyTooLarge,
    InvalidReviewResponse,
    MissingReviewBaseBinding,
    StaleReviewBaseBinding,
    InvalidReviewBaseBindings,
    InsufficientApprovals { required: usize, found: usize },
}

/// Evaluate current Forgejo approvals against the exact candidate and the
/// host-loaded reviewer policy. Forgejo's `official` flag identifies reviews
/// that count as a reviewer's current official decision; this gate adds stricter
/// requirements that it is approved, not stale or dismissed, and on this head.
///
/// This function only evaluates a complete snapshot and verifies the exact
/// policy bytes against the policy hash bound by the signed proof. It does not
/// authenticate the Forgejo reader, prove that reviewers are human, or
/// authorize a merge. The caller must load `current_policy_bytes` from the
/// proof-bound protected commit and keep this check inside the final merge gate.
pub fn validate_merge_approvals(
    snapshot: &PullRequestReviewsSnapshotV1,
    expected_scope: &PullRequestReviewScopeV1,
    candidate_commit: &str,
    execution_reviewer_id: u64,
    expected_policy_sha256: &str,
    current_policy_bytes: &[u8],
) -> Result<MergeApprovalDecisionV1, MergeApprovalError> {
    if snapshot.scope != *expected_scope {
        if snapshot.scope.origin == expected_scope.origin
            && snapshot.scope.owner == expected_scope.owner
            && snapshot.scope.repository == expected_scope.repository
        {
            return Err(MergeApprovalError::WrongPullRequest);
        }
        return Err(MergeApprovalError::WrongScope);
    }
    if !valid_commit_id(candidate_commit)
        || execution_reviewer_id == 0
        || !valid_digest(expected_policy_sha256)
    {
        return Err(MergeApprovalError::InvalidBinding);
    }

    if current_policy_bytes.is_empty()
        || current_policy_bytes.len() > MAX_MERGE_APPROVAL_POLICY_BYTES
    {
        return Err(MergeApprovalError::InvalidPolicy);
    }
    if crate::sha256(current_policy_bytes) != expected_policy_sha256 {
        return Err(MergeApprovalError::StalePolicy);
    }
    let verifier_policy: crate::Policy = serde_json::from_slice(current_policy_bytes)
        .map_err(|_| MergeApprovalError::InvalidPolicy)?;
    if verifier_policy.schema_version != 1
        || verifier_policy.revision == 0
        || verifier_policy.verifier_id.trim().is_empty()
        || verifier_policy.timeout_seconds == 0
    {
        return Err(MergeApprovalError::InvalidPolicy);
    }
    let policy = verifier_policy
        .merge_approval_policy
        .as_ref()
        .ok_or(MergeApprovalError::NoApprovalPolicy)?;

    if policy.authorized_reviewer_ids.len() > MAX_MERGE_APPROVAL_REVIEWERS {
        return Err(MergeApprovalError::InvalidPolicy);
    }
    let mut allowed_reviewers = HashSet::with_capacity(policy.authorized_reviewer_ids.len());
    if policy.schema_version != 1
        || policy.required_approvals == 0
        || policy.authorized_reviewer_ids.is_empty()
        || policy
            .authorized_reviewer_ids
            .iter()
            .any(|reviewer_id| *reviewer_id == 0 || !allowed_reviewers.insert(*reviewer_id))
        || usize::try_from(policy.required_approvals)
            .map(|required| required > allowed_reviewers.len())
            .unwrap_or(true)
    {
        return Err(MergeApprovalError::InvalidPolicy);
    }
    if snapshot.total_count > MAX_MERGE_APPROVAL_REVIEW_ENTRIES as u64 {
        return Err(MergeApprovalError::TooManyReviews);
    }
    if snapshot.total_count != snapshot.reviews.len() as u64 {
        return Err(MergeApprovalError::IncompleteReviewList);
    }

    let mut review_ids = HashSet::with_capacity(snapshot.reviews.len());
    let mut approvals_by_reviewer = BTreeMap::new();
    let expected_pull_request_url = expected_scope.pull_request_url();
    for review in &snapshot.reviews {
        if review.id == 0
            || !review_ids.insert(review.id)
            || !review_fields_within_limits(review, &expected_pull_request_url)
        {
            return Err(MergeApprovalError::InvalidReviewSet);
        }
        if review.state != "APPROVED"
            || !review.official
            || review.stale
            || review.dismissed
            || review.commit_id != candidate_commit
        {
            continue;
        }
        let Some(reviewer_id) = review.user.as_ref().map(|user| user.id) else {
            continue;
        };
        if reviewer_id == 0
            || reviewer_id == execution_reviewer_id
            || !allowed_reviewers.contains(&reviewer_id)
        {
            continue;
        }
        if approvals_by_reviewer
            .insert(reviewer_id, review.id)
            .is_some()
        {
            return Err(MergeApprovalError::InvalidReviewSet);
        }
    }

    let required_approvals = policy.required_approvals as usize;
    if approvals_by_reviewer.len() < required_approvals {
        return Err(MergeApprovalError::InsufficientApprovals {
            required: required_approvals,
            found: approvals_by_reviewer.len(),
        });
    }
    let (reviewer_ids, approved_review_ids): (Vec<_>, Vec<_>) =
        approvals_by_reviewer.into_iter().unzip();

    let mut canonical_reviews = snapshot.reviews.iter().collect::<Vec<_>>();
    canonical_reviews.sort_unstable_by_key(|review| review.id);
    let canonical_snapshot = CanonicalPullRequestReviewsV2 {
        schema_version: 2,
        scope: &snapshot.scope,
        total_count: snapshot.total_count,
        reviews: canonical_reviews,
    };
    let review_snapshot_bytes = serde_json::to_vec(&canonical_snapshot)
        .map_err(|_| MergeApprovalError::InvalidReviewSet)?;

    Ok(MergeApprovalDecisionV1 {
        scope: snapshot.scope.clone(),
        verifier_id: verifier_policy.verifier_id.clone(),
        candidate_commit: candidate_commit.to_owned(),
        review_snapshot_sha256: crate::sha256(&review_snapshot_bytes),
        approved_review_ids,
        reviewer_ids,
    })
}

/// Inputs that must come from the trusted mission and verifier records, not
/// from the agent's workspace or a status posted by that agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergePreflightContextV1 {
    pub pull_request_number: u64,
    pub execution_identity: String,
    pub expected: ExpectedBinding,
}

/// Writer model admitted by the trusted local host before it may authorize a
/// merge. Selecting `LocalSingleWriterSandbox` is a host assertion about the
/// surrounding sandbox; this value cannot prove that the sandbox excludes
/// every non-cooperating Forgejo writer. `ForgejoV17MultiWriter` is the audited
/// upstream profile, whose merge API does not require an expected-base CAS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeWriterAdmissionProfileV1 {
    LocalSingleWriterSandbox,
    ForgejoV17MultiWriter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeWriterAdmissionErrorV1 {
    LocalSingleWriterSupervisorAttestationUnavailable,
    ServerExpectedBasePreconditionUnavailable,
}

impl MergeWriterAdmissionErrorV1 {
    pub const fn code(self) -> &'static str {
        match self {
            Self::LocalSingleWriterSupervisorAttestationUnavailable => {
                "single_writer_supervisor_attestation_unavailable"
            }
            Self::ServerExpectedBasePreconditionUnavailable => {
                "server_expected_base_precondition_unavailable"
            }
        }
    }

    pub const fn action(self) -> &'static str {
        match self {
            Self::LocalSingleWriterSupervisorAttestationUnavailable => {
                "run the merge gate only under a supervisor that attests Forgejo and the gate share an isolation boundary excluding every other writer"
            }
            Self::ServerExpectedBasePreconditionUnavailable => {
                "run the gate only inside a sandbox that excludes every other Forgejo writer, or use a server that requires the verified base commit on every merge path"
            }
        }
    }
}

/// Reject a multi-writer Forgejo profile when no mandatory server-side
/// expected-base precondition has been established. This policy deliberately
/// models only the audited local sandbox and Forgejo v17 profiles; it does not
/// claim that Forgejo v17 has a CAS-capable merge route. The attestation flag
/// is a trust-boundary input, not proof generated by this function; callers
/// must obtain it from a supervisor that enforces the whole-instance boundary.
/// The production CLI currently supplies `false`; only the in-memory gate
/// fixture supplies `true` while exercising the downstream merge logic.
pub fn validate_merge_writer_admission_v1(
    profile: MergeWriterAdmissionProfileV1,
    host_attests_single_writer_exclusion: bool,
) -> Result<(), MergeWriterAdmissionErrorV1> {
    match profile {
        MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox => {
            if host_attests_single_writer_exclusion {
                Ok(())
            } else {
                Err(MergeWriterAdmissionErrorV1::LocalSingleWriterSupervisorAttestationUnavailable)
            }
        }
        MergeWriterAdmissionProfileV1::ForgejoV17MultiWriter => {
            Err(MergeWriterAdmissionErrorV1::ServerExpectedBasePreconditionUnavailable)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MergePreflightError {
    WrongScope,
    WrongPullRequest,
    InvalidSnapshot,
    PullRequestNotOpen,
    AlreadyMerged,
    DraftPullRequest,
    MergeabilityUnknown,
    NotMergeable,
    InvalidCommitId,
    InvalidExpectedBinding,
    ExpectedPostmergeProof,
    VerifierNotIndependent,
    StaleBinding(&'static str),
    Proof(GateError),
}

/// Check that a trusted current PR snapshot still matches a passing,
/// verifier-signed pre-merge proof and the host-supplied current expectations.
/// This is a read-only preflight: it does not check human approval and must not
/// be treated as authorization to call Forgejo's merge endpoint.
pub fn validate_merge_preflight(
    scope: &BaseBranchScope,
    current: &PullRequestSnapshotV1,
    proof: Option<&SignedProof>,
    verifier_key: &[u8],
    context: &MergePreflightContextV1,
) -> Result<(), MergePreflightError> {
    validate_merge_preflight_state(scope, current, context)?;
    verify_proof(proof, verifier_key, &context.expected).map_err(MergePreflightError::Proof)
}

/// Target-bound counterpart used only by a merge authority. The expected
/// target must be constructed by its trusted host from the selected profile.
pub fn validate_target_bound_merge_preflight_v2(
    scope: &BaseBranchScope,
    current: &PullRequestSnapshotV1,
    proof: Option<&TargetBoundProofV2>,
    verifier_key: &[u8],
    expected_target: &ProofTargetV2,
    context: &MergePreflightContextV1,
) -> Result<(), MergePreflightError> {
    validate_merge_preflight_state(scope, current, context)?;
    verify_target_bound_proof_v2(proof, verifier_key, expected_target, &context.expected)
        .map_err(MergePreflightError::Proof)
}

fn validate_merge_preflight_state(
    scope: &BaseBranchScope,
    current: &PullRequestSnapshotV1,
    context: &MergePreflightContextV1,
) -> Result<(), MergePreflightError> {
    if current.number == 0 || current.number != context.pull_request_number {
        return Err(MergePreflightError::WrongPullRequest);
    }
    if normalize_origin(&current.origin).as_deref() != Some(scope.origin.as_str())
        || current.owner != scope.owner
        || current.repository != scope.repository
        || current.base_branch != scope.branch
    {
        return Err(MergePreflightError::WrongScope);
    }
    if current.merged {
        return Err(MergePreflightError::AlreadyMerged);
    }
    if current.state != "open" {
        return Err(MergePreflightError::PullRequestNotOpen);
    }
    if current.draft {
        return Err(MergePreflightError::DraftPullRequest);
    }
    match current.mergeable {
        Some(true) => {}
        Some(false) => return Err(MergePreflightError::NotMergeable),
        None => return Err(MergePreflightError::MergeabilityUnknown),
    }

    if !valid_commit_id(&current.base_sha)
        || !valid_commit_id(&current.head_sha)
        || !valid_commit_id(&context.expected.base_commit)
        || !valid_commit_id(&context.expected.candidate_commit)
    {
        return Err(MergePreflightError::InvalidCommitId);
    }
    if !valid_digest(&context.expected.requirement_sha256)
        || !valid_digest(&context.expected.policy_sha256)
        || context.expected.verifier_id.trim().is_empty()
        || context.execution_identity.trim().is_empty()
    {
        return Err(MergePreflightError::InvalidExpectedBinding);
    }
    if context.expected.merge_commit.is_some() {
        return Err(MergePreflightError::ExpectedPostmergeProof);
    }
    if context.expected.verifier_id == context.execution_identity {
        return Err(MergePreflightError::VerifierNotIndependent);
    }
    if current.base_sha != context.expected.base_commit {
        return Err(MergePreflightError::StaleBinding("base"));
    }
    if current.head_sha != context.expected.candidate_commit {
        return Err(MergePreflightError::StaleBinding("candidate"));
    }
    Ok(())
}

/// Read-only result combining a current PR preflight with an independently
/// validated approval snapshot. This is a readiness decision for a later
/// trusted gate, not a merge authorization or a claim that the Forgejo reads
/// were authenticated or atomic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeReadinessDecisionV1 {
    pub origin: String,
    pub owner: String,
    pub repository: String,
    pub pull_request_number: u64,
    pub base_branch: String,
    pub base_commit: String,
    pub candidate_commit: String,
    pub requirement_sha256: String,
    pub policy_sha256: String,
    pub verifier_id: String,
    pub review_snapshot_sha256: String,
    pub review_base_bindings_sha256: String,
    pub approved_review_ids: Vec<u64>,
    pub reviewer_ids: Vec<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MergeReadinessError {
    Preflight(MergePreflightError),
    Approval(MergeApprovalError),
}

/// Compose the verifier proof, a current PR snapshot, current Forgejo review
/// pages and host-loaded policy bytes into one scoped, read-only readiness
/// decision. `execution_reviewer_id` must be resolved by the trusted host from
/// the same authenticated execution identity represented in `context`.
///
/// Callers must obtain every input from trusted sources. This function does
/// not authenticate those sources, guarantee that paged reviews were read
/// atomically, re-read the PR at merge time, or authorize a merge request.
pub fn evaluate_merge_readiness(
    branch_scope: &BaseBranchScope,
    current: &PullRequestSnapshotV1,
    reviews: &PullRequestReviewsSnapshotV1,
    review_base_bindings: &PullRequestReviewBaseBindingsSnapshotV1,
    proof: Option<&SignedProof>,
    verifier_key: &[u8],
    context: &MergePreflightContextV1,
    execution_reviewer_id: u64,
    current_policy_bytes: &[u8],
) -> Result<MergeReadinessDecisionV1, MergeReadinessError> {
    validate_merge_preflight(branch_scope, current, proof, verifier_key, context)
        .map_err(MergeReadinessError::Preflight)?;

    evaluate_merge_approvals(
        branch_scope,
        current,
        reviews,
        review_base_bindings,
        context,
        execution_reviewer_id,
        current_policy_bytes,
    )
}

/// Target-bound readiness path for a gate that may write to Forgejo. V1
/// readiness remains available for its existing read-only preflight callers.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_target_bound_merge_readiness_v2(
    branch_scope: &BaseBranchScope,
    current: &PullRequestSnapshotV1,
    reviews: &PullRequestReviewsSnapshotV1,
    review_base_bindings: &PullRequestReviewBaseBindingsSnapshotV1,
    proof: Option<&TargetBoundProofV2>,
    verifier_key: &[u8],
    expected_target: &ProofTargetV2,
    context: &MergePreflightContextV1,
    execution_reviewer_id: u64,
    current_policy_bytes: &[u8],
) -> Result<MergeReadinessDecisionV1, MergeReadinessError> {
    validate_target_bound_merge_preflight_v2(
        branch_scope,
        current,
        proof,
        verifier_key,
        expected_target,
        context,
    )
    .map_err(MergeReadinessError::Preflight)?;

    evaluate_merge_approvals(
        branch_scope,
        current,
        reviews,
        review_base_bindings,
        context,
        execution_reviewer_id,
        current_policy_bytes,
    )
}

#[allow(clippy::too_many_arguments)]
fn evaluate_merge_approvals(
    branch_scope: &BaseBranchScope,
    current: &PullRequestSnapshotV1,
    reviews: &PullRequestReviewsSnapshotV1,
    review_base_bindings: &PullRequestReviewBaseBindingsSnapshotV1,
    context: &MergePreflightContextV1,
    execution_reviewer_id: u64,
    current_policy_bytes: &[u8],
) -> Result<MergeReadinessDecisionV1, MergeReadinessError> {
    let review_scope = PullRequestReviewScopeV1::new(
        branch_scope.origin(),
        branch_scope.owner(),
        branch_scope.repository(),
        context.pull_request_number,
    )
    .map_err(MergeReadinessError::Approval)?;
    let approval = validate_merge_approvals(
        reviews,
        &review_scope,
        &current.head_sha,
        execution_reviewer_id,
        &context.expected.policy_sha256,
        current_policy_bytes,
    )
    .map_err(MergeReadinessError::Approval)?;
    if approval.verifier_id != context.expected.verifier_id {
        return Err(MergeReadinessError::Approval(
            MergeApprovalError::VerifierMismatch,
        ));
    }
    let review_base_bindings_sha256 = validate_review_base_bindings(
        review_base_bindings,
        &review_scope,
        current,
        &approval.approved_review_ids,
    )
    .map_err(MergeReadinessError::Approval)?;

    Ok(MergeReadinessDecisionV1 {
        origin: branch_scope.origin().to_owned(),
        owner: branch_scope.owner().to_owned(),
        repository: branch_scope.repository().to_owned(),
        pull_request_number: current.number,
        base_branch: current.base_branch.clone(),
        base_commit: current.base_sha.clone(),
        candidate_commit: current.head_sha.clone(),
        requirement_sha256: context.expected.requirement_sha256.clone(),
        policy_sha256: context.expected.policy_sha256.clone(),
        verifier_id: context.expected.verifier_id.clone(),
        review_snapshot_sha256: approval.review_snapshot_sha256,
        review_base_bindings_sha256,
        approved_review_ids: approval.approved_review_ids,
        reviewer_ids: approval.reviewer_ids,
    })
}

fn validate_review_base_bindings(
    snapshot: &PullRequestReviewBaseBindingsSnapshotV1,
    expected_scope: &PullRequestReviewScopeV1,
    current: &PullRequestSnapshotV1,
    approved_review_ids: &[u64],
) -> Result<String, MergeApprovalError> {
    if snapshot.scope != *expected_scope {
        if snapshot.scope.origin == expected_scope.origin
            && snapshot.scope.owner == expected_scope.owner
            && snapshot.scope.repository == expected_scope.repository
        {
            return Err(MergeApprovalError::WrongPullRequest);
        }
        return Err(MergeApprovalError::WrongScope);
    }
    if snapshot.bindings.len() > MAX_MERGE_APPROVAL_REVIEW_ENTRIES {
        return Err(MergeApprovalError::InvalidReviewBaseBindings);
    }

    let approved_review_ids = approved_review_ids.iter().copied().collect::<HashSet<_>>();
    let mut bindings_by_review_id = BTreeMap::new();
    for binding in &snapshot.bindings {
        if binding.review_id == 0
            || !valid_branch(&binding.base_branch)
            || !valid_commit_id(&binding.base_commit)
            || !approved_review_ids.contains(&binding.review_id)
            || bindings_by_review_id
                .insert(binding.review_id, binding)
                .is_some()
        {
            return Err(MergeApprovalError::InvalidReviewBaseBindings);
        }
    }

    for review_id in &approved_review_ids {
        let binding = bindings_by_review_id
            .get(review_id)
            .ok_or(MergeApprovalError::MissingReviewBaseBinding)?;
        if binding.base_branch != current.base_branch || binding.base_commit != current.base_sha {
            return Err(MergeApprovalError::StaleReviewBaseBinding);
        }
    }

    let mut canonical_bindings = snapshot.bindings.iter().collect::<Vec<_>>();
    canonical_bindings.sort_unstable_by_key(|binding| binding.review_id);
    let canonical_snapshot = CanonicalPullRequestReviewBaseBindingsV1 {
        schema_version: 1,
        scope: &snapshot.scope,
        bindings: canonical_bindings,
    };
    let canonical_bytes = serde_json::to_vec(&canonical_snapshot)
        .map_err(|_| MergeApprovalError::InvalidReviewBaseBindings)?;
    Ok(crate::sha256(&canonical_bytes))
}

fn valid_commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// An acquired local interprocess lock for one base branch.
///
/// Keep this value alive from the final merge preflight through the merge
/// result readback. Lock files are stable coordination identities and must not
/// be deleted while any cooperating process may still use them.
pub struct BaseBranchMergeLock {
    file: File,
}

impl BaseBranchMergeLock {
    /// Acquire the lock in a dedicated, owner-only directory on a local
    /// filesystem. `timeout` bounds waiting for another cooperating process.
    pub fn acquire(
        lock_root: &Path,
        scope: &BaseBranchScope,
        timeout: Duration,
    ) -> Result<Self, MergeLockError> {
        prepare_lock_root(lock_root)?;
        let path = lock_root.join(format!("base-{}.lock", scope.lock_key()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| {
                if error.raw_os_error() == Some(libc::ELOOP) {
                    MergeLockError::UntrustedLockFile
                } else {
                    MergeLockError::Io(error)
                }
            })?;

        let metadata = file.metadata().map_err(MergeLockError::Io)?;
        let effective_uid = unsafe { libc::geteuid() };
        if !metadata.file_type().is_file()
            || metadata.uid() != effective_uid
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(MergeLockError::UntrustedLockFile);
        }

        let started = Instant::now();
        loop {
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result == 0 {
                return Ok(Self { file });
            }
            let error = io::Error::last_os_error();
            let raw = error.raw_os_error();
            if raw != Some(libc::EWOULDBLOCK) && raw != Some(libc::EAGAIN) {
                return Err(MergeLockError::Io(error));
            }

            let elapsed = started.elapsed();
            if elapsed >= timeout {
                return Err(MergeLockError::TimedOut);
            }
            thread::sleep(Duration::from_millis(10).min(timeout - elapsed));
        }
    }
}

impl Drop for BaseBranchMergeLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Debug)]
pub enum MergeLockError {
    InvalidScope,
    UntrustedLockDirectory,
    UntrustedLockFile,
    TimedOut,
    Io(io::Error),
}

impl fmt::Display for MergeLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScope => formatter.write_str("invalid Forgejo base branch scope"),
            Self::UntrustedLockDirectory => {
                formatter.write_str("lock directory must be an owner-only, non-symlink directory")
            }
            Self::UntrustedLockFile => {
                formatter.write_str("lock file must be an owner-only, non-symlink regular file")
            }
            Self::TimedOut => formatter.write_str("timed out waiting for the base branch lock"),
            Self::Io(error) => write!(formatter, "local merge lock I/O failed: {error}"),
        }
    }
}

impl std::error::Error for MergeLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

fn normalize_origin(origin: &str) -> Option<String> {
    if origin.trim() != origin
        || origin
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || origin.contains(['?', '#', '\\'])
    {
        return None;
    }
    let (scheme, remainder) = origin.split_once("://")?;
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "http" => "http",
        "https" => "https",
        _ => return None,
    };
    let authority_end = remainder.find('/').unwrap_or(remainder.len());
    let (authority, path) = remainder.split_at(authority_end);
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let path = path.trim_end_matches('/');
    Some(format!(
        "{scheme}://{}{path}",
        authority.to_ascii_lowercase()
    ))
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
}

/// Percent-encode one UTF-8 URL path segment using Forgejo-compatible escaping.
pub fn encode_url_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        // Match Go's net/url.PathEscape, which Forgejo uses for repository
        // owner and name URLs (including its treatment of '+' and sub-delims).
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.' | b'_' | b'~' | b'$' | b'&' | b'+' | b':' | b'=' | b'@'
            )
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Percent-encode each slash-delimited path segment while preserving the
/// separators, matching Forgejo v17's `util.PathEscapeSegments` for release
/// tag URLs.
pub fn encode_url_path_segments(value: &str) -> String {
    let mut segments = value.split('/');
    let Some(first) = segments.next() else {
        return String::new();
    };
    let mut encoded = encode_url_path_segment(first);
    for segment in segments {
        encoded.push('/');
        encoded.push_str(&encode_url_path_segment(segment));
    }
    encoded
}

/// Check Git refname constraints for a tag name, including slash-delimited
/// namespaces. Leading dashes remain valid for tags; callers that use a ref as
/// a branch argument should apply the branch-specific restriction separately.
pub fn valid_tag_ref_name(tag_name: &str) -> bool {
    !tag_name.is_empty()
        && tag_name.trim() == tag_name
        && !tag_name.starts_with('/')
        && !tag_name.ends_with('/')
        && !tag_name.ends_with('.')
        && !tag_name.contains("//")
        && !tag_name.contains("..")
        && !tag_name.contains("@{")
        && !tag_name.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
        && tag_name
            .split('/')
            .all(|part| !part.starts_with('.') && !part.ends_with(".lock"))
}

/// Check the Git ref constraints used by local Forgejo branch scopes.
pub fn valid_branch(branch: &str) -> bool {
    !branch.starts_with('-') && valid_tag_ref_name(branch)
}

fn prepare_lock_root(lock_root: &Path) -> Result<(), MergeLockError> {
    match fs::symlink_metadata(lock_root) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            match builder.create(lock_root) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(MergeLockError::Io(error)),
            }
        }
        Err(error) => return Err(MergeLockError::Io(error)),
    }

    let metadata = fs::symlink_metadata(lock_root).map_err(MergeLockError::Io)?;
    let effective_uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(MergeLockError::UntrustedLockDirectory);
    }
    Ok(())
}
