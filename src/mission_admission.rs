//! Host-owned admission from a Forgejo issue to one protected requirement revision.
//!
//! The binding is an explicit input from a trusted host/operator. Issue titles and
//! bodies are untrusted request content and never choose a requirement path. A
//! source adapter must read the issue from the configured Forgejo origin, confirm
//! the selected base branch is protected, and only return requirement bytes from
//! the protected path at that exact commit. This module does not authenticate the
//! operator that supplied the binding or enforce operating-system isolation.

use crate::{
    EventJournal, JournalAttestationError, Requirements,
    agent_runtime::{
        AgentExecutionRequestV1, AgentRuntimeErrorV1, ContextPacketV1, ExecutionClaimResultV1,
        MissionExecutionJournalV1, MissionGenerationAllocationV1, RuntimeCapabilitiesV1,
        UntrustedWorkItemContentV1, WorkspaceRefV1, derive_mission_identity_v1,
    },
    forgejo::{
        BaseBranchScope, MAX_MERGE_APPROVAL_REVIEW_COMMIT_ID_BYTES,
        MAX_MERGE_APPROVAL_REVIEW_ENTRIES, MAX_MERGE_APPROVAL_REVIEW_PULL_REQUEST_URL_BYTES,
        MAX_MERGE_APPROVAL_REVIEW_STATE_BYTES, PullRequestReviewScopeV1, PullRequestReviewV1,
        PullRequestReviewsSnapshotV1, RequirementArbitrationDecisionV1,
        RequirementSourcePullRequestV1,
    },
    sha256,
    webhook::reconcile::{
        ForgejoCurrentState, ReconcileError, ReconciledState, ResourceKind, event_resource_ref,
        reconcile_inbox_event,
    },
    webhook::{JournaledWebhookEventV1, WebhookError, read_journaled_webhook_event_verified},
};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};

pub const MAX_MISSION_ARBITRATION_POLICY_BYTES: usize = 64 * 1024;
pub const MAX_MISSION_ARBITRATION_REVIEWERS: usize = 1024;

/// Version-3 input binding. Its PR number is intentionally read from the
/// protected requirements document and checked against Forgejo before use.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionAdmissionBindingV3 {
    pub schema_version: u32,
    pub issue_number: u64,
    pub requirement_id: String,
    pub requirement_revision: u64,
    pub allocation_id: String,
}

/// Host-owned allowlist policy bytes, loaded outside the agent workspace.
/// Account IDs prove an authorized Forgejo identity only; Forgejo v17's public
/// review response does not establish whether that account is a human or bot.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionArbitrationPolicyScopeV1 {
    pub origin: String,
    pub owner: String,
    pub repository: String,
    pub branch: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionArbitrationPolicyV1 {
    pub schema_version: u32,
    pub revision: u64,
    pub scope: MissionArbitrationPolicyScopeV1,
    pub required_approvals: u32,
    pub authorized_reviewer_ids: Vec<u64>,
}

/// The source port must authenticate to the configured Forgejo origin, bind
/// each returned object to the requested repository/PR, fetch every review
/// page under the byte limits, and set `total_count` from X-Total-Count.
/// Review pagination is not an atomic server snapshot; the caller brackets it
/// with PR readbacks and refuses if the PR identity/ref binding changed.
pub trait MissionArbitrationSourceV1: MissionAdmissionSourceV1 {
    fn read_requirement_source_pull_request(
        &self,
        scope: &BaseBranchScope,
        pull_request_number: u64,
    ) -> Result<RequirementSourcePullRequestV1, SourceFailureV1>;

    fn read_requirement_source_reviews(
        &self,
        scope: &BaseBranchScope,
        pull_request_number: u64,
    ) -> Result<PullRequestReviewsSnapshotV1, SourceFailureV1>;
}

/// Version 2 delegates generation allocation to the trusted host journal.
/// `allocation_id` must remain stable when the host retries after interruption.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionAdmissionBindingV2 {
    pub schema_version: u32,
    pub issue_number: u64,
    pub requirement_id: String,
    pub requirement_revision: u64,
    pub allocation_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionIssueStateV1 {
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueSnapshotV1 {
    pub number: u64,
    /// Canonical UI URL read from the configured Forgejo origin.
    pub html_url: String,
    pub state: MissionIssueStateV1,
    /// Current API content is optional, user controlled, and not an authority
    /// for selecting requirements. It is copied into a separate runtime field.
    pub work_item_content: Option<UntrustedWorkItemContentV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedBranchHeadV1 {
    pub branch: String,
    pub commit: String,
    pub protected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFailureV1 {
    NotFound,
    Unauthorized,
    Forbidden,
    Unavailable,
    InvalidResponse,
    TreeTooLarge,
}

/// Read-only source port. `read_requirement_file_at` must enforce the trusted
/// protected-path policy as well as fetching the exact commit, and fail closed
/// when either property cannot be established.
pub trait MissionAdmissionSourceV1 {
    fn read_issue(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
    ) -> Result<IssueSnapshotV1, SourceFailureV1>;

    fn read_protected_branch_head(
        &self,
        scope: &BaseBranchScope,
    ) -> Result<ProtectedBranchHeadV1, SourceFailureV1>;

    fn read_requirement_file_at(
        &self,
        scope: &BaseBranchScope,
        commit: &str,
        path: &str,
    ) -> Result<Vec<u8>, SourceFailureV1>;

    /// Return every requirement JSON path at the exact immutable commit. An
    /// adapter must refuse incomplete or truncated listings; callers use this
    /// inventory to establish that an issue has only one protected binding.
    fn list_requirement_paths_at(
        &self,
        scope: &BaseBranchScope,
        commit: &str,
    ) -> Result<Vec<String>, SourceFailureV1>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionAdmissionReceiptV2 {
    pub issue_number: u64,
    pub base_commit: String,
    pub requirement_path: String,
    pub requirement_sha256: String,
    pub request: AgentExecutionRequestV1,
    pub allocation: MissionGenerationAllocationV1,
    pub claim: ExecutionClaimResultV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionAdmissionReceiptV3 {
    pub admission: MissionAdmissionReceiptV2,
    pub arbitration: RequirementArbitrationDecisionV1,
}

/// Minimal, versioned handoff from trusted admission to a host executor.
/// It carries the exact request and allocation produced by admission; it is
/// transport data, not an authority by itself. The consuming host must still
/// validate this binding against its pinned profile and durable claim journal.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionHostAdmissionV1 {
    pub schema_version: u32,
    pub scope: MissionHostScopeV1,
    pub issue_number: u64,
    pub base_commit: String,
    pub requirement_path: String,
    pub requirement_sha256: String,
    pub allocation: MissionHostAllocationV1,
    pub request: AgentExecutionRequestV1,
}

/// Stable allocation identity for cross-process handoff. Response-only
/// replay metadata is deliberately omitted so the same admitted mission has
/// identical handoff bytes after a duplicate or resumed delivery.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionHostAllocationV1 {
    pub schema_version: u32,
    pub mission_id: String,
    pub allocation_id: String,
    pub requirement_sha256: String,
    pub execution_generation: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionHostScopeV1 {
    pub origin: String,
    pub owner: String,
    pub repository: String,
    pub branch: String,
}

/// Per-delivery evidence that binds a stable mission handoff to one exact
/// validated webhook journal record. This reference is not a signature: the
/// consuming host must re-read the private inbox and compare the record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionHostAdmissionWebhookSourceV1 {
    pub schema_version: u32,
    pub delivery_id: String,
    pub journal_sequence: u64,
    pub journal_event_id: String,
    pub journal_kind: String,
    pub journal_event_sha256: String,
    pub scope: MissionHostScopeV1,
    pub issue_number: u64,
    pub mission_id: String,
    pub allocation_id: String,
    pub requirement_sha256: String,
}

impl MissionHostAdmissionV1 {
    /// Create the handoff from one completed admission result. The executor
    /// still checks the supplied DTO against its own profile and journal.
    pub fn from_receipt(scope: &BaseBranchScope, receipt: &MissionAdmissionReceiptV2) -> Self {
        Self {
            schema_version: 1,
            scope: MissionHostScopeV1 {
                origin: scope.origin().to_owned(),
                owner: scope.owner().to_owned(),
                repository: scope.repository().to_owned(),
                branch: scope.branch().to_owned(),
            },
            issue_number: receipt.issue_number,
            base_commit: receipt.base_commit.clone(),
            requirement_path: receipt.requirement_path.clone(),
            requirement_sha256: receipt.requirement_sha256.clone(),
            allocation: MissionHostAllocationV1 {
                schema_version: receipt.allocation.schema_version,
                mission_id: receipt.allocation.mission_id.clone(),
                allocation_id: receipt.allocation.allocation_id.clone(),
                requirement_sha256: receipt.allocation.requirement_sha256.clone(),
                execution_generation: receipt.allocation.execution_generation,
            },
            request: receipt.request.clone(),
        }
    }

    /// Check duplicated bindings before a host uses the handoff. Durable claim
    /// existence and authorization remain the host's responsibility.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("mission_host_admission_unsupported");
        }
        if BaseBranchScope::new(
            &self.scope.origin,
            &self.scope.owner,
            &self.scope.repository,
            &self.scope.branch,
        )
        .is_err()
            || self.issue_number == 0
            || self.base_commit.len() != 40
            || !self
                .base_commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || !valid_sha256_hex(&self.requirement_sha256)
            || !valid_requirement_id(
                self.requirement_path
                    .strip_prefix("requirements/")
                    .and_then(|value| value.strip_suffix(".json"))
                    .unwrap_or_default(),
            )
            || self.allocation.schema_version != 1
            || self.allocation.execution_generation == 0
            || !valid_allocation_id(&self.allocation.allocation_id)
            || self.allocation.mission_id != self.request.mission_id
            || self.allocation.requirement_sha256 != self.requirement_sha256
            || self.request.requirement_sha256 != self.requirement_sha256
            || self.request.workspace.base_commit != self.base_commit
            || self
                .request
                .validate_for_issue_number(self.issue_number)
                .is_err()
        {
            return Err("mission_host_admission_invalid");
        }
        let requirements: Requirements = serde_json::from_str(&self.request.requirements_json)
            .map_err(|_| "mission_host_admission_invalid")?;
        if self.requirement_path != format!("requirements/{}.json", requirements.id)
            || sha256(self.request.requirements_json.as_bytes()) != self.requirement_sha256
        {
            return Err("mission_host_admission_invalid");
        }
        Ok(())
    }
}

impl MissionHostAdmissionWebhookSourceV1 {
    /// Derive a per-delivery source reference from an event read through the
    /// private-field `JournaledWebhookEventV1` wrapper.
    pub fn from_journaled_event(
        admission: &MissionHostAdmissionV1,
        event: &JournaledWebhookEventV1,
    ) -> Result<Self, &'static str> {
        let journal_event = event.event();
        let delivery_id = journal_event
            .idempotency_key
            .strip_prefix("forgejo-delivery-v1:")
            .filter(|value| valid_webhook_delivery_id(value))
            .ok_or("host_admission_source_invalid")?;
        let resource =
            event_resource_ref(journal_event).map_err(|_| "host_admission_source_invalid")?;
        if journal_event.sequence == 0
            || journal_event.event_id.is_empty()
            || journal_event.event_id.len() > 128
            || journal_event.kind != "forgejo.webhook.v1.issues"
            || resource.kind != ResourceKind::Issue
            || resource.owner != admission.scope.owner
            || resource.repository != admission.scope.repository
            || resource.number() != Some(admission.issue_number)
        {
            return Err("host_admission_source_mismatch");
        }
        let source = Self {
            schema_version: 1,
            delivery_id: delivery_id.to_owned(),
            journal_sequence: journal_event.sequence,
            journal_event_id: journal_event.event_id.clone(),
            journal_kind: journal_event.kind.clone(),
            journal_event_sha256: journal_event.hash.clone(),
            scope: admission.scope.clone(),
            issue_number: admission.issue_number,
            mission_id: admission.allocation.mission_id.clone(),
            allocation_id: admission.allocation.allocation_id.clone(),
            requirement_sha256: admission.requirement_sha256.clone(),
        };
        source.validate_binding(admission)?;
        Ok(source)
    }

    /// Re-read the original inbox record and compare its event identity, hash,
    /// resource scope, and stable mission bindings with the host handoff.
    pub fn verify_against_inbox(
        &self,
        admission: &MissionHostAdmissionV1,
        inbox: &EventJournal,
        trusted_keys: &[VerifyingKey],
    ) -> Result<(), &'static str> {
        self.validate_binding(admission)?;
        let event = read_journaled_webhook_event_verified(inbox, &self.delivery_id, trusted_keys)
            .map_err(|error| match error {
            WebhookError::Attestation(JournalAttestationError::Missing) => {
                "host_journal_attestation_missing"
            }
            WebhookError::Attestation(JournalAttestationError::UntrustedKey) => {
                "host_journal_attestation_untrusted_key"
            }
            WebhookError::Attestation(_) => "host_journal_attestation_invalid",
            _ => "host_admission_source_unavailable",
        })?;
        let journal_event = event.event();
        if journal_event.sequence != self.journal_sequence
            || journal_event.event_id != self.journal_event_id
            || journal_event.kind != self.journal_kind
            || journal_event.hash != self.journal_event_sha256
        {
            return Err("host_admission_source_mismatch");
        }
        let resource =
            event_resource_ref(journal_event).map_err(|_| "host_admission_source_mismatch")?;
        if resource.kind != ResourceKind::Issue
            || resource.owner != admission.scope.owner
            || resource.repository != admission.scope.repository
            || resource.number() != Some(admission.issue_number)
        {
            return Err("host_admission_source_mismatch");
        }
        Ok(())
    }

    fn validate_binding(&self, admission: &MissionHostAdmissionV1) -> Result<(), &'static str> {
        if self.schema_version != 1
            || !valid_webhook_delivery_id(&self.delivery_id)
            || self.journal_sequence == 0
            || self.journal_event_id.is_empty()
            || self.journal_event_id.len() > 128
            || self.journal_kind != "forgejo.webhook.v1.issues"
            || !valid_sha256_hex(&self.journal_event_sha256)
            || self.scope != admission.scope
            || self.issue_number != admission.issue_number
            || self.mission_id != admission.allocation.mission_id
            || self.allocation_id != admission.allocation.allocation_id
            || self.requirement_sha256 != admission.requirement_sha256
        {
            return Err("host_admission_source_mismatch");
        }
        Ok(())
    }
}

fn valid_webhook_delivery_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Result of reconciling a journaled issue notification and then admitting the
/// explicitly host-bound issue against a fresh protected-branch snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookMissionAdmissionReceiptV1 {
    pub reconciled: ReconciledState,
    pub admission: MissionAdmissionReceiptV2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookMissionAdmissionReceiptV3 {
    pub reconciled: ReconciledState,
    pub admission: MissionAdmissionReceiptV3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebhookMissionAdmissionErrorV1 {
    Reconciliation(ReconcileError),
    ResourceMismatch,
    IssueNotOpen,
    Admission(AgentRuntimeErrorV1),
}

/// Reconcile an inbox notification and admit one explicit host binding.
///
/// The argument can only be constructed by reading a validated journal entry.
/// The notification can identify only a resource to re-read. It cannot select
/// the requirement, revision, allocation ID, or workspace. An expected digest
/// pins the protected requirement bytes chosen by the host; the issue is fetched
/// again before claim, so a close between reads blocks admission. Delivery IDs
/// are provenance only and do not affect mission identity or allocation.
pub fn admit_reconciled_issue_v2(
    event: &JournaledWebhookEventV1,
    reconciliation_source: &impl ForgejoCurrentState,
    scope: &BaseBranchScope,
    binding: &MissionAdmissionBindingV2,
    expected_requirement_sha256: &str,
    admission_source: &dyn MissionAdmissionSourceV1,
    context: ContextPacketV1,
    workspace_id: &str,
    capabilities: RuntimeCapabilitiesV1,
    journal: &MissionExecutionJournalV1,
) -> Result<WebhookMissionAdmissionReceiptV1, WebhookMissionAdmissionErrorV1> {
    validate_binding_v2(binding).map_err(WebhookMissionAdmissionErrorV1::Admission)?;
    if !valid_sha256_hex(expected_requirement_sha256) {
        return Err(WebhookMissionAdmissionErrorV1::Admission(error(
            "mission_requirement_hash_invalid",
            "supply the lowercase SHA-256 digest of the selected protected requirements bytes",
            json!({"requirement_id":binding.requirement_id,"requirement_revision":binding.requirement_revision}),
        )));
    }

    let journal_event = event.event();
    let resource = event_resource_ref(journal_event)
        .map_err(WebhookMissionAdmissionErrorV1::Reconciliation)?;
    if resource.kind != ResourceKind::Issue
        || resource.owner != scope.owner()
        || resource.repository != scope.repository()
        || resource.number() != Some(binding.issue_number)
    {
        return Err(WebhookMissionAdmissionErrorV1::ResourceMismatch);
    }
    let expected_html_url = format!(
        "{}/{}/{}/issues/{}",
        scope.origin(),
        scope.owner(),
        scope.repository(),
        binding.issue_number
    );
    if reconciliation_source.expected_html_url(&resource) != expected_html_url {
        return Err(WebhookMissionAdmissionErrorV1::ResourceMismatch);
    }

    let reconciled = reconcile_inbox_event(journal_event, reconciliation_source)
        .map_err(WebhookMissionAdmissionErrorV1::Reconciliation)?;
    if reconciled.resource.kind != ResourceKind::Issue
        || reconciled.resource.owner != scope.owner()
        || reconciled.resource.repository != scope.repository()
        || reconciled.resource.number() != Some(binding.issue_number)
        || reconciled.html_url != expected_html_url
    {
        return Err(WebhookMissionAdmissionErrorV1::ResourceMismatch);
    }
    if reconciled.state != "open" {
        return Err(WebhookMissionAdmissionErrorV1::IssueNotOpen);
    }

    let admission = admit_and_claim_mission_v2_with_expected_hash(
        scope,
        binding,
        admission_source,
        context,
        workspace_id,
        capabilities,
        journal,
        Some(expected_requirement_sha256),
    )
    .map_err(WebhookMissionAdmissionErrorV1::Admission)?;

    Ok(WebhookMissionAdmissionReceiptV1 {
        reconciled,
        admission,
    })
}

/// Strict schema-v3 webhook admission. A request cannot choose the policy or
/// become a mission until its protected requirements bytes are read from the
/// source PR merge commit, match the current protected branch, and have a
/// complete, authorized review snapshot.
#[allow(clippy::too_many_arguments)]
pub fn admit_reconciled_issue_v3(
    event: &JournaledWebhookEventV1,
    reconciliation_source: &impl ForgejoCurrentState,
    scope: &BaseBranchScope,
    binding: &MissionAdmissionBindingV3,
    expected_requirement_sha256: &str,
    expected_policy_sha256: &str,
    policy_bytes: &[u8],
    admission_source: &dyn MissionArbitrationSourceV1,
    context: ContextPacketV1,
    workspace_id: &str,
    capabilities: RuntimeCapabilitiesV1,
    journal: &MissionExecutionJournalV1,
) -> Result<WebhookMissionAdmissionReceiptV3, WebhookMissionAdmissionErrorV1> {
    validate_binding_v3(binding).map_err(WebhookMissionAdmissionErrorV1::Admission)?;
    if !valid_sha256_hex(expected_requirement_sha256) || !valid_sha256_hex(expected_policy_sha256) {
        return Err(WebhookMissionAdmissionErrorV1::Admission(error(
            "mission_requirement_arbitration_hash_invalid",
            "supply lowercase SHA-256 digests for the selected requirement bytes and host policy bytes",
            json!({"requirement_id":binding.requirement_id,"requirement_revision":binding.requirement_revision}),
        )));
    }

    let journal_event = event.event();
    let resource = event_resource_ref(journal_event)
        .map_err(WebhookMissionAdmissionErrorV1::Reconciliation)?;
    if resource.kind != ResourceKind::Issue
        || resource.owner != scope.owner()
        || resource.repository != scope.repository()
        || resource.number() != Some(binding.issue_number)
    {
        return Err(WebhookMissionAdmissionErrorV1::ResourceMismatch);
    }
    let expected_html_url = format!(
        "{}/{}/{}/issues/{}",
        scope.origin(),
        scope.owner(),
        scope.repository(),
        binding.issue_number
    );
    if reconciliation_source.expected_html_url(&resource) != expected_html_url {
        return Err(WebhookMissionAdmissionErrorV1::ResourceMismatch);
    }
    let reconciled = reconcile_inbox_event(journal_event, reconciliation_source)
        .map_err(WebhookMissionAdmissionErrorV1::Reconciliation)?;
    if reconciled.resource.kind != ResourceKind::Issue
        || reconciled.resource.owner != scope.owner()
        || reconciled.resource.repository != scope.repository()
        || reconciled.resource.number() != Some(binding.issue_number)
        || reconciled.html_url != expected_html_url
    {
        return Err(WebhookMissionAdmissionErrorV1::ResourceMismatch);
    }
    if reconciled.state != "open" {
        return Err(WebhookMissionAdmissionErrorV1::IssueNotOpen);
    }

    let admission = admit_and_claim_mission_v3(
        scope,
        binding,
        expected_requirement_sha256,
        expected_policy_sha256,
        policy_bytes,
        admission_source,
        context,
        workspace_id,
        capabilities,
        journal,
    )
    .map_err(WebhookMissionAdmissionErrorV1::Admission)?;
    Ok(WebhookMissionAdmissionReceiptV3 {
        reconciled,
        admission,
    })
}

/// Version 2 reads the protected requirement snapshot, allocates a generation
/// durably, builds the request, and then claims that exact request. Repeating
/// the same `allocation_id` after a crash reuses its generation.
pub fn admit_and_claim_mission_v2(
    scope: &BaseBranchScope,
    binding: &MissionAdmissionBindingV2,
    source: &dyn MissionAdmissionSourceV1,
    context: ContextPacketV1,
    workspace_id: &str,
    capabilities: RuntimeCapabilitiesV1,
    journal: &MissionExecutionJournalV1,
) -> Result<MissionAdmissionReceiptV2, AgentRuntimeErrorV1> {
    admit_and_claim_mission_v2_with_expected_hash(
        scope,
        binding,
        source,
        context,
        workspace_id,
        capabilities,
        journal,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn admit_and_claim_mission_v3(
    scope: &BaseBranchScope,
    binding: &MissionAdmissionBindingV3,
    expected_requirement_sha256: &str,
    expected_policy_sha256: &str,
    policy_bytes: &[u8],
    source: &dyn MissionArbitrationSourceV1,
    context: ContextPacketV1,
    workspace_id: &str,
    capabilities: RuntimeCapabilitiesV1,
    journal: &MissionExecutionJournalV1,
) -> Result<MissionAdmissionReceiptV3, AgentRuntimeErrorV1> {
    validate_binding_v3(binding)?;
    let arbitration = evaluate_requirement_arbitration(
        scope,
        binding,
        expected_requirement_sha256,
        expected_policy_sha256,
        policy_bytes,
        source,
    )?;
    let admission = admit_and_claim_mission_for_schema(
        scope,
        binding.issue_number,
        &binding.requirement_id,
        binding.requirement_revision,
        &binding.allocation_id,
        source,
        context,
        workspace_id,
        capabilities,
        journal,
        Some(expected_requirement_sha256),
        3,
        Some(arbitration.clone()),
    )?;
    Ok(MissionAdmissionReceiptV3 {
        admission,
        arbitration,
    })
}

fn evaluate_requirement_arbitration(
    scope: &BaseBranchScope,
    binding: &MissionAdmissionBindingV3,
    expected_requirement_sha256: &str,
    expected_policy_sha256: &str,
    policy_bytes: &[u8],
    source: &dyn MissionArbitrationSourceV1,
) -> Result<RequirementArbitrationDecisionV1, AgentRuntimeErrorV1> {
    if !valid_sha256_hex(expected_requirement_sha256)
        || !valid_sha256_hex(expected_policy_sha256)
        || policy_bytes.is_empty()
        || policy_bytes.len() > MAX_MISSION_ARBITRATION_POLICY_BYTES
        || sha256(policy_bytes) != expected_policy_sha256
    {
        return Err(error(
            "mission_requirement_arbitration_hash_invalid",
            "reload the host-pinned policy and exact protected requirements digest",
            json!({"policy_bytes":policy_bytes.len()}),
        ));
    }
    let policy_value = crate::strict_json::parse_value_rejecting_duplicate_members(policy_bytes)
        .map_err(|_| arbitration_invalid("policy JSON is invalid or contains duplicate members"))?;
    let policy: MissionArbitrationPolicyV1 = serde_json::from_value(policy_value)
        .map_err(|_| arbitration_invalid("policy schema is invalid"))?;
    let mut authorized = HashSet::with_capacity(policy.authorized_reviewer_ids.len());
    if policy.schema_version != 1
        || policy.revision == 0
        || policy.scope.origin != scope.origin()
        || policy.scope.owner != scope.owner()
        || policy.scope.repository != scope.repository()
        || policy.scope.branch != scope.branch()
        || policy.authorized_reviewer_ids.is_empty()
        || policy.authorized_reviewer_ids.len() > MAX_MISSION_ARBITRATION_REVIEWERS
        || policy
            .authorized_reviewer_ids
            .iter()
            .any(|id| *id == 0 || !authorized.insert(*id))
        || policy.required_approvals == 0
        || usize::try_from(policy.required_approvals)
            .map(|required| required > authorized.len())
            .unwrap_or(true)
    {
        return Err(arbitration_invalid(
            "policy version, reviewer allowlist, or required approval count is invalid",
        ));
    }

    let branch = source
        .read_protected_branch_head(scope)
        .map_err(source_error)?;
    if branch.branch != scope.branch() || !branch.protected || !valid_git_revision(&branch.commit) {
        return Err(error(
            "mission_base_unprotected",
            "select a protected base branch and reload its full commit ID",
            json!({"expected_branch":scope.branch(),"actual_branch":branch.branch,"protected":branch.protected}),
        ));
    }
    let requirement_path = format!("requirements/{}.json", binding.requirement_id);
    let current_bytes = source
        .read_requirement_file_at(scope, &branch.commit, &requirement_path)
        .map_err(source_error)?;
    let requirement_value = crate::strict_json::parse_value_rejecting_duplicate_members(
        &current_bytes,
    )
    .map_err(|_| {
        arbitration_invalid("protected requirements JSON is invalid or contains duplicate members")
    })?;
    let requirements: Requirements = serde_json::from_value(requirement_value)
        .map_err(|_| arbitration_invalid("protected requirements schema is invalid"))?;
    validate_requirement_document_v3(&requirements, binding)?;
    if sha256(&current_bytes) != expected_requirement_sha256 {
        return Err(error(
            "mission_requirement_hash_mismatch",
            "review the protected requirements change and update the host-pinned digest before admission",
            json!({"expected_sha256":expected_requirement_sha256,"actual_sha256":sha256(&current_bytes)}),
        ));
    }
    validate_unique_issue_binding(
        source,
        scope,
        &branch.commit,
        binding.issue_number,
        &requirement_path,
        &current_bytes,
    )?;
    let source_pull_request_number = requirements.source_pull_request_number.ok_or_else(|| {
        arbitration_invalid("schema-v3 requirement must name its source pull request")
    })?;
    let review_scope = PullRequestReviewScopeV1::new(
        scope.origin(),
        scope.owner(),
        scope.repository(),
        source_pull_request_number,
    )
    .map_err(|_| arbitration_invalid("source pull request scope is invalid"))?;

    let pull_request_before = source
        .read_requirement_source_pull_request(scope, source_pull_request_number)
        .map_err(source_error)?;
    validate_requirement_source_pull_request(scope, &review_scope, &pull_request_before)?;
    let merge_bytes = source
        .read_requirement_file_at(
            scope,
            &pull_request_before.merge_commit_sha,
            &requirement_path,
        )
        .map_err(source_error)?;
    if merge_bytes != current_bytes {
        return Err(error(
            "mission_requirement_source_mismatch",
            "make the protected requirement bytes identical to the document in the merged source pull request",
            json!({"path":requirement_path,"source_pull_request_number":source_pull_request_number,"merge_commit_sha":pull_request_before.merge_commit_sha}),
        ));
    }
    let reviews_before = source
        .read_requirement_source_reviews(scope, source_pull_request_number)
        .map_err(source_error)?;
    let pull_request_after = source
        .read_requirement_source_pull_request(scope, source_pull_request_number)
        .map_err(source_error)?;
    if pull_request_after != pull_request_before {
        return Err(error(
            "mission_requirement_review_snapshot_changed",
            "re-read the merged pull request after loading all review pages and retry if its refs changed",
            json!({"source_pull_request_number":source_pull_request_number}),
        ));
    }
    let first_decision = validate_requirement_reviews(
        &reviews_before,
        &review_scope,
        &pull_request_before.head_sha,
        &authorized,
        policy.required_approvals,
    )?;
    let reviews_after = source
        .read_requirement_source_reviews(scope, source_pull_request_number)
        .map_err(source_error)?;
    let second_decision = validate_requirement_reviews(
        &reviews_after,
        &review_scope,
        &pull_request_before.head_sha,
        &authorized,
        policy.required_approvals,
    )?;
    if first_decision != second_decision {
        return Err(error(
            "mission_requirement_review_snapshot_changed",
            "re-read all review pages until two complete, consecutive snapshots agree",
            json!({"source_pull_request_number":source_pull_request_number}),
        ));
    }
    // Bracket the final review read as well. Forgejo does not provide an
    // atomic PR/review snapshot, but this catches ref movement during both
    // consecutive full review listings before durable mission allocation.
    let pull_request_after_reviews = source
        .read_requirement_source_pull_request(scope, source_pull_request_number)
        .map_err(source_error)?;
    if pull_request_after_reviews != pull_request_before {
        return Err(error(
            "mission_requirement_review_snapshot_changed",
            "re-read the merged pull request after the final review snapshot and retry if its refs changed",
            json!({"source_pull_request_number":source_pull_request_number}),
        ));
    }
    let (review_snapshot_sha256, approved_review_ids, reviewer_ids) = first_decision;

    Ok(RequirementArbitrationDecisionV1 {
        schema_version: 1,
        source_pull_request_number,
        source_merge_commit_sha: pull_request_before.merge_commit_sha,
        source_head_sha: pull_request_before.head_sha,
        requirements_sha256: expected_requirement_sha256.to_owned(),
        policy_revision: policy.revision,
        policy_sha256: expected_policy_sha256.to_owned(),
        review_snapshot_sha256,
        approved_review_ids,
        reviewer_ids,
    })
}

fn validate_requirement_source_pull_request(
    scope: &BaseBranchScope,
    expected_scope: &PullRequestReviewScopeV1,
    pull_request: &RequirementSourcePullRequestV1,
) -> Result<(), AgentRuntimeErrorV1> {
    if pull_request.scope != *expected_scope
        || !pull_request.merged
        || pull_request.base_branch != scope.branch()
        || !valid_git_revision(&pull_request.head_sha)
        || !valid_git_revision(&pull_request.merge_commit_sha)
    {
        return Err(error(
            "mission_requirement_source_pr_invalid",
            "select a merged pull request in the configured repository and protected base branch, then re-read its full head and merge commits",
            json!({"expected_branch":scope.branch(),"actual_branch":pull_request.base_branch,"merged":pull_request.merged,"pull_request_number":expected_scope.pull_request_number()}),
        ));
    }
    Ok(())
}

fn validate_requirement_reviews(
    snapshot: &PullRequestReviewsSnapshotV1,
    expected_scope: &PullRequestReviewScopeV1,
    candidate_commit: &str,
    authorized_reviewers: &HashSet<u64>,
    required_approvals: u32,
) -> Result<(String, Vec<u64>, Vec<u64>), AgentRuntimeErrorV1> {
    if snapshot.scope != *expected_scope {
        return Err(arbitration_invalid(
            "review response belongs to another repository or pull request",
        ));
    }
    if snapshot.total_count > MAX_MERGE_APPROVAL_REVIEW_ENTRIES as u64
        || snapshot.total_count != snapshot.reviews.len() as u64
    {
        return Err(error(
            "mission_requirement_review_list_incomplete",
            "fetch every review page with a stable X-Total-Count and retry if the listing changes",
            json!({"expected_total_count":snapshot.total_count,"actual_reviews":snapshot.reviews.len()}),
        ));
    }
    let mut seen_review_ids = HashSet::with_capacity(snapshot.reviews.len());
    let mut approvals = BTreeMap::<u64, u64>::new();
    let pull_request_url = expected_scope.pull_request_url();
    for review in &snapshot.reviews {
        if review.id == 0
            || !seen_review_ids.insert(review.id)
            || review.state.len() > MAX_MERGE_APPROVAL_REVIEW_STATE_BYTES
            || review.commit_id.len() > MAX_MERGE_APPROVAL_REVIEW_COMMIT_ID_BYTES
            || review.pull_request_url.len() > MAX_MERGE_APPROVAL_REVIEW_PULL_REQUEST_URL_BYTES
            || review.pull_request_url != pull_request_url
        {
            return Err(arbitration_invalid(
                "review IDs, scope, or fields are invalid",
            ));
        }
        // Forgejo can mark an approval stale after the source PR is merged
        // because the protected base has advanced. The merged requirement
        // bytes are separately compared with that PR's merge commit, and the
        // review must still name the exact source head. A review of an older
        // PR head therefore remains ineligible through the commit binding.
        if review.state != "APPROVED"
            || !review.official
            || review.dismissed
            || review.commit_id != candidate_commit
        {
            continue;
        }
        let Some(reviewer_id) = review.user.as_ref().map(|user| user.id) else {
            continue;
        };
        if reviewer_id == 0 || !authorized_reviewers.contains(&reviewer_id) {
            continue;
        }
        if approvals.insert(reviewer_id, review.id).is_some() {
            return Err(arbitration_invalid(
                "an authorized reviewer has multiple current official approvals",
            ));
        }
    }
    if approvals.len() < required_approvals as usize {
        return Err(error(
            "mission_requirement_arbitration_insufficient",
            "obtain the required official approvals from host-authorized reviewers on the exact source head",
            json!({"required_approvals":required_approvals,"authorized_approvals":approvals.len()}),
        ));
    }
    #[derive(Serialize)]
    struct CanonicalReviews<'a> {
        schema_version: u32,
        scope: &'a PullRequestReviewScopeV1,
        total_count: u64,
        reviews: Vec<&'a PullRequestReviewV1>,
    }
    let mut canonical_reviews = snapshot.reviews.iter().collect::<Vec<_>>();
    canonical_reviews.sort_unstable_by_key(|review| review.id);
    let canonical = CanonicalReviews {
        schema_version: 1,
        scope: &snapshot.scope,
        total_count: snapshot.total_count,
        reviews: canonical_reviews,
    };
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|_| arbitration_invalid("review snapshot could not be canonicalized"))?;
    let (reviewer_ids, review_ids) = approvals.into_iter().unzip();
    Ok((sha256(&bytes), review_ids, reviewer_ids))
}

fn arbitration_invalid(message: &str) -> AgentRuntimeErrorV1 {
    error(
        "mission_requirement_arbitration_invalid",
        "repair the host-pinned requirements review policy and retry admission from a complete, scoped Forgejo read",
        json!({"reason":message}),
    )
}

fn admit_and_claim_mission_v2_with_expected_hash(
    scope: &BaseBranchScope,
    binding: &MissionAdmissionBindingV2,
    source: &dyn MissionAdmissionSourceV1,
    context: ContextPacketV1,
    workspace_id: &str,
    capabilities: RuntimeCapabilitiesV1,
    journal: &MissionExecutionJournalV1,
    expected_requirement_sha256: Option<&str>,
) -> Result<MissionAdmissionReceiptV2, AgentRuntimeErrorV1> {
    validate_binding_v2(binding)?;
    admit_and_claim_mission_for_schema(
        scope,
        binding.issue_number,
        &binding.requirement_id,
        binding.requirement_revision,
        &binding.allocation_id,
        source,
        context,
        workspace_id,
        capabilities,
        journal,
        expected_requirement_sha256,
        2,
        None,
    )
}

fn admit_and_claim_mission_for_schema(
    scope: &BaseBranchScope,
    issue_number: u64,
    requirement_id: &str,
    requirement_revision: u64,
    allocation_id: &str,
    source: &dyn MissionAdmissionSourceV1,
    context: ContextPacketV1,
    workspace_id: &str,
    capabilities: RuntimeCapabilitiesV1,
    journal: &MissionExecutionJournalV1,
    expected_requirement_sha256: Option<&str>,
    required_schema_version: u32,
    arbitration: Option<RequirementArbitrationDecisionV1>,
) -> Result<MissionAdmissionReceiptV2, AgentRuntimeErrorV1> {
    if expected_requirement_sha256.is_some_and(|digest| !valid_sha256_hex(digest)) {
        return Err(error(
            "mission_requirement_hash_invalid",
            "supply the lowercase SHA-256 digest of the selected protected requirements bytes",
            json!({"requirement_id":requirement_id,"requirement_revision":requirement_revision}),
        ));
    }
    let prepared = prepare_admission(
        scope,
        issue_number,
        requirement_id,
        requirement_revision,
        source,
        context,
        workspace_id,
        capabilities,
        expected_requirement_sha256,
        required_schema_version,
        arbitration,
        |requirement_sha256| {
            let allocation = journal.allocate_generation(
                scope,
                issue_number,
                requirement_sha256,
                allocation_id,
            )?;
            Ok(GenerationChoice {
                execution_generation: allocation.execution_generation,
                allocation: Some(allocation),
            })
        },
    )?;
    let allocation = prepared.allocation.ok_or_else(|| {
        error(
            "execution_allocation_missing",
            "retry admission through the version 2 host allocator",
            json!({"issue_number":issue_number}),
        )
    })?;
    let claim = journal.claim_allocated(
        scope,
        issue_number,
        allocation.execution_generation,
        &allocation.allocation_id,
        &prepared.request,
    )?;
    Ok(MissionAdmissionReceiptV2 {
        issue_number,
        base_commit: prepared.base_commit,
        requirement_path: prepared.requirement_path,
        requirement_sha256: prepared.requirement_sha256,
        request: prepared.request,
        allocation,
        claim,
    })
}

struct GenerationChoice {
    execution_generation: u64,
    allocation: Option<MissionGenerationAllocationV1>,
}

struct PreparedAdmission {
    base_commit: String,
    requirement_path: String,
    requirement_sha256: String,
    request: AgentExecutionRequestV1,
    allocation: Option<MissionGenerationAllocationV1>,
}

fn prepare_admission(
    scope: &BaseBranchScope,
    issue_number: u64,
    requirement_id: &str,
    requirement_revision: u64,
    source: &dyn MissionAdmissionSourceV1,
    context: ContextPacketV1,
    workspace_id: &str,
    capabilities: RuntimeCapabilitiesV1,
    expected_requirement_sha256: Option<&str>,
    required_schema_version: u32,
    arbitration: Option<RequirementArbitrationDecisionV1>,
    allocate_generation: impl FnOnce(&str) -> Result<GenerationChoice, AgentRuntimeErrorV1>,
) -> Result<PreparedAdmission, AgentRuntimeErrorV1> {
    let issue = source
        .read_issue(scope, issue_number)
        .map_err(source_error)?;
    let expected_url = format!(
        "{}/{}/{}/issues/{}",
        scope.origin(),
        scope.owner(),
        scope.repository(),
        issue_number
    );
    if issue.number != issue_number || issue.html_url != expected_url {
        return Err(error(
            "mission_source_mismatch",
            "re-read the issue from the configured Forgejo repository and verify its canonical URL",
            json!({"expected_number":issue_number,"actual_number":issue.number}),
        ));
    }
    if issue.state != MissionIssueStateV1::Open {
        return Err(error(
            "mission_issue_closed",
            "reopen the issue through its authorized workflow before starting a new execution",
            json!({"issue_number":issue_number}),
        ));
    }

    let branch = source
        .read_protected_branch_head(scope)
        .map_err(source_error)?;
    if branch.branch != scope.branch() || !branch.protected {
        return Err(error(
            "mission_base_unprotected",
            "select a protected base branch and confirm the Forgejo protection policy",
            json!({"expected_branch":scope.branch(),"actual_branch":branch.branch,"protected":branch.protected}),
        ));
    }
    if !valid_git_revision(&branch.commit) {
        return Err(error(
            "mission_base_invalid",
            "reload the protected branch head as a full Git commit ID",
            json!({"branch":branch.branch}),
        ));
    }

    let requirement_path = format!("requirements/{requirement_id}.json");
    let requirements_json = source
        .read_requirement_file_at(scope, &branch.commit, &requirement_path)
        .map_err(source_error)?;
    let requirements: Requirements =
        crate::strict_json::parse_value_rejecting_duplicate_members(&requirements_json)
            .and_then(serde_json::from_value)
            .map_err(|_| {
                error(
                    "mission_requirements_invalid",
                    "repair the protected requirements document before creating an execution",
                    json!({"path":requirement_path}),
                )
            })?;
    validate_unique_issue_binding(
        source,
        scope,
        &branch.commit,
        issue_number,
        &requirement_path,
        &requirements_json,
    )?;
    if requirements.schema_version != required_schema_version {
        return Err(error(
            "mission_requirement_binding_required",
            "select the requirements schema admitted by this host path and bind it to the source issue",
            json!({"path":requirement_path,"expected_schema_version":required_schema_version,"schema_version":requirements.schema_version}),
        ));
    }
    let source_issue_number = requirements.source_issue_number.ok_or_else(|| {
        error(
            "mission_requirement_binding_missing",
            "set source_issue_number in the protected requirement before admission",
            json!({"issue_number":issue_number,"path":requirement_path}),
        )
    })?;
    if source_issue_number != issue_number {
        return Err(error(
            "mission_requirement_issue_mismatch",
            "bind this protected requirement to the same issue that was re-read from Forgejo",
            json!({"issue_number":issue_number,"source_issue_number":source_issue_number,"path":requirement_path}),
        ));
    }
    if requirements.id != requirement_id
        || requirements.revision != requirement_revision
        || requirements.request.trim().is_empty()
        || requirements.acceptance.is_empty()
        || requirements
            .acceptance
            .iter()
            .chain(requirements.ambiguities.iter())
            .any(|item| item.trim().is_empty())
    {
        return Err(error(
            "mission_requirement_revision_mismatch",
            "select the requirement ID and revision present in the protected branch snapshot",
            json!({"expected_id":requirement_id,"expected_revision":requirement_revision,"actual_id":requirements.id,"actual_revision":requirements.revision}),
        ));
    }

    let requirement_sha256 = sha256(&requirements_json);
    if expected_requirement_sha256.is_some_and(|expected| expected != requirement_sha256) {
        return Err(error(
            "mission_requirement_hash_mismatch",
            "review the protected requirements change and update the host-pinned digest before admission",
            json!({"requirement_id":requirement_id,"requirement_revision":requirement_revision,"expected_sha256":expected_requirement_sha256,"actual_sha256":requirement_sha256}),
        ));
    }
    if !requirements.ambiguities.is_empty() {
        return Err(error(
            "mission_requirement_ambiguity_unresolved",
            "resolve every ambiguity through an authorized human decision in a new protected requirements revision, then retry admission",
            json!({"requirement_id":requirement_id,"requirement_revision":requirement_revision,"unresolved_ambiguity_count":requirements.ambiguities.len()}),
        ));
    }
    match required_schema_version {
        2 if requirements.source_pull_request_number.is_none()
            && requirements.decisions.is_empty()
            && arbitration.is_none() => {}
        3 => {
            let decision = arbitration.as_ref().ok_or_else(|| {
                error(
                    "mission_requirement_arbitration_missing",
                    "admit this protected schema-v3 revision through the host pull-request review gate",
                    json!({"requirement_id":requirement_id,"requirement_revision":requirement_revision}),
                )
            })?;
            if requirements.source_pull_request_number != Some(decision.source_pull_request_number)
                || decision.requirements_sha256 != requirement_sha256
                || requirements.decisions.is_empty()
                || requirements.decisions.iter().any(|item| {
                    item.ambiguity.trim().is_empty()
                        || item.resolution.trim().is_empty()
                        || item.rationale.trim().is_empty()
                })
            {
                return Err(error(
                    "mission_requirement_arbitration_mismatch",
                    "re-read the exact reviewed requirements bytes and their host-evaluated arbitration record",
                    json!({"requirement_id":requirement_id,"requirement_revision":requirement_revision}),
                ));
            }
        }
        _ => {
            return Err(error(
                "mission_requirement_schema_unsupported",
                "use the supported schema-v2 admission or the host-reviewed schema-v3 admission path",
                json!({"required_schema_version":required_schema_version}),
            ));
        }
    }

    // Bind admission to a branch snapshot that remained stable while the
    // protected requirement files and issue bindings were being resolved.
    // This detects an intervening advance or policy change before allocation.
    let latest_branch = source
        .read_protected_branch_head(scope)
        .map_err(source_error)?;
    if latest_branch != branch {
        return Err(error(
            "mission_base_changed_during_admission",
            "retry this delivery to bind a stable protected base before allocating the generation",
            json!({
                "expected_branch":branch.branch,
                "expected_commit":branch.commit,
                "expected_protected":branch.protected,
                "current_branch":latest_branch.branch,
                "current_commit":latest_branch.commit,
                "current_protected":latest_branch.protected
            }),
        ));
    }

    // Bracket requirement and context reads with issue snapshots. If the issue
    // changes while admission resolves protected inputs, refuse before creating
    // a generation or handing the request to the execution host.
    let latest_issue = source
        .read_issue(scope, issue_number)
        .map_err(source_error)?;
    if latest_issue != issue {
        return Err(error(
            "mission_issue_changed_during_admission",
            "retry this delivery to bind a stable current issue snapshot before allocating the generation",
            json!({"issue_number":issue_number}),
        ));
    }

    let untrusted_work_item = issue.work_item_content.filter(|content| {
        context
            .manifest
            .estimated_tokens
            .saturating_add(content.estimated_tokens())
            <= context.manifest.token_budget
    });
    let requirements_json = String::from_utf8(requirements_json).map_err(|_| {
        error(
            "mission_requirements_encoding_invalid",
            "store the protected requirements document as UTF-8 JSON",
            json!({"path":requirement_path}),
        )
    })?;
    // Run all request/context validations that do not depend on the generated
    // mission identity before the durable generation reservation.
    let mut request = AgentExecutionRequestV1 {
        schema_version: if untrusted_work_item
            .as_ref()
            .is_some_and(|content| content.schema_version == 3)
        {
            crate::agent_runtime::AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4
        } else if untrusted_work_item
            .as_ref()
            .is_some_and(|content| content.schema_version == 2)
        {
            crate::agent_runtime::AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3
        } else if required_schema_version == 3 {
            crate::agent_runtime::AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V2
        } else {
            crate::agent_runtime::AGENT_RUNTIME_SCHEMA_VERSION
        },
        mission_id: String::new(),
        idempotency_key: String::new(),
        requirement_sha256: requirement_sha256.clone(),
        requirements_json,
        untrusted_work_item,
        requirement_arbitration: arbitration,
        context,
        workspace: WorkspaceRefV1 {
            id: workspace_id.to_owned(),
            base_commit: branch.commit.clone(),
        },
        capabilities,
    };
    request.validate_before_allocation()?;

    let generation = allocate_generation(&requirement_sha256)?;
    let identity = derive_mission_identity_v1(
        scope,
        issue_number,
        &requirement_sha256,
        generation.execution_generation,
    )?;
    if generation.allocation.as_ref().is_some_and(|allocation| {
        allocation.mission_id != identity.mission_id
            || allocation.execution_generation != generation.execution_generation
            || allocation.requirement_sha256 != requirement_sha256
    }) {
        return Err(error(
            "execution_allocation_binding_mismatch",
            "re-read the issue and protected requirement before building the execution request",
            json!({"mission_id":identity.mission_id}),
        ));
    }
    request.mission_id = identity.mission_id;
    request.idempotency_key = identity.idempotency_key;
    request.validate()?;
    Ok(PreparedAdmission {
        base_commit: branch.commit,
        requirement_path,
        requirement_sha256,
        request,
        allocation: generation.allocation,
    })
}

const MAX_REQUIREMENT_DOCUMENTS: usize = 512;

fn validate_requirement_document_v3(
    requirements: &Requirements,
    binding: &MissionAdmissionBindingV3,
) -> Result<(), AgentRuntimeErrorV1> {
    let mut seen_decisions = HashSet::with_capacity(requirements.decisions.len());
    if requirements.schema_version != 3
        || requirements.id != binding.requirement_id
        || requirements.revision != binding.requirement_revision
        || requirements.source_issue_number != Some(binding.issue_number)
        || requirements
            .source_pull_request_number
            .is_none_or(|number| number == 0)
        || requirements.request.trim().is_empty()
        || requirements.acceptance.is_empty()
        || requirements
            .acceptance
            .iter()
            .any(|value| value.trim().is_empty())
        || !requirements.ambiguities.is_empty()
        || requirements.decisions.is_empty()
        || requirements.decisions.iter().any(|decision| {
            decision.ambiguity.trim().is_empty()
                || decision.resolution.trim().is_empty()
                || decision.rationale.trim().is_empty()
                || !seen_decisions.insert(decision.ambiguity.trim().to_owned())
        })
    {
        return Err(error(
            "mission_requirement_arbitration_invalid",
            "record version-3 requirements with resolved ambiguities, structured rationales, and a positive source PR number",
            json!({"requirement_id":requirements.id,"revision":requirements.revision,"source_pull_request_number":requirements.source_pull_request_number}),
        ));
    }
    Ok(())
}

fn validate_unique_issue_binding(
    source: &dyn MissionAdmissionSourceV1,
    scope: &BaseBranchScope,
    commit: &str,
    issue_number: u64,
    selected_path: &str,
    selected_json: &[u8],
) -> Result<(), AgentRuntimeErrorV1> {
    let paths = source
        .list_requirement_paths_at(scope, commit)
        .map_err(source_error)?;
    if paths.is_empty()
        || paths.len() > MAX_REQUIREMENT_DOCUMENTS
        || !paths.iter().any(|path| path == selected_path)
    {
        return Err(error(
            "mission_requirement_inventory_invalid",
            "re-read the complete protected requirements inventory, include the selected path, and keep it within the 512-document local bound",
            json!({"commit":commit,"path_count":paths.len(),"max_documents":MAX_REQUIREMENT_DOCUMENTS,"selected_path":selected_path}),
        ));
    }

    let mut seen_paths = std::collections::HashSet::new();
    for path in paths {
        let Some(requirement_id) = path
            .strip_prefix("requirements/")
            .and_then(|path| path.strip_suffix(".json"))
        else {
            return Err(error(
                "mission_requirement_inventory_invalid",
                "repair the protected requirements inventory so every entry is a canonical JSON document",
                json!({"commit":commit,"path":path}),
            ));
        };
        if !valid_requirement_id(requirement_id) || !seen_paths.insert(path.clone()) {
            return Err(error(
                "mission_requirement_inventory_invalid",
                "repair the protected requirements inventory so paths are safe and unique",
                json!({"commit":commit,"path":path}),
            ));
        }
        let bytes = if path == selected_path {
            selected_json.to_vec()
        } else {
            source
                .read_requirement_file_at(scope, commit, &path)
                .map_err(source_error)?
        };
        let requirement: Requirements =
            crate::strict_json::parse_value_rejecting_duplicate_members(&bytes)
                .and_then(serde_json::from_value)
                .map_err(|_| {
                    error(
                        "mission_requirement_inventory_invalid",
                        "repair every protected requirements JSON document before admission",
                        json!({"commit":commit,"path":path}),
                    )
                })?;
        let schema_fields_valid = match requirement.schema_version {
            1 => {
                requirement.source_issue_number.is_none()
                    && requirement.source_pull_request_number.is_none()
                    && requirement.decisions.is_empty()
            }
            2 => {
                requirement.source_pull_request_number.is_none() && requirement.decisions.is_empty()
            }
            3 => {
                requirement
                    .source_pull_request_number
                    .is_some_and(|number| number > 0)
                    && requirement.ambiguities.is_empty()
                    && !requirement.decisions.is_empty()
                    && requirement.decisions.iter().all(|item| {
                        !item.ambiguity.trim().is_empty()
                            && !item.resolution.trim().is_empty()
                            && !item.rationale.trim().is_empty()
                    })
            }
            _ => false,
        };
        if requirement.id != requirement_id
            || !schema_fields_valid
            || (requirement.schema_version == 1 && requirement.source_issue_number.is_some())
            || (path != selected_path
                && matches!(requirement.schema_version, 2 | 3)
                && requirement.source_issue_number.is_none())
            || requirement.source_issue_number == Some(0)
            || requirement.revision == 0
            || requirement.request.trim().is_empty()
            || requirement.acceptance.is_empty()
            || requirement
                .acceptance
                .iter()
                .chain(requirement.ambiguities.iter())
                .any(|item| item.trim().is_empty())
        {
            return Err(error(
                "mission_requirement_inventory_invalid",
                "align each protected requirement path and schema before admission",
                json!({"commit":commit,"path":path,"id":requirement.id,"schema_version":requirement.schema_version}),
            ));
        }
        if path != selected_path && requirement.source_issue_number == Some(issue_number) {
            return Err(error(
                "mission_requirement_issue_duplicate",
                "keep exactly one protected requirements document bound to this issue",
                json!({"issue_number":issue_number,"selected_path":selected_path,"duplicate_path":path,"commit":commit}),
            ));
        }
    }
    Ok(())
}

fn validate_binding_v2(binding: &MissionAdmissionBindingV2) -> Result<(), AgentRuntimeErrorV1> {
    if binding.schema_version != 2
        || binding.issue_number == 0
        || binding.requirement_revision == 0
        || !valid_requirement_id(&binding.requirement_id)
        || !valid_allocation_id(&binding.allocation_id)
    {
        return Err(error(
            "mission_binding_invalid",
            "provide a version 2 host binding with a safe requirement ID and stable allocation ID",
            json!({"schema_version":binding.schema_version,"issue_number":binding.issue_number,"requirement_revision":binding.requirement_revision}),
        ));
    }
    Ok(())
}

fn validate_binding_v3(binding: &MissionAdmissionBindingV3) -> Result<(), AgentRuntimeErrorV1> {
    if binding.schema_version != 3
        || binding.issue_number == 0
        || binding.requirement_revision == 0
        || !valid_requirement_id(&binding.requirement_id)
        || !valid_allocation_id(&binding.allocation_id)
    {
        return Err(error(
            "mission_binding_invalid",
            "provide a version 3 host binding with a safe requirement ID and stable allocation ID",
            json!({"schema_version":binding.schema_version,"issue_number":binding.issue_number,"requirement_revision":binding.requirement_revision}),
        ));
    }
    Ok(())
}

fn valid_requirement_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
        && value.as_bytes()[0].is_ascii_uppercase()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}

fn valid_allocation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_git_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn source_error(failure: SourceFailureV1) -> AgentRuntimeErrorV1 {
    let (code, retryable, action) = match failure {
        SourceFailureV1::NotFound => (
            "mission_source_not_found",
            false,
            "confirm that the issue and protected requirement exist in the configured repository",
        ),
        SourceFailureV1::Unauthorized => (
            "mission_source_unauthorized",
            false,
            "configure the host's read-only Forgejo identity",
        ),
        SourceFailureV1::Forbidden => (
            "mission_source_forbidden",
            false,
            "confirm read access and protected-path policy with the repository authority",
        ),
        SourceFailureV1::Unavailable => (
            "mission_source_unavailable",
            true,
            "retry after Forgejo or the configured source adapter is available",
        ),
        SourceFailureV1::InvalidResponse => (
            "mission_source_invalid_response",
            false,
            "inspect the Forgejo response and adapter version before retrying",
        ),
        SourceFailureV1::TreeTooLarge => (
            "mission_requirement_tree_limit",
            false,
            "reduce the repository tree below 20,000 entries and 256 pages, or raise the configured tree page size within the adapter bound",
        ),
    };
    AgentRuntimeErrorV1::new(code, retryable, action, json!({}))
}

fn error(
    code: impl Into<String>,
    action: impl Into<String>,
    details: serde_json::Value,
) -> AgentRuntimeErrorV1 {
    AgentRuntimeErrorV1::new(code, false, action, details)
}
