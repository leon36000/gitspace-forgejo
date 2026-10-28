//! Versioned boundary between the forge core and an independently supplied agent runtime.
//!
//! This module defines a contract only. It does not start an agent, provide a tool host,
//! or isolate a workspace. Implementations must honor `idempotency_key`; the host must
//! enforce the declared tool/path/budget capabilities and the operating-system boundary.

use crate::{
    ContextManifest, EventJournal, JournalError, JournalEvent, JournalTransaction,
    JournalTransactionError, Requirements,
    forgejo::{BaseBranchScope, MAX_FORGEJO_LABEL_NAME_BYTES},
    valid_mission_id_v1,
    webhook::reconcile::valid_forgejo_timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const AGENT_RUNTIME_SCHEMA_VERSION: u32 = 1;
pub const AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V2: u32 = 2;
pub const AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3: u32 = 3;
pub const AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4: u32 = 4;
const MAX_WORKSPACE_WRITE_CONTENT_BYTES: usize = 65_536;
const MAX_WORKSPACE_WRITE_RESPONSE_BYTES: usize = 16_384;
const MAX_JSONRPC_REQUEST_ID_BYTES: usize = 256;
pub const MAX_MISSION_EXECUTION_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

/// Stable mission and logical-execution keys derived from the reconciled issue,
/// a versioned requirement digest, and a host-owned generation.
///
/// Webhook delivery IDs are intentionally absent: Forgejo may issue a fresh ID
/// when replaying the same semantic event. The host must persist and allocate
/// `execution_generation`; this pure derivation does not claim or serialize work.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionIdentityV1 {
    pub schema_version: u32,
    pub mission_id: String,
    pub idempotency_key: String,
    pub execution_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionClaimResultV1 {
    Claimed(MissionIdentityV1),
    AlreadyClaimed(MissionIdentityV1),
}

/// Durable host-owned execution-generation allocation for one mission.
/// `replayed` is response metadata and is not part of the journal record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionGenerationAllocationV1 {
    pub schema_version: u32,
    pub mission_id: String,
    pub allocation_id: String,
    pub requirement_sha256: String,
    pub execution_generation: u64,
    pub replayed: bool,
}

/// Read-only confirmation that one exact host admission is still the current
/// durable allocation and claim for its mission generation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionExecutionClaimVerificationV1 {
    pub mission_id: String,
    pub allocation_id: String,
    pub execution_generation: u64,
    pub request_sha256: String,
}

/// Immutable, versioned inputs that identify one proposed merge. `scope_sha256`
/// binds the Forgejo origin, repository, and base branch without persisting a
/// second copy of their names in the execution journal.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MergeAttemptBindingV1 {
    pub schema_version: u32,
    pub scope_sha256: String,
    pub issue_number: u64,
    pub pull_request_number: u64,
    pub base_commit: String,
    pub candidate_commit: String,
    pub requirement_sha256: String,
    pub policy_sha256: String,
    pub proof_sha256: String,
    pub review_sha256: String,
    pub request_sha256: String,
    pub admission_sha256: String,
}

impl MergeAttemptBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: &BaseBranchScope,
        issue_number: u64,
        pull_request_number: u64,
        base_commit: &str,
        candidate_commit: &str,
        requirement_sha256: &str,
        policy_sha256: &str,
        proof_sha256: &str,
        review_sha256: &str,
        request_sha256: &str,
        admission_sha256: &str,
    ) -> Result<Self, AgentRuntimeErrorV1> {
        let binding = Self {
            schema_version: 1,
            scope_sha256: merge_scope_sha256(scope),
            issue_number,
            pull_request_number,
            base_commit: base_commit.to_ascii_lowercase(),
            candidate_commit: candidate_commit.to_ascii_lowercase(),
            requirement_sha256: requirement_sha256.to_ascii_lowercase(),
            policy_sha256: policy_sha256.to_ascii_lowercase(),
            proof_sha256: proof_sha256.to_ascii_lowercase(),
            review_sha256: review_sha256.to_ascii_lowercase(),
            request_sha256: request_sha256.to_ascii_lowercase(),
            admission_sha256: admission_sha256.to_ascii_lowercase(),
        };
        validate_merge_binding_shape(&binding)?;
        Ok(binding)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergeAttemptModeV1 {
    NewAttempt,
    RecoverOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergeAttemptOutcomeV1 {
    Merged,
    Rejected,
    Inconclusive,
}

/// Redacted, durable decision returned by the trusted host merge authority.
/// A `merged` result requires a merge commit, tree, and hash of the exact
/// authenticated readback; none of these fields alone proves software correct.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MergeAttemptResolutionV1 {
    pub schema_version: u32,
    pub outcome: MergeAttemptOutcomeV1,
    /// A nonterminal observation must be retried as authenticated readback;
    /// it never grants another merge POST.
    #[serde(default = "legacy_merge_resolution_terminal")]
    pub terminal: bool,
    /// `None` preserves replay of pre-field journal records; new host records
    /// persist the exact retryability that the CLI returned.
    #[serde(default)]
    pub retryable: Option<bool>,
    pub reason_code: Option<String>,
    pub merge_commit: Option<String>,
    pub merge_tree_manifest_sha256: Option<String>,
    pub observation_sha256: Option<String>,
}

impl MergeAttemptResolutionV1 {
    pub fn inconclusive(reason_code: &str) -> Self {
        Self {
            schema_version: 1,
            outcome: MergeAttemptOutcomeV1::Inconclusive,
            terminal: false,
            retryable: Some(true),
            reason_code: Some(reason_code.to_owned()),
            merge_commit: None,
            merge_tree_manifest_sha256: None,
            observation_sha256: None,
        }
    }

    pub fn rejected(reason_code: &str) -> Self {
        Self {
            schema_version: 1,
            outcome: MergeAttemptOutcomeV1::Rejected,
            terminal: true,
            retryable: Some(false),
            reason_code: Some(reason_code.to_owned()),
            merge_commit: None,
            merge_tree_manifest_sha256: None,
            observation_sha256: None,
        }
    }

    pub fn merged(
        merge_commit: &str,
        merge_tree_manifest_sha256: &str,
        observation_sha256: &str,
    ) -> Self {
        Self {
            schema_version: 1,
            outcome: MergeAttemptOutcomeV1::Merged,
            terminal: true,
            retryable: Some(false),
            reason_code: None,
            merge_commit: Some(merge_commit.to_ascii_lowercase()),
            merge_tree_manifest_sha256: Some(merge_tree_manifest_sha256.to_ascii_lowercase()),
            observation_sha256: Some(observation_sha256.to_ascii_lowercase()),
        }
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = Some(retryable);
        self
    }

    pub fn is_terminal(&self) -> bool {
        self.terminal && !matches!(self.outcome, MergeAttemptOutcomeV1::Inconclusive)
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
            .unwrap_or(matches!(self.outcome, MergeAttemptOutcomeV1::Inconclusive))
    }
}

fn legacy_merge_resolution_terminal() -> bool {
    true
}

/// Durable host-owned lease for one already-claimed execution generation.
///
/// `fencing_token` increases across the mission whenever a lease is replaced or
/// a newer generation starts. A host tool port must validate both generation
/// and token before accepting work; this DTO does not stop or isolate a process.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionExecutionLeaseV1 {
    pub schema_version: u32,
    pub mission_id: String,
    pub execution_generation: u64,
    pub allocation_id: String,
    pub lease_id: String,
    pub holder_id: String,
    pub requirement_sha256: String,
    pub request_sha256: String,
    /// Runtime timeout captured from the exact claimed request.
    pub runtime_timeout_ms: u64,
    pub fencing_token: u64,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub ttl_ms: u64,
    /// Response metadata; it is not persisted in the journal event.
    pub replayed: bool,
}

/// Durable reservation for a verification operation charged to the mission's
/// cumulative CPU budget. A reservation is recorded before the operation is
/// started. If the host stops before resolving it, the full reservation remains
/// charged when the journal is replayed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionVerificationCpuReservationV1 {
    pub mission_id: String,
    pub reservation_id: String,
    pub policy_sha256: String,
    pub mission_cpu_budget_ms: u64,
    pub reserved_cpu_ms: u64,
    pub charged_cpu_ms: u64,
    pub remaining_cpu_ms: u64,
    /// Response metadata. Replaying an operation ID is refused rather than
    /// returning a reservation that could accidentally run the effect twice.
    pub replayed: bool,
}

/// Read-only view of one durable verifier CPU reservation. An unresolved
/// reservation is reported at its full reserved amount because the host cannot
/// prove how much work ran before interruption.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionVerificationCpuReservationSummaryV1 {
    pub reservation_id: String,
    pub reserved_cpu_ms: u64,
    pub charged_cpu_ms: u64,
    pub resolved: bool,
}

/// Validated cumulative CPU ledger for one admitted mission identity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionVerificationCpuLedgerSummaryV1 {
    pub mission_id: String,
    pub policy_sha256: String,
    pub mission_cpu_budget_ms: u64,
    pub charged_cpu_ms: u64,
    pub remaining_cpu_ms: u64,
    pub reservations: Vec<MissionVerificationCpuReservationSummaryV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MissionVerificationCpuReservationRecordV1 {
    schema_version: u32,
    mission_id: String,
    execution_generation: u64,
    lease_id: String,
    holder_id: String,
    fencing_token: u64,
    request_sha256: String,
    reservation_id: String,
    policy_sha256: String,
    mission_cpu_budget_ms: u64,
    reserved_cpu_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MissionVerificationCpuResolutionRecordV1 {
    schema_version: u32,
    mission_id: String,
    execution_generation: u64,
    lease_id: String,
    fencing_token: u64,
    reservation_id: String,
    policy_sha256: String,
    charged_cpu_ms: u64,
}

/// Host-authored durable intent for the one bounded workspace write exposed by
/// the local MISSION-012 adapter. The input is retained so a later host can
/// reconcile the exact requested bytes after a process interruption.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceWriteIntentV1 {
    pub schema_version: u32,
    pub mission_id: String,
    pub execution_generation: u64,
    pub operation_id: String,
    pub request_id: Value,
    pub allocation_id: String,
    pub request_sha256: String,
    pub lease_id: String,
    pub holder_id: String,
    pub fencing_token: u64,
    pub tool_id: String,
    pub input: Value,
    pub input_sha256: String,
    pub path: String,
    pub before_source_sha256: String,
    pub after_source_sha256: String,
    /// Response metadata; it is not part of the journal record.
    #[serde(skip)]
    pub replayed: bool,
}

/// Durable exact MCP result body for a completed workspace write. The JSON-RPC
/// correlation ID remains separate so an identical call replayed with a new
/// transport ID can return the saved result without repeating its effect. The
/// resolving lease may be a newer fence in the same execution generation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceWriteResultV1 {
    pub schema_version: u32,
    pub mission_id: String,
    pub execution_generation: u64,
    pub operation_id: String,
    pub request_id: Value,
    pub allocation_id: String,
    pub request_sha256: String,
    pub input_sha256: String,
    pub before_source_sha256: String,
    pub observed_source_sha256: String,
    pub lease_id: String,
    pub holder_id: String,
    pub fencing_token: u64,
    pub result_body: Value,
    pub result_sha256: String,
    /// Response metadata; it is not part of the journal record.
    #[serde(skip)]
    pub replayed: bool,
}

/// Read-only status for an operation key, validated against both the complete
/// mission journal and the caller's current lease. It lets the host return a
/// stored response without repeating the filesystem effect.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceWriteOperationStatusV1 {
    pub intent: WorkspaceWriteIntentV1,
    pub result: Option<WorkspaceWriteResultV1>,
}

/// Result returned by the trusted host after one serialized workspace write.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceWriteExecutionResultV1 {
    pub operation_id: String,
    pub request_id: Value,
    pub result_body: Value,
    pub replayed: bool,
}

/// Narrow durable admission and lease journal backed by the existing hash-chained
/// inbox journal. It does not run the agent, isolate its process, or terminate a
/// worker when its lease expires.
pub struct MissionExecutionJournalV1 {
    journal: EventJournal,
}

impl MissionExecutionJournalV1 {
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            journal: EventJournal::open_with_max_bytes(path, MAX_MISSION_EXECUTION_JOURNAL_BYTES),
        }
    }

    /// Allocate a monotonic generation with a stable host request ID.
    ///
    /// Reading the generation high-water mark and reserving the next slot happen
    /// under one journal lock. Repeating the same `allocation_id` after a restart
    /// returns the original allocation. This is an allocator only: it does not
    /// grant an execution lease, prove that work started, or release an
    /// allocation after a crash.
    pub fn allocate_generation(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        requirement_sha256: &str,
        allocation_id: &str,
    ) -> Result<MissionGenerationAllocationV1, AgentRuntimeErrorV1> {
        if !valid_allocation_id(allocation_id) {
            return Err(error(
                "execution_allocation_id_invalid",
                "provide a stable host-owned allocation ID using 1–128 ASCII letters, digits, hyphens, or underscores",
                json!({}),
            ));
        }
        let mission = derive_mission_identity_v1(scope, issue_number, requirement_sha256, 1)?;

        self.journal
            .transact_mission(&mission.mission_id, |events| {
                // Validate the complete mission ledger before replaying an
                // earlier allocation, so a malformed later record cannot be
                // hidden by an idempotent retry.
                validate_mission_journal(events)?;
                let highest = highest_known_generation(events, &mission.mission_id)?;
                if let Some(existing) =
                    allocation_for_request(events, &mission.mission_id, allocation_id)?
                {
                    if existing.requirement_sha256 != requirement_sha256.to_ascii_lowercase() {
                        return Err(error(
                            "execution_allocation_binding_mismatch",
                            "use the same issue and protected requirement hash for an allocation ID",
                            json!({"mission_id":mission.mission_id}),
                        ));
                    }
                    return Ok(JournalTransaction::Return(
                        MissionGenerationAllocationV1 {
                            replayed: true,
                            ..existing
                        },
                    ));
                }

                if unresolved_merge_attempt_for_mission(events, &mission.mission_id)?.is_some() {
                    return Err(error(
                        "execution_merge_attempt_unresolved",
                        "recover the existing merge intent through authenticated readback before allocating another execution generation",
                        json!({"mission_id":mission.mission_id}),
                    ));
                }

                let execution_generation = highest.checked_add(1).ok_or_else(|| {
                    error(
                        "execution_generation_exhausted",
                        "start a new mission identity or repair the generation ledger",
                        json!({"mission_id":mission.mission_id}),
                    )
                })?;
                let allocation = MissionGenerationAllocationV1 {
                    schema_version: 1,
                    mission_id: mission.mission_id.clone(),
                    allocation_id: allocation_id.to_owned(),
                    requirement_sha256: requirement_sha256.to_ascii_lowercase(),
                    execution_generation,
                    replayed: false,
                };
                Ok(JournalTransaction::Append {
                    idempotency_key: format!(
                        "mission-generation-v1:{}:{execution_generation}",
                        mission.mission_id
                    ),
                    kind: "mission_generation_allocated_v1".to_owned(),
                    payload: json!({
                        "schema_version": 1,
                        "mission_id": mission.mission_id,
                        "allocation_id": allocation_id,
                        "requirement_sha256": requirement_sha256.to_ascii_lowercase(),
                        "execution_generation": execution_generation,
                    }),
                    result: allocation,
                })
            })
            .map_err(map_journal_transaction_error)
    }

    /// Claim a generation previously reserved by `allocate_generation`.
    /// The allocation ID is required so a stale or unrelated caller cannot
    /// consume another admission's reserved slot. This is the only public API
    /// that creates a new claim. It is still a trusted-host API: the request's
    /// self-reported contents do not prove Forgejo provenance. A matching
    /// prior claim stays idempotent after a newer allocation, but cannot obtain
    /// a lease because lease acquisition enforces the current generation.
    pub fn claim_allocated(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        execution_generation: u64,
        allocation_id: &str,
        request: &AgentExecutionRequestV1,
    ) -> Result<ExecutionClaimResultV1, AgentRuntimeErrorV1> {
        if !valid_allocation_id(allocation_id) {
            return Err(error(
                "execution_allocation_id_invalid",
                "provide the stable host-owned allocation ID that reserved this generation",
                json!({}),
            ));
        }
        self.claim_inner(
            scope,
            issue_number,
            execution_generation,
            allocation_id,
            request,
        )
    }

    /// Acquire an expiring lease for a V2 allocation that already has its exact
    /// execution claim. The trusted host supplies time and IDs; an agent must
    /// never receive this API or choose its own holder, lease ID, or clock.
    /// A retry with the same `lease_id` returns the original lease while its
    /// generation is current. If a newer generation has been allocated, the
    /// generation fence returns `execution_lease_generation_superseded` before
    /// lease replay. The first lease starts one absolute runtime deadline; later
    /// leases cannot extend it. As with claim, the public Rust method does not
    /// authenticate requirement provenance.
    pub fn acquire_lease(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        execution_generation: u64,
        allocation_id: &str,
        lease_id: &str,
        holder_id: &str,
        clock: impl FnOnce() -> u64,
        ttl_ms: u64,
        request: &AgentExecutionRequestV1,
    ) -> Result<MissionExecutionLeaseV1, AgentRuntimeErrorV1> {
        request.validate_for_issue_number(issue_number)?;
        if !valid_allocation_id(allocation_id)
            || !valid_allocation_id(lease_id)
            || !valid_allocation_id(holder_id)
        {
            return Err(error(
                "execution_lease_identity_invalid",
                "use host-owned allocation, lease, and holder IDs containing 1–128 ASCII letters, digits, hyphens, or underscores",
                json!({}),
            ));
        }
        if ttl_ms == 0 || ttl_ms > request.capabilities.budget.timeout_ms {
            return Err(error(
                "execution_lease_ttl_invalid",
                "choose a positive lease no longer than the execution timeout",
                json!({
                    "ttl_ms":ttl_ms,
                    "timeout_ms":request.capabilities.budget.timeout_ms,
                }),
            ));
        }
        let identity = derive_mission_identity_v1(
            scope,
            issue_number,
            &request.requirement_sha256,
            execution_generation,
        )?;
        if request.mission_id != identity.mission_id
            || request.idempotency_key != identity.idempotency_key
        {
            return Err(error(
                "execution_identity_mismatch",
                "rebuild the request from this issue, protected requirement revision, and host-assigned generation",
                json!({"mission_id":identity.mission_id}),
            ));
        }
        let request_sha256 = sha256(&serde_json::to_vec(request).map_err(|_| {
            error(
                "execution_request_serialization_failed",
                "rebuild the versioned execution request before leasing it",
                json!({}),
            )
        })?);
        let mission_id = identity.mission_id;

        self.journal
            .transact_mission(&mission_id, |events| {
                validate_mission_journal(events)?;
                // Sample only after both the per-mission and global journal
                // locks have been acquired; callers cannot carry a stale time
                // across lock contention or scheduler suspension.
                let now_unix_ms = clock();
                let highest_generation = highest_known_generation(events, &mission_id)?;
                if execution_generation < highest_generation {
                    return Err(error(
                        "execution_lease_generation_superseded",
                        "discard this lease request and use the latest host-allocated generation",
                        json!({
                            "mission_id":mission_id,
                            "requested_generation":execution_generation,
                            "current_generation":highest_generation,
                        }),
                    ));
                }
                if execution_generation > highest_generation {
                    return Err(error(
                        "execution_allocation_missing",
                        "allocate this generation with a stable host-owned ID before leasing it",
                        json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                    ));
                }
                let allocation = allocation_for_generation(
                    events,
                    &mission_id,
                    execution_generation,
                )?
                .ok_or_else(|| {
                    error(
                        "execution_allocation_missing",
                        "allocate this generation with a stable host-owned ID before leasing it",
                        json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                    )
                })?;
                if allocation.allocation_id != allocation_id
                    || allocation.requirement_sha256 != request.requirement_sha256
                {
                    return Err(error(
                        "execution_allocation_binding_mismatch",
                        "use the allocation ID and protected requirement hash reserved for this generation",
                        json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                    ));
                }
                let claimed_request_sha256 = claim_request_sha256(
                    events,
                    &mission_id,
                    execution_generation,
                )?
                .ok_or_else(|| {
                    error(
                        "execution_claim_missing",
                        "persist the exact V2 execution claim before starting a worker",
                        json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                    )
                })?;
                if claimed_request_sha256 != request_sha256 {
                    return Err(error(
                        "execution_claim_binding_mismatch",
                        "rebuild the exact request recorded by the execution claim before leasing it",
                        json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                    ));
                }
                let mission_history = execution_lease_history(events, &mission_id)?;
                let history = mission_history
                    .iter()
                    .filter(|lease| lease.execution_generation == execution_generation)
                    .cloned()
                    .collect::<Vec<_>>();
                validate_lease_history_binding(
                    &history,
                    allocation_id,
                    &request.requirement_sha256,
                    &request_sha256,
                    request.capabilities.budget.timeout_ms,
                )?;
                if let Some(existing) = history.iter().find(|lease| lease.lease_id == lease_id) {
                    if existing.allocation_id != allocation_id
                        || existing.holder_id != holder_id
                        || existing.requirement_sha256 != request.requirement_sha256
                        || existing.request_sha256 != request_sha256
                        || existing.ttl_ms != ttl_ms
                    {
                        return Err(error(
                            "execution_lease_idempotency_conflict",
                            "keep the original lease request or choose a new host-owned lease ID",
                            json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                        ));
                    }
                    return Ok(JournalTransaction::Return(MissionExecutionLeaseV1 {
                        replayed: true,
                        ..existing.clone()
                    }));
                }
                if let Some(current) = history.last() {
                    if now_unix_ms < current.issued_at_unix_ms {
                        return Err(error(
                            "execution_lease_clock_regressed",
                            "confirm the host clock before acquiring or recovering a lease",
                            json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                        ));
                    }
                    if current.expires_at_unix_ms > now_unix_ms {
                        return Err(AgentRuntimeErrorV1::new(
                            "execution_lease_active",
                            true,
                            "wait until the current lease expires; if the host stops its worker, keep the reservation until expiry",
                            json!({
                                "mission_id":mission_id,
                                "execution_generation":execution_generation,
                                "fencing_token":current.fencing_token,
                                "expires_at_unix_ms":current.expires_at_unix_ms,
                            }),
                        ));
                    }
                }
                let runtime_deadline_unix_ms = if let Some(first) = history.first() {
                    if now_unix_ms < first.issued_at_unix_ms {
                        return Err(error(
                            "execution_lease_clock_regressed",
                            "confirm the host clock before acquiring or recovering a lease",
                            json!({"mission_id":mission_id,"execution_generation":execution_generation}),
                        ));
                    }
                    first
                        .issued_at_unix_ms
                        .checked_add(first.runtime_timeout_ms)
                        .ok_or_else(|| {
                            error(
                                "execution_lease_time_overflow",
                                "choose a smaller runtime timeout or correct the host clock",
                                json!({"mission_id":mission_id}),
                            )
                        })?
                } else {
                    now_unix_ms
                        .checked_add(request.capabilities.budget.timeout_ms)
                        .ok_or_else(|| {
                            error(
                                "execution_lease_time_overflow",
                                "choose a smaller runtime timeout or correct the host clock",
                                json!({"mission_id":mission_id}),
                            )
                        })?
                };
                if now_unix_ms >= runtime_deadline_unix_ms {
                    return Err(error(
                        "execution_lease_deadline_exceeded",
                        "the absolute runtime deadline has passed; start a new host-approved generation",
                        json!({
                            "mission_id":mission_id,
                            "execution_generation":execution_generation,
                            "runtime_deadline_unix_ms":runtime_deadline_unix_ms,
                        }),
                    ));
                }
                let expires_at_unix_ms = now_unix_ms.checked_add(ttl_ms).ok_or_else(|| {
                    error(
                        "execution_lease_time_overflow",
                        "choose a smaller lease duration or correct the host clock",
                        json!({}),
                    )
                })?;
                if expires_at_unix_ms > runtime_deadline_unix_ms {
                    return Err(error(
                        "execution_lease_deadline_exceeded",
                        "choose a lease that ends no later than the generation's absolute runtime deadline",
                        json!({
                            "mission_id":mission_id,
                            "execution_generation":execution_generation,
                            "runtime_deadline_unix_ms":runtime_deadline_unix_ms,
                        }),
                    ));
                }
                let fencing_token = mission_history
                    .last()
                    .map(|lease| lease.fencing_token)
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| {
                        error(
                            "execution_fencing_token_exhausted",
                            "start a new mission identity or repair the lease journal",
                            json!({"mission_id":mission_id}),
                        )
                    })?;
                let lease = MissionExecutionLeaseV1 {
                    schema_version: 1,
                    mission_id: mission_id.clone(),
                    execution_generation,
                    allocation_id: allocation_id.to_owned(),
                    lease_id: lease_id.to_owned(),
                    holder_id: holder_id.to_owned(),
                    requirement_sha256: request.requirement_sha256.clone(),
                    request_sha256: request_sha256.clone(),
                    runtime_timeout_ms: request.capabilities.budget.timeout_ms,
                    fencing_token,
                    issued_at_unix_ms: now_unix_ms,
                    expires_at_unix_ms,
                    ttl_ms,
                    replayed: false,
                };
                Ok(JournalTransaction::Append {
                    idempotency_key: lease_idempotency_key(
                        &mission_id,
                        execution_generation,
                        lease_id,
                    ),
                    kind: "mission_execution_lease_acquired_v1".to_owned(),
                    payload: execution_lease_payload(&lease),
                    result: lease,
                })
            })
            .map_err(map_journal_transaction_error)
    }

    /// Load a host-issued lease by its stable ID and validate it against the
    /// current generation, holder, issue scope, and expiry. This is a read-only
    /// lookup; any subsequent effect must still call a journal operation that
    /// revalidates the lease while holding its mission lock.
    pub fn current_lease_by_id(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease_id: &str,
        expected_holder_id: &str,
        now_unix_ms: u64,
    ) -> Result<MissionExecutionLeaseV1, AgentRuntimeErrorV1> {
        if !valid_allocation_id(lease_id) || !valid_allocation_id(expected_holder_id) {
            return Err(error(
                "execution_lease_identity_invalid",
                "use the exact host-owned lease and holder IDs from the local execution profile",
                json!({}),
            ));
        }
        let events = self.journal.try_replay().map_err(journal_error)?;
        validate_mission_journal(&events)?;
        let mut matches = events
            .iter()
            .filter(|event| event.kind == "mission_execution_lease_acquired_v1")
            .map(parse_execution_lease)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|lease| lease.lease_id == lease_id);
        let lease = matches.next().ok_or_else(|| {
            error(
                "execution_lease_missing",
                "acquire a lease through the trusted host before reserving verifier work",
                json!({}),
            )
        })?;
        if matches.next().is_some() {
            return Err(error(
                "execution_lease_id_ambiguous",
                "use a host-owned lease ID that identifies exactly one durable lease",
                json!({}),
            ));
        }
        validate_lease_against_events(
            &events,
            scope,
            issue_number,
            &lease,
            expected_holder_id,
            now_unix_ms,
        )?;
        Ok(lease)
    }

    /// Persist the host-derived intent for a bounded `workspace.write_text`
    /// call before the caller performs its filesystem effect. The JSON-RPC ID
    /// is a correlation value, not an authority: the operation key is derived
    /// by this host API from the mission generation, canonical input hash, and
    /// candidate pre-state; the JSON-RPC ID is retained only as correlation.
    /// On replay, `observed_source_sha256` must still equal the recorded
    /// pre-state or post-state; an unrelated third state is inconclusive.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_workspace_write_intent(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease: &MissionExecutionLeaseV1,
        expected_holder_id: &str,
        now_unix_ms: u64,
        request_id: Value,
        input: Value,
        observed_source_sha256: &str,
    ) -> Result<WorkspaceWriteIntentV1, AgentRuntimeErrorV1> {
        validate_jsonrpc_request_id(&request_id)?;
        let (path, input_sha256, after_source_sha256) = validate_workspace_write_input(&input)?;
        let observed_source_sha256 = observed_source_sha256.to_ascii_lowercase();
        if !valid_sha256(&observed_source_sha256) {
            return Err(error(
                "tool_operation_source_hash_invalid",
                "read the trusted candidate source and provide its SHA-256 before writing",
                json!({"path":path}),
            ));
        }
        let operation_id = workspace_write_operation_id(
            &lease.mission_id,
            lease.execution_generation,
            &input_sha256,
            &observed_source_sha256,
        )?;

        let requested = WorkspaceWriteIntentV1 {
            schema_version: 1,
            mission_id: lease.mission_id.clone(),
            execution_generation: lease.execution_generation,
            operation_id: operation_id.clone(),
            request_id,
            allocation_id: lease.allocation_id.clone(),
            request_sha256: lease.request_sha256.clone(),
            lease_id: lease.lease_id.clone(),
            holder_id: lease.holder_id.clone(),
            fencing_token: lease.fencing_token,
            tool_id: "workspace.write_text".to_owned(),
            input,
            input_sha256,
            path,
            before_source_sha256: observed_source_sha256.clone(),
            after_source_sha256,
            replayed: false,
        };

        self.journal
            .with_mission_guard(&lease.mission_id, |guard| {
                guard.transact(|events| {
                    validate_mission_journal(events)?;
                    validate_lease_against_events(
                        events,
                        scope,
                        issue_number,
                        lease,
                        expected_holder_id,
                        now_unix_ms,
                    )?;
                    let ledger = workspace_write_operation_ledger(events)?;
                    let replay_intents = ledger.intents.values().filter(|existing| {
                        existing.mission_id == requested.mission_id
                            && existing.execution_generation == requested.execution_generation
                            && existing.input_sha256 == requested.input_sha256
                            && existing.input == requested.input
                            && (observed_source_sha256 == existing.before_source_sha256
                                || observed_source_sha256 == existing.after_source_sha256)
                    }).collect::<Vec<_>>();
                    if replay_intents.len() > 1 {
                        return Err(error(
                            "tool_operation_recovery_ambiguous",
                            "inspect the candidate and durable write history before replaying an operation with multiple matching source states",
                            json!({"mission_id":lease.mission_id}),
                        ));
                    }
                    let selected = replay_intents.first().copied();
                    validate_unresolved_workspace_write(
                        &ledger,
                        &requested.mission_id,
                        requested.execution_generation,
                        &requested.input,
                        &requested.request_id,
                        selected.map(|existing| existing.operation_id.as_str()),
                        &observed_source_sha256,
                    )?;
                    let selected_operation_id = selected
                        .map(|existing| existing.operation_id.as_str())
                        .unwrap_or(operation_id.as_str());
                    if ledger.intents.values().any(|existing| {
                        existing.mission_id == requested.mission_id
                            && existing.execution_generation == requested.execution_generation
                            && existing.request_id == requested.request_id
                            && existing.operation_id != selected_operation_id
                    }) {
                        return Err(error(
                            "tool_operation_request_id_conflict",
                            "use a new JSON-RPC correlation ID for a different workspace-write input",
                            json!({"mission_id":lease.mission_id}),
                        ));
                    }
                    if let Some(existing) = selected {
                        if existing.input != requested.input
                            || existing.input_sha256 != requested.input_sha256
                            || existing.allocation_id != requested.allocation_id
                            || existing.request_sha256 != requested.request_sha256
                        {
                            return Err(error(
                                "tool_operation_idempotency_conflict",
                                "reuse the exact write input for this host-derived operation key, or start a separately admitted execution generation",
                                json!({"mission_id":lease.mission_id,"operation_id":operation_id}),
                            ));
                        }
                        return Ok(JournalTransaction::Return(WorkspaceWriteIntentV1 {
                            replayed: true,
                            ..existing.clone()
                        }));
                    }

                    if ledger.intents.contains_key(&operation_id) {
                        return Err(invalid_tool_operation_record(&lease.mission_id));
                    }
                    let payload = serde_json::to_value(&requested).map_err(|_| {
                        invalid_tool_operation_record(&lease.mission_id)
                    })?;
                    Ok(JournalTransaction::Append {
                        idempotency_key: operation_id.clone(),
                        kind: "mission_tool_operation_intent_v1".to_owned(),
                        payload,
                        result: requested,
                    })
                })
                .map_err(map_journal_transaction_error)
            })
            .map_err(map_journal_transaction_error)
    }

    /// Persist the exact successful MCP result body after the trusted host has
    /// read back the candidate file and confirmed the declared post-state.
    /// Completion is idempotent and may be performed by a newer lease in the
    /// same execution generation after an interrupted write is reconciled.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_workspace_write_result(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease: &MissionExecutionLeaseV1,
        expected_holder_id: &str,
        now_unix_ms: u64,
        operation_id: &str,
        observed_source_sha256: &str,
        result_body: Value,
    ) -> Result<WorkspaceWriteResultV1, AgentRuntimeErrorV1> {
        let observed_source_sha256 = observed_source_sha256.to_ascii_lowercase();
        if !valid_sha256(&observed_source_sha256) {
            return Err(error(
                "tool_operation_source_hash_invalid",
                "read the trusted candidate source before recording the write result",
                json!({"operation_id":operation_id}),
            ));
        }
        let result_bytes = serde_json::to_vec(&result_body).map_err(|_| {
            error(
                "tool_operation_response_invalid",
                "rebuild the bounded successful MCP result body before recording the write result",
                json!({"operation_id":operation_id}),
            )
        })?;
        if result_bytes.len() > MAX_WORKSPACE_WRITE_RESPONSE_BYTES {
            return Err(error(
                "tool_operation_response_too_large",
                "keep the stored workspace-write response within the host's 16 KiB bound",
                json!({"max_bytes":MAX_WORKSPACE_WRITE_RESPONSE_BYTES}),
            ));
        }

        self.journal
            .with_mission_guard(&lease.mission_id, |guard| {
                guard.transact(|events| {
                    validate_mission_journal(events)?;
                    validate_lease_against_events(
                        events,
                        scope,
                        issue_number,
                        lease,
                        expected_holder_id,
                        now_unix_ms,
                    )?;
                    let ledger = workspace_write_operation_ledger(events)?;
                    let intent = ledger.intents.get(operation_id).ok_or_else(|| {
                        error(
                            "tool_operation_intent_missing",
                            "persist the exact authorized workspace-write intent before recording a result",
                            json!({"mission_id":lease.mission_id,"operation_id":operation_id}),
                        )
                    })?;
                    if intent.execution_generation != lease.execution_generation
                        || intent.allocation_id != lease.allocation_id
                        || intent.request_sha256 != lease.request_sha256
                    {
                        return Err(error(
                            "tool_operation_lease_binding_mismatch",
                            "resolve a write only within its admitted execution generation and request",
                            json!({"mission_id":lease.mission_id,"operation_id":operation_id}),
                        ));
                    }
                    if observed_source_sha256 != intent.after_source_sha256 {
                        return Err(error(
                            "tool_operation_effect_not_observed",
                            "read back the exact declared post-write bytes; preserve the intent and refuse success while the candidate differs",
                            json!({
                                "mission_id":lease.mission_id,
                                "operation_id":operation_id,
                                "expected_after_sha256":intent.after_source_sha256,
                                "observed_sha256":observed_source_sha256,
                            }),
                        ));
                    }
                    if !valid_mcp_success_result_body(&result_body) {
                        return Err(error(
                            "tool_operation_response_invalid",
                            "store the exact successful MCP result body with isError=false",
                            json!({"operation_id":operation_id}),
                        ));
                    }

                    if let Some(existing) = ledger.results.get(operation_id) {
                        if existing.result_body != result_body
                            || existing.observed_source_sha256 != observed_source_sha256
                        {
                            return Err(error(
                                "tool_operation_result_conflict",
                                "replay the original successful MCP result bytes for this completed operation",
                                json!({"mission_id":lease.mission_id,"operation_id":operation_id}),
                            ));
                        }
                        return Ok(JournalTransaction::Return(WorkspaceWriteResultV1 {
                            replayed: true,
                            ..existing.clone()
                        }));
                    }

                    if lease.fencing_token < intent.fencing_token {
                        return Err(error(
                            "tool_operation_lease_fenced",
                            "complete this operation only under its original or a newer current lease fence",
                            json!({"mission_id":lease.mission_id,"operation_id":operation_id}),
                        ));
                    }
                    let result = WorkspaceWriteResultV1 {
                        schema_version: 1,
                        mission_id: lease.mission_id.clone(),
                        execution_generation: lease.execution_generation,
                        operation_id: operation_id.to_owned(),
                        request_id: intent.request_id.clone(),
                        allocation_id: lease.allocation_id.clone(),
                        request_sha256: lease.request_sha256.clone(),
                        input_sha256: intent.input_sha256.clone(),
                        before_source_sha256: intent.before_source_sha256.clone(),
                        observed_source_sha256: observed_source_sha256.clone(),
                        lease_id: lease.lease_id.clone(),
                        holder_id: lease.holder_id.clone(),
                        fencing_token: lease.fencing_token,
                        result_sha256: sha256(&result_bytes),
                        result_body,
                        replayed: false,
                    };
                    let payload = serde_json::to_value(&result)
                        .map_err(|_| invalid_tool_operation_record(&lease.mission_id))?;
                    Ok(JournalTransaction::Append {
                        idempotency_key: workspace_write_result_key(operation_id),
                        kind: "mission_tool_operation_result_v1".to_owned(),
                        payload,
                        result,
                    })
                })
                .map_err(map_journal_transaction_error)
            })
            .map_err(map_journal_transaction_error)
    }

    /// Hold the mission fence across intent append, the bounded filesystem
    /// effect, readback, and result append. `read_source_sha256` must inspect
    /// the fixed host-owned candidate path; `apply_write` must replace it only
    /// when its current digest equals the supplied pre-state digest. An
    /// interrupted effect leaves an intent for a newer lease to reconcile.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_workspace_write<Clock, ReadSource, ApplyWrite>(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease: &MissionExecutionLeaseV1,
        expected_holder_id: &str,
        mut clock: Clock,
        request_id: Value,
        input: Value,
        mut read_source_sha256: ReadSource,
        mut apply_write: ApplyWrite,
    ) -> Result<WorkspaceWriteExecutionResultV1, AgentRuntimeErrorV1>
    where
        Clock: FnMut() -> u64,
        ReadSource: FnMut() -> Result<String, AgentRuntimeErrorV1>,
        ApplyWrite: FnMut(&str, &str) -> Result<(), AgentRuntimeErrorV1>,
    {
        validate_jsonrpc_request_id(&request_id)?;
        let (path, input_sha256, after_source_sha256) = validate_workspace_write_input(&input)?;
        let content = input["content"]
            .as_str()
            .ok_or_else(invalid_workspace_write_input)?;
        let content = content.to_owned();

        self.journal
            .with_mission_guard(&lease.mission_id, |guard| {
                let mut last_clock = None;
                let mut sample_clock = || -> Result<u64, AgentRuntimeErrorV1> {
                    let now = clock();
                    if last_clock.is_some_and(|previous| now < previous) {
                        return Err(error(
                            "execution_lease_clock_regressed",
                            "confirm the host clock before continuing the journaled workspace write",
                            json!({"mission_id":lease.mission_id}),
                        ));
                    }
                    last_clock = Some(now);
                    Ok(now)
                };

                guard
                    .transact(|events| {
                        validate_mission_journal(events)?;
                        validate_lease_against_events(
                            events,
                            scope,
                            issue_number,
                            lease,
                            expected_holder_id,
                            sample_clock()?,
                        )?;
                        Ok(JournalTransaction::Return(()))
                    })
                    .map_err(map_journal_transaction_error)?;

                let observed_before = read_source_sha256()?.to_ascii_lowercase();
                if !valid_sha256(&observed_before) {
                    return Err(error(
                        "tool_operation_source_hash_invalid",
                        "read the regular candidate source and return its SHA-256 before writing",
                        json!({"path":path}),
                    ));
                }

                let (intent, existing_result) = guard
                    .transact(|events| {
                        validate_mission_journal(events)?;
                        validate_lease_against_events(
                            events,
                            scope,
                            issue_number,
                            lease,
                            expected_holder_id,
                            sample_clock()?,
                        )?;
                        let ledger = workspace_write_operation_ledger(events)?;
                        let matching = ledger
                            .intents
                            .values()
                            .filter(|intent| {
                                intent.mission_id == lease.mission_id
                                    && intent.execution_generation == lease.execution_generation
                                    && intent.input_sha256 == input_sha256
                                    && intent.input == input
                                    && (observed_before == intent.before_source_sha256
                                        || observed_before == intent.after_source_sha256)
                            })
                            .collect::<Vec<_>>();
                        if matching.len() > 1 {
                            return Err(error(
                                "tool_operation_recovery_ambiguous",
                                "inspect the candidate and durable write history before replaying an operation with multiple matching source states",
                                json!({"mission_id":lease.mission_id,"observed_sha256":observed_before}),
                            ));
                        }
                        let selected = matching.first().copied();
                        validate_unresolved_workspace_write(
                            &ledger,
                            &lease.mission_id,
                            lease.execution_generation,
                            &input,
                            &request_id,
                            selected.map(|intent| intent.operation_id.as_str()),
                            &observed_before,
                        )?;
                        if let Some(intent) = selected {
                            if ledger.intents.values().any(|existing| {
                                existing.mission_id == lease.mission_id
                                    && existing.execution_generation == lease.execution_generation
                                    && existing.request_id == request_id
                                    && existing.operation_id != intent.operation_id
                            }) {
                                return Err(error(
                                    "tool_operation_request_id_conflict",
                                    "use a new JSON-RPC correlation ID for a different workspace-write input",
                                    json!({"mission_id":lease.mission_id}),
                                ));
                            }
                            if let Some(result) = ledger.results.get(&intent.operation_id) {
                                if observed_before != result.observed_source_sha256 {
                                    return Err(error(
                                        "tool_operation_effect_diverged",
                                        "the completed operation's post-state is no longer present; verify the candidate before replying with its stored success",
                                        json!({"mission_id":lease.mission_id,"operation_id":intent.operation_id,"expected_sha256":result.observed_source_sha256,"observed_sha256":observed_before}),
                                    ));
                                }
                                return Ok(JournalTransaction::Return((
                                    intent.clone(),
                                    Some(result.clone()),
                                )));
                            }
                            return Ok(JournalTransaction::Return((intent.clone(), None)));
                        }

                        let operation_id = workspace_write_operation_id(
                            &lease.mission_id,
                            lease.execution_generation,
                            &input_sha256,
                            &observed_before,
                        )?;
                        if ledger.intents.values().any(|existing| {
                            existing.mission_id == lease.mission_id
                                && existing.execution_generation == lease.execution_generation
                                && existing.request_id == request_id
                                && existing.operation_id != operation_id
                        }) {
                            return Err(error(
                                "tool_operation_request_id_conflict",
                                "use a new JSON-RPC correlation ID for a different workspace-write input",
                                json!({"mission_id":lease.mission_id}),
                            ));
                        }
                        if ledger.intents.contains_key(&operation_id) {
                            return Err(invalid_tool_operation_record(&lease.mission_id));
                        }
                        let intent = WorkspaceWriteIntentV1 {
                        schema_version: 1,
                        mission_id: lease.mission_id.clone(),
                        execution_generation: lease.execution_generation,
                        operation_id: operation_id.clone(),
                        request_id: request_id.clone(),
                        allocation_id: lease.allocation_id.clone(),
                        request_sha256: lease.request_sha256.clone(),
                        lease_id: lease.lease_id.clone(),
                        holder_id: lease.holder_id.clone(),
                        fencing_token: lease.fencing_token,
                        tool_id: "workspace.write_text".to_owned(),
                        input: input.clone(),
                        input_sha256: input_sha256.clone(),
                        path: path.clone(),
                        before_source_sha256: observed_before.clone(),
                        after_source_sha256: after_source_sha256.clone(),
                        replayed: false,
                        };
                        let payload = serde_json::to_value(&intent)
                            .map_err(|_| invalid_tool_operation_record(&lease.mission_id))?;
                        Ok(JournalTransaction::Append {
                            idempotency_key: operation_id,
                            kind: "mission_tool_operation_intent_v1".to_owned(),
                            payload,
                            result: (intent, None),
                        })
                    })
                    .map_err(map_journal_transaction_error)?;

                if let Some(result) = existing_result {
                    return Ok(WorkspaceWriteExecutionResultV1 {
                        operation_id: intent.operation_id,
                        request_id,
                        result_body: result.result_body,
                        replayed: true,
                    });
                }

                if observed_before == intent.before_source_sha256 {
                    guard
                        .transact(|events| {
                            validate_mission_journal(events)?;
                            validate_lease_against_events(
                                events,
                                scope,
                                issue_number,
                                lease,
                                expected_holder_id,
                                sample_clock()?,
                            )?;
                            Ok(JournalTransaction::Return(()))
                        })
                        .map_err(map_journal_transaction_error)?;
                    apply_write(&intent.before_source_sha256, &content)?;
                }

                let observed_after = read_source_sha256()?.to_ascii_lowercase();
                if observed_after != intent.after_source_sha256 {
                    return Err(error(
                        "tool_operation_effect_not_observed",
                        "read back the exact declared post-write bytes; preserve the intent and refuse success while the candidate differs",
                        json!({
                            "mission_id":lease.mission_id,
                            "operation_id":intent.operation_id,
                            "expected_after_sha256":intent.after_source_sha256,
                            "observed_sha256":observed_after,
                        }),
                    ));
                }
                let output_text = json!({
                    "written":true,
                    "path":intent.path,
                    "sha256":intent.after_source_sha256,
                })
                .to_string();
                let result_body = json!({
                    "content":[{"type":"text","text":output_text}],
                    "isError":false,
                });
                let result_bytes = serde_json::to_vec(&result_body).map_err(|_| {
                    invalid_tool_operation_record(&lease.mission_id)
                })?;
                if result_bytes.len() > MAX_WORKSPACE_WRITE_RESPONSE_BYTES {
                    return Err(invalid_tool_operation_record(&lease.mission_id));
                }
                let result = WorkspaceWriteResultV1 {
                    schema_version: 1,
                    mission_id: lease.mission_id.clone(),
                    execution_generation: lease.execution_generation,
                    operation_id: intent.operation_id.clone(),
                    request_id: intent.request_id.clone(),
                    allocation_id: lease.allocation_id.clone(),
                    request_sha256: lease.request_sha256.clone(),
                    input_sha256: intent.input_sha256.clone(),
                    before_source_sha256: intent.before_source_sha256.clone(),
                    observed_source_sha256: observed_after,
                    lease_id: lease.lease_id.clone(),
                    holder_id: lease.holder_id.clone(),
                    fencing_token: lease.fencing_token,
                    result_sha256: sha256(&result_bytes),
                    result_body,
                    replayed: false,
                };
                guard
                    .transact(|events| {
                        validate_mission_journal(events)?;
                        validate_lease_against_events(
                            events,
                            scope,
                            issue_number,
                            lease,
                            expected_holder_id,
                            sample_clock()?,
                        )?;
                        let ledger = workspace_write_operation_ledger(events)?;
                        if let Some(existing) = ledger.results.get(&intent.operation_id) {
                            if existing.result_body == result.result_body {
                                return Ok(JournalTransaction::Return(existing.clone()));
                            }
                            return Err(invalid_tool_operation_record(&lease.mission_id));
                        }
                        let payload = serde_json::to_value(&result)
                            .map_err(|_| invalid_tool_operation_record(&lease.mission_id))?;
                        Ok(JournalTransaction::Append {
                            idempotency_key: workspace_write_result_key(&intent.operation_id),
                            kind: "mission_tool_operation_result_v1".to_owned(),
                            payload,
                            result: result.clone(),
                        })
                    })
                    .map(|saved| WorkspaceWriteExecutionResultV1 {
                        operation_id: intent.operation_id,
                        request_id,
                        result_body: saved.result_body,
                        replayed: saved.replayed,
                    })
                    .map_err(map_journal_transaction_error)
            })
            .map_err(map_journal_transaction_error)
    }

    /// Read-only recovery preflight for the one interrupted workspace write
    /// associated with an exact admitted execution. This discovers a durable
    /// unresolved intent without granting permission to modify the candidate;
    /// callers must acquire a fresh lease and revalidate the intent before any
    /// replay or filesystem effect.
    pub fn workspace_write_recovery_preflight(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        allocation_id: &str,
        execution_generation: u64,
        request: &AgentExecutionRequestV1,
        admitted_base_source_sha256: &str,
        observed_candidate_source_sha256: &str,
    ) -> Result<Option<WorkspaceWriteIntentV1>, AgentRuntimeErrorV1> {
        request.validate_for_issue_number(issue_number)?;
        let admitted_base_source_sha256 = admitted_base_source_sha256.to_ascii_lowercase();
        let observed_candidate_source_sha256 =
            observed_candidate_source_sha256.to_ascii_lowercase();
        if !valid_sha256(&admitted_base_source_sha256)
            || !valid_sha256(&observed_candidate_source_sha256)
        {
            return Err(error(
                "tool_operation_source_hash_invalid",
                "read regular base and candidate source files before checking an interrupted workspace write",
                json!({}),
            ));
        }
        if !valid_allocation_id(allocation_id) {
            return Err(error(
                "execution_allocation_binding_mismatch",
                "use the host-owned allocation recorded for this admitted execution",
                json!({}),
            ));
        }
        let identity = derive_mission_identity_v1(
            scope,
            issue_number,
            &request.requirement_sha256,
            execution_generation,
        )?;
        if request.mission_id != identity.mission_id
            || request.idempotency_key != identity.idempotency_key
        {
            return Err(error(
                "execution_identity_mismatch",
                "rebuild the request from this issue, protected requirement revision, and host-assigned generation",
                json!({"mission_id":identity.mission_id}),
            ));
        }
        let request_sha256 = sha256(&serde_json::to_vec(request).map_err(|_| {
            error(
                "execution_request_serialization_failed",
                "rebuild the versioned execution request before checking write recovery",
                json!({}),
            )
        })?);

        let events = self.journal.replay_read_only().map_err(journal_error)?;
        validate_mission_journal(&events)?;
        let allocation =
            allocation_for_generation(&events, &identity.mission_id, execution_generation)?
                .ok_or_else(|| {
                    error(
                        "execution_allocation_missing",
                        "restore the exact host allocation before checking write recovery",
                        json!({"mission_id":identity.mission_id}),
                    )
                })?;
        let claimed_request_sha256 =
            claim_request_sha256(&events, &identity.mission_id, execution_generation)?.ok_or_else(
                || {
                    error(
                        "execution_claim_missing",
                        "persist the exact execution claim before checking write recovery",
                        json!({"mission_id":identity.mission_id}),
                    )
                },
            )?;
        if allocation.allocation_id != allocation_id
            || allocation.requirement_sha256 != request.requirement_sha256
        {
            return Err(error(
                "execution_allocation_binding_mismatch",
                "use the allocation ID and protected requirement hash reserved for this generation",
                json!({"mission_id":identity.mission_id}),
            ));
        }
        if claimed_request_sha256 != request_sha256 {
            return Err(error(
                "execution_claim_binding_mismatch",
                "rebuild the exact request recorded by the execution claim before checking write recovery",
                json!({"mission_id":identity.mission_id}),
            ));
        }

        let ledger = workspace_write_operation_ledger(&events)?;
        let unresolved = ledger
            .intents
            .values()
            .filter(|intent| {
                intent.mission_id == identity.mission_id
                    && intent.execution_generation == execution_generation
                    && !ledger.results.contains_key(&intent.operation_id)
            })
            .collect::<Vec<_>>();
        if unresolved.len() > 1 {
            return Err(error(
                "tool_operation_recovery_ambiguous",
                "inspect the durable write history and candidate before recovering multiple unresolved operations",
                json!({
                    "mission_id":identity.mission_id,
                    "operation_ids":unresolved.iter().map(|intent| intent.operation_id.as_str()).collect::<Vec<_>>(),
                }),
            ));
        }
        let Some(intent) = unresolved.first().copied() else {
            return Ok(None);
        };
        if intent.allocation_id != allocation_id
            || intent.request_sha256 != request_sha256
            || intent.before_source_sha256 != admitted_base_source_sha256
            || (observed_candidate_source_sha256 != intent.before_source_sha256
                && observed_candidate_source_sha256 != intent.after_source_sha256)
        {
            return Err(error(
                "tool_operation_recovery_binding_mismatch",
                "recreate the candidate from the admitted base and recover only the exact pre-write or post-write hash recorded by the unresolved intent",
                json!({
                    "mission_id":identity.mission_id,
                    "operation_id":intent.operation_id,
                    "expected_before_sha256":intent.before_source_sha256,
                    "expected_after_sha256":intent.after_source_sha256,
                    "observed_sha256":observed_candidate_source_sha256,
                }),
            ));
        }
        Ok(Some(intent.clone()))
    }

    /// Inspect one durable write operation through a validated current lease.
    /// This read is not authorization for a later effect; effectful callers
    /// must still use the lease-checked transition that records the intent.
    pub fn workspace_write_operation_status(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease: &MissionExecutionLeaseV1,
        expected_holder_id: &str,
        now_unix_ms: u64,
        input: &Value,
        observed_source_sha256: &str,
    ) -> Result<Option<WorkspaceWriteOperationStatusV1>, AgentRuntimeErrorV1> {
        let events = self.journal.replay_read_only().map_err(journal_error)?;
        validate_mission_journal(&events)?;
        validate_lease_against_events(
            &events,
            scope,
            issue_number,
            lease,
            expected_holder_id,
            now_unix_ms,
        )?;
        let (_, input_sha256, _) = validate_workspace_write_input(input)?;
        let observed_source_sha256 = observed_source_sha256.to_ascii_lowercase();
        if !valid_sha256(&observed_source_sha256) {
            return Err(error(
                "tool_operation_source_hash_invalid",
                "read the trusted candidate source before inspecting its workspace-write operation",
                json!({}),
            ));
        }
        let ledger = workspace_write_operation_ledger(&events)?;
        let matching = ledger
            .intents
            .values()
            .filter(|intent| {
                intent.mission_id == lease.mission_id
                    && intent.execution_generation == lease.execution_generation
                    && intent.input_sha256 == input_sha256
                    && intent.input == *input
                    && (intent.before_source_sha256 == observed_source_sha256
                        || intent.after_source_sha256 == observed_source_sha256)
            })
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(error(
                "tool_operation_recovery_ambiguous",
                "inspect the candidate and durable write history before reading an ambiguous operation status",
                json!({"mission_id":lease.mission_id,"observed_sha256":observed_source_sha256}),
            ));
        }
        let intent = matching.first().copied();
        Ok(intent.map(|intent| WorkspaceWriteOperationStatusV1 {
            intent: intent.clone(),
            result: ledger.results.get(&intent.operation_id).cloned(),
        }))
    }

    /// Read a fully validated cumulative verifier ledger for an exact lease's
    /// mission. This is for host-owned evidence and observability; it does not
    /// authorize a new verifier operation.
    pub fn verification_cpu_ledger_summary(
        &self,
        lease: &MissionExecutionLeaseV1,
    ) -> Result<MissionVerificationCpuLedgerSummaryV1, AgentRuntimeErrorV1> {
        let events = self.journal.try_replay().map_err(journal_error)?;
        validate_mission_journal(&events)?;
        let mission_leases = execution_lease_history(&events, &lease.mission_id)?;
        if !mission_leases.iter().any(|recorded| {
            recorded.execution_generation == lease.execution_generation
                && recorded.lease_id == lease.lease_id
                && recorded.holder_id == lease.holder_id
                && recorded.fencing_token == lease.fencing_token
                && recorded.request_sha256 == lease.request_sha256
        }) {
            return Err(error(
                "execution_lease_missing",
                "read verifier accounting only for an exact lease recorded in the trusted journal",
                json!({"mission_id":lease.mission_id}),
            ));
        }
        let ledger = mission_verification_cpu_ledger(&events, &lease.mission_id)?;
        let Some((policy_sha256, mission_cpu_budget_ms)) = ledger.policy else {
            return Ok(MissionVerificationCpuLedgerSummaryV1 {
                mission_id: lease.mission_id.clone(),
                policy_sha256: String::new(),
                mission_cpu_budget_ms: 0,
                charged_cpu_ms: 0,
                remaining_cpu_ms: 0,
                reservations: Vec::new(),
            });
        };
        let reservations = ledger
            .reservations
            .iter()
            .map(|reservation| {
                let resolution = ledger
                    .resolutions
                    .iter()
                    .find(|resolution| resolution.reservation_id == reservation.reservation_id);
                MissionVerificationCpuReservationSummaryV1 {
                    reservation_id: reservation.reservation_id.clone(),
                    reserved_cpu_ms: reservation.reserved_cpu_ms,
                    charged_cpu_ms: resolution
                        .map(|resolution| resolution.charged_cpu_ms)
                        .unwrap_or(reservation.reserved_cpu_ms),
                    resolved: resolution.is_some(),
                }
            })
            .collect();
        Ok(MissionVerificationCpuLedgerSummaryV1 {
            mission_id: lease.mission_id.clone(),
            policy_sha256,
            mission_cpu_budget_ms,
            charged_cpu_ms: ledger.charged_cpu_ms,
            remaining_cpu_ms: mission_cpu_budget_ms.saturating_sub(ledger.charged_cpu_ms),
            reservations,
        })
    }

    /// Check that a lease is still the current, unexpired fence. This read-only
    /// check is not atomic with a later tool effect; use `with_valid_lease` at
    /// the host tool boundary when the effect must be serialized with generation
    /// allocation and lease replacement.
    pub fn validate_lease(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease: &MissionExecutionLeaseV1,
        expected_holder_id: &str,
        now_unix_ms: u64,
    ) -> Result<(), AgentRuntimeErrorV1> {
        let events = self.journal.replay().map_err(journal_error)?;
        validate_lease_against_events(
            &events,
            scope,
            issue_number,
            lease,
            expected_holder_id,
            now_unix_ms,
        )
    }

    /// Validate a lease and invoke one short, host-owned tool operation while
    /// holding the mission lock. The shared journal lock is released before the
    /// effect, so unrelated inbox writes and missions can proceed. If another
    /// journal transaction is active, this returns retryable busy instead of
    /// waiting while holding the mission lock. The host must still bound/cancel
    /// the operation, isolate its process and avoid re-entering mission
    /// transitions from inside the closure.
    pub fn with_valid_lease<T>(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease: &MissionExecutionLeaseV1,
        expected_holder_id: &str,
        mut clock: impl FnMut() -> u64,
        operation: impl FnOnce() -> Result<T, AgentRuntimeErrorV1>,
    ) -> Result<T, AgentRuntimeErrorV1> {
        self.journal
            .with_mission_lock(&lease.mission_id, || {
                let events = self.journal.try_replay().map_err(journal_error)?;
                validate_mission_journal(&events)?;
                let now_unix_ms = clock();
                validate_lease_against_events(
                    &events,
                    scope,
                    issue_number,
                    lease,
                    expected_holder_id,
                    now_unix_ms,
                )?;
                if clock().max(now_unix_ms) >= lease.expires_at_unix_ms {
                    return Err(error(
                        "execution_lease_expired",
                        "checkpoint the work and acquire a new lease before continuing",
                        json!({
                            "mission_id":lease.mission_id,
                            "fencing_token":lease.fencing_token,
                            "expires_at_unix_ms":lease.expires_at_unix_ms,
                        }),
                    ));
                }
                operation()
            })
            .map_err(map_journal_transaction_error)
    }

    /// Reserve part of a cumulative mission CPU budget before a trusted host
    /// starts a verifier operation. Reservations are serialized with mission
    /// generation changes and survive lease expiry, retries, and journal reopen.
    /// The caller must resolve a completed reservation with a conservative
    /// measured upper bound; an unresolved reservation keeps its full amount.
    #[allow(clippy::too_many_arguments)]
    pub fn reserve_mission_verification_cpu(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        lease: &MissionExecutionLeaseV1,
        expected_holder_id: &str,
        now_unix_ms: u64,
        policy_sha256: &str,
        mission_cpu_budget_ms: u64,
        reservation_id: &str,
        reserved_cpu_ms: u64,
    ) -> Result<MissionVerificationCpuReservationV1, AgentRuntimeErrorV1> {
        if !valid_sha256(policy_sha256)
            || !valid_allocation_id(reservation_id)
            || mission_cpu_budget_ms == 0
            || reserved_cpu_ms == 0
            || reserved_cpu_ms > mission_cpu_budget_ms
        {
            return Err(error(
                "mission_verification_cpu_reservation_invalid",
                "use a valid host policy hash, a unique operation ID, and a positive reservation within the mission CPU budget",
                json!({"mission_id":lease.mission_id}),
            ));
        }

        self.journal
            .with_mission_guard(&lease.mission_id, |guard| {
                guard.transact(|events| {
                    validate_mission_journal(events)?;
                    validate_lease_against_events(
                        events,
                        scope,
                        issue_number,
                        lease,
                        expected_holder_id,
                        now_unix_ms,
                    )?;
                    let ledger = mission_verification_cpu_ledger(events, &lease.mission_id)?;
                    if let Some(existing) = ledger
                        .reservations
                        .iter()
                        .find(|existing| existing.reservation_id == reservation_id)
                    {
                        return Err(error(
                            "mission_verification_cpu_reservation_replayed",
                            "do not repeat an operation whose durable reservation already exists; recover its result or use a new host-authorized operation ID",
                            json!({
                                "mission_id":lease.mission_id,
                                "reservation_id":reservation_id,
                                "reserved_cpu_ms":existing.reserved_cpu_ms,
                            }),
                        ));
                    }
                    if let Some((existing_policy_sha256, existing_budget_ms)) = ledger.policy {
                        if existing_policy_sha256 != policy_sha256
                            || existing_budget_ms != mission_cpu_budget_ms
                        {
                            return Err(error(
                                "mission_verification_cpu_policy_conflict",
                                "continue with the original host-owned CPU budget policy or admit a new mission identity",
                                json!({"mission_id":lease.mission_id}),
                            ));
                        }
                    }
                    let charged_cpu_ms = ledger.charged_cpu_ms.checked_add(reserved_cpu_ms).ok_or_else(|| {
                        error(
                            "mission_verification_cpu_budget_invalid",
                            "restore the verified mission journal before starting another verifier operation",
                            json!({"mission_id":lease.mission_id}),
                        )
                    })?;
                    if charged_cpu_ms > mission_cpu_budget_ms {
                        return Err(error(
                            "mission_verification_cpu_budget_exhausted",
                            "stop verification for this mission or request a host-authorized budget policy for a new mission identity",
                            json!({
                                "mission_id":lease.mission_id,
                                "charged_cpu_ms":ledger.charged_cpu_ms,
                                "requested_cpu_ms":reserved_cpu_ms,
                                "mission_cpu_budget_ms":mission_cpu_budget_ms,
                            }),
                        ));
                    }

                    let record = MissionVerificationCpuReservationRecordV1 {
                        schema_version: 1,
                        mission_id: lease.mission_id.clone(),
                        execution_generation: lease.execution_generation,
                        lease_id: lease.lease_id.clone(),
                        holder_id: lease.holder_id.clone(),
                        fencing_token: lease.fencing_token,
                        request_sha256: lease.request_sha256.clone(),
                        reservation_id: reservation_id.to_owned(),
                        policy_sha256: policy_sha256.to_owned(),
                        mission_cpu_budget_ms,
                        reserved_cpu_ms,
                    };
                    let payload = serde_json::to_value(&record).map_err(|_| {
                        error(
                            "mission_verification_cpu_journal_invalid",
                            "restore the verified mission journal before starting another verifier operation",
                            json!({"mission_id":lease.mission_id}),
                        )
                    })?;
                    Ok(JournalTransaction::Append {
                        idempotency_key: verification_cpu_reservation_key(
                            &lease.mission_id,
                            reservation_id,
                        ),
                        kind: "mission_verification_cpu_reserved_v1".to_owned(),
                        payload,
                        result: MissionVerificationCpuReservationV1 {
                            mission_id: lease.mission_id.clone(),
                            reservation_id: reservation_id.to_owned(),
                            policy_sha256: policy_sha256.to_owned(),
                            mission_cpu_budget_ms,
                            reserved_cpu_ms,
                            charged_cpu_ms,
                            remaining_cpu_ms: mission_cpu_budget_ms - charged_cpu_ms,
                            replayed: false,
                        },
                    })
                })
                .map_err(map_journal_transaction_error)
            })
            .map_err(map_journal_transaction_error)
    }

    /// Replace an existing worst-case reservation with a conservative measured
    /// CPU upper bound. The original lease may have expired by this point: the
    /// resolution is host accounting for an already-authorized operation, not a
    /// new mission effect. Repeating the same resolution is idempotent.
    pub fn resolve_mission_verification_cpu(
        &self,
        lease: &MissionExecutionLeaseV1,
        reservation_id: &str,
        charged_cpu_ms: u64,
    ) -> Result<(), AgentRuntimeErrorV1> {
        self.journal
            .with_mission_guard(&lease.mission_id, |guard| {
                guard.transact(|events| {
                    validate_mission_journal(events)?;
                    let ledger = mission_verification_cpu_ledger(events, &lease.mission_id)?;
                    let reservation = ledger
                        .reservations
                        .iter()
                        .find(|reservation| reservation.reservation_id == reservation_id)
                        .ok_or_else(|| {
                            error(
                                "mission_verification_cpu_reservation_missing",
                                "restore the durable reservation before recording verifier usage",
                                json!({"mission_id":lease.mission_id,"reservation_id":reservation_id}),
                            )
                        })?;
                    if reservation.execution_generation != lease.execution_generation
                        || reservation.lease_id != lease.lease_id
                        || reservation.holder_id != lease.holder_id
                        || reservation.fencing_token != lease.fencing_token
                        || reservation.request_sha256 != lease.request_sha256
                    {
                        return Err(error(
                            "mission_verification_cpu_reservation_binding_mismatch",
                            "resolve usage only for the exact lease that created the reservation",
                            json!({"mission_id":lease.mission_id,"reservation_id":reservation_id}),
                        ));
                    }
                    if charged_cpu_ms == 0 || charged_cpu_ms > reservation.reserved_cpu_ms {
                        return Err(error(
                            "mission_verification_cpu_usage_invalid",
                            "record a positive conservative CPU upper bound no greater than the durable reservation",
                            json!({
                                "mission_id":lease.mission_id,
                                "reservation_id":reservation_id,
                                "charged_cpu_ms":charged_cpu_ms,
                                "reserved_cpu_ms":reservation.reserved_cpu_ms,
                            }),
                        ));
                    }
                    if let Some(existing) = ledger.resolutions.iter().find(|resolution| {
                        resolution.reservation_id == reservation_id
                    }) {
                        if existing.charged_cpu_ms == charged_cpu_ms {
                            return Ok(JournalTransaction::Return(()));
                        }
                        return Err(error(
                            "mission_verification_cpu_resolution_conflict",
                            "keep the first durable verifier usage result for this operation",
                            json!({"mission_id":lease.mission_id,"reservation_id":reservation_id}),
                        ));
                    }

                    let record = MissionVerificationCpuResolutionRecordV1 {
                        schema_version: 1,
                        mission_id: lease.mission_id.clone(),
                        execution_generation: lease.execution_generation,
                        lease_id: lease.lease_id.clone(),
                        fencing_token: lease.fencing_token,
                        reservation_id: reservation_id.to_owned(),
                        policy_sha256: reservation.policy_sha256.clone(),
                        charged_cpu_ms,
                    };
                    let payload = serde_json::to_value(&record).map_err(|_| {
                        error(
                            "mission_verification_cpu_journal_invalid",
                            "restore the verified mission journal before recording verifier usage",
                            json!({"mission_id":lease.mission_id}),
                        )
                    })?;
                    Ok(JournalTransaction::Append {
                        idempotency_key: verification_cpu_resolution_key(
                            &lease.mission_id,
                            reservation_id,
                        ),
                        kind: "mission_verification_cpu_resolved_v1".to_owned(),
                        payload,
                        result: (),
                    })
                })
                .map_err(map_journal_transaction_error)
            })
            .map_err(map_journal_transaction_error)
    }

    fn claim_inner(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        execution_generation: u64,
        allocation_id: &str,
        request: &AgentExecutionRequestV1,
    ) -> Result<ExecutionClaimResultV1, AgentRuntimeErrorV1> {
        request.validate_for_issue_number(issue_number)?;
        let identity = derive_mission_identity_v1(
            scope,
            issue_number,
            &request.requirement_sha256,
            execution_generation,
        )?;
        if request.mission_id != identity.mission_id
            || request.idempotency_key != identity.idempotency_key
        {
            return Err(error(
                "execution_identity_mismatch",
                "rebuild the request from this issue, protected requirement revision, and host-assigned generation",
                json!({
                    "expected_mission_id":identity.mission_id,
                    "actual_mission_id":request.mission_id,
                    "expected_idempotency_key":identity.idempotency_key,
                    "actual_idempotency_key":request.idempotency_key,
                }),
            ));
        }
        let serialized_request = serde_json::to_vec(request).map_err(|_| {
            error(
                "execution_request_serialization_failed",
                "rebuild the versioned execution request before claiming it",
                json!({}),
            )
        })?;
        let request_hash = sha256(&serialized_request);
        let idempotency_key = format!("mission-execution-claim-v1:{}", identity.idempotency_key);
        let payload = json!({
            "schema_version": 1,
            "mission_id": identity.mission_id,
            "execution_generation": identity.execution_generation,
            "request_sha256": request_hash,
        });
        self.journal
            .transact_mission(&identity.mission_id, |events| {
                validate_mission_journal(events)?;
                let highest_generation = highest_known_generation(events, &identity.mission_id)?;
                match allocation_for_generation(
                    events,
                    &identity.mission_id,
                    identity.execution_generation,
                )? {
                    Some(allocation) if allocation.allocation_id == allocation_id => {
                        if allocation.requirement_sha256 != request.requirement_sha256 {
                            return Err(error(
                                "execution_allocation_binding_mismatch",
                                "use the same protected requirement hash that reserved this generation",
                                json!({"mission_id":identity.mission_id}),
                            ));
                        }
                    }
                    Some(allocation) => {
                        return Err(error(
                            "execution_generation_reserved",
                            "claim this generation with the allocation ID that reserved it",
                            json!({
                                "mission_id":identity.mission_id,
                                "execution_generation":identity.execution_generation,
                                "allocation_id":allocation.allocation_id,
                            }),
                        ));
                    }
                    None => {
                        return Err(error(
                            "execution_allocation_missing",
                            "retry admission with the host-owned allocation that reserved this generation",
                            json!({
                                "mission_id":identity.mission_id,
                                "execution_generation":identity.execution_generation,
                            }),
                        ));
                    }
                }

                if let Some(existing) = events
                    .iter()
                    .find(|event| event.idempotency_key == idempotency_key)
                {
                    if existing.kind == "mission_execution_claim_v1" && existing.payload == payload {
                        return Ok(JournalTransaction::Return(
                            ExecutionClaimResultV1::AlreadyClaimed(identity.clone()),
                        ));
                    }
                    return Err(AgentRuntimeErrorV1::new(
                        "execution_idempotency_conflict",
                        false,
                        "keep the same request for this generation or use a new host-approved generation",
                        json!({
                            "mission_id":identity.mission_id,
                            "execution_generation":identity.execution_generation,
                        }),
                    ));
                }

                if identity.execution_generation < highest_generation {
                    return Err(error(
                        "execution_generation_superseded",
                        "retry admission using the latest host-assigned generation",
                        json!({
                            "mission_id":identity.mission_id,
                            "requested_generation":identity.execution_generation,
                            "highest_known_generation":highest_generation,
                        }),
                    ));
                }

                Ok(JournalTransaction::Append {
                    idempotency_key: idempotency_key.clone(),
                    kind: "mission_execution_claim_v1".to_owned(),
                    payload: payload.clone(),
                    result: ExecutionClaimResultV1::Claimed(identity.clone()),
                })
            })
            .map_err(map_journal_transaction_error)
    }

    /// Replay and validate the existing hash-chained journal; the returned entries
    /// include the inbox's webhook records as well as execution claim records.
    pub fn replay(&self) -> Result<Vec<JournalEvent>, AgentRuntimeErrorV1> {
        self.journal.replay().map_err(journal_error)
    }

    /// Verify one exact request against its durable allocation and claim.
    ///
    /// This uses a bounded strict replay: an incomplete final journal record
    /// is refused without repairing or otherwise changing the journal. A
    /// newer generation also invalidates this claim for decisions such as a
    /// pull-request merge.
    pub fn verify_claimed_request(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        allocation_id: &str,
        execution_generation: u64,
        request: &AgentExecutionRequestV1,
    ) -> Result<MissionExecutionClaimVerificationV1, AgentRuntimeErrorV1> {
        request.validate_for_issue_number(issue_number)?;
        if !valid_allocation_id(allocation_id) {
            return Err(error(
                "execution_allocation_id_invalid",
                "use the host-owned allocation ID recorded in the mission admission",
                json!({}),
            ));
        }
        let identity = derive_mission_identity_v1(
            scope,
            issue_number,
            &request.requirement_sha256,
            execution_generation,
        )?;
        if request.mission_id != identity.mission_id
            || request.idempotency_key != identity.idempotency_key
        {
            return Err(error(
                "execution_identity_mismatch",
                "use the request derived from this issue, requirement revision, and host generation",
                json!({"mission_id":identity.mission_id}),
            ));
        }
        let request_bytes = serde_json::to_vec(request).map_err(|_| {
            error(
                "execution_request_serialization_failed",
                "rebuild the versioned execution request before verifying its claim",
                json!({}),
            )
        })?;
        let request_sha256 = sha256(&request_bytes);
        let events = self.journal.replay_read_only().map_err(journal_error)?;
        validate_mission_journal(&events)?;

        let allocation =
            allocation_for_generation(&events, &identity.mission_id, execution_generation)?
                .ok_or_else(|| {
                    error(
                        "execution_allocation_missing",
                        "restore the exact host allocation before authorizing a merge",
                        json!({
                            "mission_id":identity.mission_id,
                            "execution_generation":execution_generation,
                        }),
                    )
                })?;
        if allocation.allocation_id != allocation_id
            || allocation.requirement_sha256 != request.requirement_sha256
        {
            return Err(error(
                "execution_allocation_binding_mismatch",
                "use the host allocation recorded for this issue and requirement revision",
                json!({
                    "mission_id":identity.mission_id,
                    "execution_generation":execution_generation,
                }),
            ));
        }

        let claimed_request_sha256 =
            claim_request_sha256(&events, &identity.mission_id, execution_generation)?.ok_or_else(
                || {
                    error(
                        "execution_claim_missing",
                        "a merge requires the request claim to be durable before verification",
                        json!({
                            "mission_id":identity.mission_id,
                            "execution_generation":execution_generation,
                        }),
                    )
                },
            )?;
        if claimed_request_sha256 != request_sha256 {
            return Err(error(
                "execution_claim_request_mismatch",
                "restore the exact request bytes that were durably claimed before verifying this candidate",
                json!({"mission_id":identity.mission_id,"execution_generation":execution_generation}),
            ));
        }
        let highest_generation = highest_known_generation(&events, &identity.mission_id)?;
        if highest_generation != execution_generation {
            return Err(error(
                "execution_claim_superseded",
                "admit and verify the latest host-assigned execution generation",
                json!({
                    "mission_id":identity.mission_id,
                    "claimed_generation":execution_generation,
                    "highest_known_generation":highest_generation,
                }),
            ));
        }

        Ok(MissionExecutionClaimVerificationV1 {
            mission_id: identity.mission_id,
            allocation_id: allocation.allocation_id,
            execution_generation,
            request_sha256,
        })
    }

    /// Revalidate a claim while holding the same per-mission lock used by
    /// generation allocation, then keep that lock for one bounded host-owned
    /// effect. Merge authorities use this around their final Forgejo state
    /// reads and merge POST so a newer cooperative allocation cannot appear
    /// between the generation check and that POST. The effect must have a
    /// deadline and must not perform another transition for this mission.
    pub fn with_current_claimed_request<T>(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        allocation_id: &str,
        execution_generation: u64,
        request: &AgentExecutionRequestV1,
        effect: impl FnOnce(&MissionExecutionClaimVerificationV1) -> Result<T, AgentRuntimeErrorV1>,
    ) -> Result<T, AgentRuntimeErrorV1> {
        let identity = derive_mission_identity_v1(
            scope,
            issue_number,
            &request.requirement_sha256,
            execution_generation,
        )?;
        self.journal
            .with_mission_lock(&identity.mission_id, || {
                let verification = self.verify_claimed_request(
                    scope,
                    issue_number,
                    allocation_id,
                    execution_generation,
                    request,
                )?;
                effect(&verification)
            })
            .map_err(map_journal_transaction_error)
    }

    /// Execute one bounded host-owned merge decision with a durable one-use
    /// intent. `prepare` must perform only read-only final checks. For a new
    /// attempt, those checks finish before the intent is fsynced; `effect` runs
    /// only afterward. A restart/retry receives `RecoverOnly`, and a resolved
    /// result is replayed without invoking either closure. The mission lock
    /// covers claim validation, preparation, intent append, effect, and
    /// resolution append; the journal lock is held only for short transitions.
    #[allow(clippy::too_many_arguments)]
    pub fn with_merge_attempt<P>(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        allocation_id: &str,
        execution_generation: u64,
        request: &AgentExecutionRequestV1,
        binding: MergeAttemptBindingV1,
        prepare: impl FnOnce(
            MergeAttemptModeV1,
            &MissionExecutionClaimVerificationV1,
        ) -> Result<P, AgentRuntimeErrorV1>,
        effect: impl FnOnce(
            MergeAttemptModeV1,
            &MissionExecutionClaimVerificationV1,
            P,
        ) -> Result<MergeAttemptResolutionV1, AgentRuntimeErrorV1>,
    ) -> Result<MergeAttemptResolutionV1, AgentRuntimeErrorV1> {
        let identity = derive_mission_identity_v1(
            scope,
            issue_number,
            &request.requirement_sha256,
            execution_generation,
        )?;
        validate_merge_binding_shape(&binding)?;
        if binding.scope_sha256 != merge_scope_sha256(scope)
            || binding.issue_number != issue_number
            || binding.requirement_sha256 != request.requirement_sha256.to_ascii_lowercase()
        {
            return Err(error(
                "merge_attempt_binding_mismatch",
                "rebuild the merge binding from the currently admitted scope, issue, and requirement revision",
                json!({"mission_id":identity.mission_id}),
            ));
        }
        let attempt_id = merge_attempt_id(
            &identity.mission_id,
            execution_generation,
            binding.pull_request_number,
        );
        self.journal
            .with_mission_guard(&identity.mission_id, |guard| {
                let verification = self.verify_claimed_request(
                    scope,
                    issue_number,
                    allocation_id,
                    execution_generation,
                    request,
                )?;
                if binding.request_sha256 != verification.request_sha256 {
                    return Err(error(
                        "merge_attempt_binding_mismatch",
                        "rebuild the merge binding from the exact durably claimed admission",
                        json!({"mission_id":identity.mission_id}),
                    ));
                }
                let expected_started = MergeAttemptStartedV1 {
                    schema_version: 1,
                    mission_id: identity.mission_id.clone(),
                    attempt_id: attempt_id.clone(),
                    allocation_id: verification.allocation_id.clone(),
                    execution_generation,
                    request_sha256: verification.request_sha256.clone(),
                    binding: binding.clone(),
                };
                let (mode, saved) = guard.transact(|events| {
                    validate_mission_journal(events)?;
                    if let Some(unresolved) =
                        unresolved_merge_attempt_for_mission(events, &identity.mission_id)?
                        && unresolved != attempt_id
                    {
                        return Err(error(
                            "execution_merge_attempt_unresolved",
                            "recover the existing pull-request merge intent before starting another attempt for this mission",
                            json!({"mission_id":identity.mission_id}),
                        ));
                    }
                    if let Some(unresolved) = unresolved_merge_attempt_for_scope(
                        events,
                        &binding.scope_sha256,
                    )? && unresolved.attempt_id != attempt_id
                    {
                        return Err(AgentRuntimeErrorV1::new(
                            "execution_merge_target_unresolved",
                            true,
                            "recover the blocking pull request through authenticated readback before starting another merge attempt on this base branch",
                            json!({"mission_id":identity.mission_id,"pull_request_number":binding.pull_request_number,"blocking_pull_request_number":unresolved.binding.pull_request_number}),
                        ));
                    }
                    match merge_attempt_state(events, &attempt_id)? {
                        Some((started, resolved)) => {
                            if started != expected_started {
                                return Err(error(
                                    "merge_attempt_binding_mismatch",
                                    "use a new host-approved execution generation for changed merge inputs",
                                    json!({"mission_id":identity.mission_id,"pull_request_number":binding.pull_request_number}),
                                ));
                            }
                            Ok(JournalTransaction::Return((
                                MergeAttemptModeV1::RecoverOnly,
                                resolved
                                    .filter(|entry| entry.resolution.is_terminal())
                                    .map(|entry| entry.resolution),
                            )))
                        }
                        None => Ok(JournalTransaction::Return((
                            MergeAttemptModeV1::NewAttempt,
                            None,
                        ))),
                    }
                })
                .map_err(map_journal_transaction_error)?;
                if let Some(saved) = saved {
                    return Ok(saved);
                }

                let prepared = prepare(mode, &verification)?;
                if mode == MergeAttemptModeV1::NewAttempt {
                    let started_for_append = expected_started.clone();
                    guard
                        .transact(|events| {
                            validate_mission_journal(events)?;
                            if let Some(unresolved) = unresolved_merge_attempt_for_scope(
                                events,
                                &binding.scope_sha256,
                            )? && unresolved.attempt_id != attempt_id
                            {
                                return Err(AgentRuntimeErrorV1::new(
                                    "execution_merge_target_unresolved",
                                    true,
                                    "recover the blocking pull request through authenticated readback before appending another merge attempt on this base branch",
                                    json!({"mission_id":identity.mission_id,"pull_request_number":binding.pull_request_number,"blocking_pull_request_number":unresolved.binding.pull_request_number}),
                                ));
                            }
                            match merge_attempt_state(events, &attempt_id)? {
                                None => Ok(JournalTransaction::Append {
                                    idempotency_key: attempt_id.clone(),
                                    kind: "mission_merge_attempt_started_v1".to_owned(),
                                    payload: serde_json::to_value(started_for_append).map_err(
                                        |_| {
                                            error(
                                                "merge_attempt_serialization_failed",
                                                "rebuild the versioned merge binding before starting the attempt",
                                                json!({"mission_id":identity.mission_id}),
                                            )
                                        },
                                    )?,
                                    result: (),
                                }),
                                _ => Err(error(
                                    "merge_attempt_journal_invalid",
                                    "reload the current merge attempt before any merge side effect",
                                    json!({"mission_id":identity.mission_id}),
                                )),
                            }
                        })
                        .map_err(map_journal_transaction_error)?;
                }

                let resolution = effect(mode, &verification, prepared)?;
                validate_merge_resolution(&resolution)?;
                if !resolution.is_terminal() {
                    return Ok(resolution);
                }
                let binding_sha256 = merge_binding_sha256(&binding)?;
                let resolved = MergeAttemptResolvedV1 {
                    schema_version: 1,
                    mission_id: identity.mission_id.clone(),
                    attempt_id: attempt_id.clone(),
                    binding_sha256: binding_sha256.clone(),
                    resolution: resolution.clone(),
                };
                guard
                    .transact(|events| {
                    validate_mission_journal(events)?;
                    match merge_attempt_state(events, &attempt_id)? {
                        Some((started, Some(existing))) if existing.resolution.is_terminal() => {
                            if started != expected_started
                                || existing.binding_sha256 != binding_sha256
                                || existing.resolution != resolution
                            {
                                return Err(error(
                                    "merge_attempt_resolution_conflict",
                                    "read the durable merge resolution and do not replace it",
                                    json!({"mission_id":identity.mission_id}),
                                ));
                            }
                            Ok(JournalTransaction::Return(existing.resolution))
                        }
                        Some((started, existing))
                            if started == expected_started
                                && existing.as_ref().is_none_or(|entry| {
                                    !entry.resolution.is_terminal()
                                }) =>
                        {
                            let idempotency_key = if existing.is_some() {
                                merge_recovered_resolution_idempotency_key(&attempt_id)
                            } else {
                                merge_resolution_idempotency_key(&attempt_id)
                            };
                            Ok(JournalTransaction::Append {
                                idempotency_key,
                                kind: "mission_merge_attempt_resolved_v1".to_owned(),
                                payload: serde_json::to_value(resolved).map_err(|_| {
                                    error(
                                        "merge_attempt_serialization_failed",
                                        "rebuild the versioned merge resolution before recording it",
                                        json!({"mission_id":identity.mission_id}),
                                    )
                                })?,
                                result: resolution.clone(),
                            })
                        }
                        _ => Err(error(
                            "merge_attempt_journal_invalid",
                            "restore a verified journal snapshot before resolving this merge attempt",
                            json!({"mission_id":identity.mission_id}),
                        )),
                    }
                    })
                    .map_err(map_journal_transaction_error)
            })
            .map_err(map_journal_transaction_error)
    }

    /// Resolve an already-started merge intent without creating a new one.
    /// This entry point is used before ordinary open-PR preflight so a restart
    /// can inspect a closed or still-open PR. The callback belongs to trusted
    /// host code and must use authenticated read-only observations only.
    #[allow(clippy::too_many_arguments)]
    pub fn recover_merge_attempt_if_present(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
        allocation_id: &str,
        execution_generation: u64,
        request: &AgentExecutionRequestV1,
        binding: MergeAttemptBindingV1,
        readback: impl FnOnce(
            &MissionExecutionClaimVerificationV1,
        ) -> Result<MergeAttemptResolutionV1, AgentRuntimeErrorV1>,
    ) -> Result<Option<MergeAttemptResolutionV1>, AgentRuntimeErrorV1> {
        let identity = derive_mission_identity_v1(
            scope,
            issue_number,
            &request.requirement_sha256,
            execution_generation,
        )?;
        validate_merge_binding_shape(&binding)?;
        if binding.scope_sha256 != merge_scope_sha256(scope)
            || binding.issue_number != issue_number
            || binding.requirement_sha256 != request.requirement_sha256.to_ascii_lowercase()
        {
            return Err(error(
                "merge_attempt_binding_mismatch",
                "rebuild the merge binding from the currently admitted scope, issue, and requirement revision",
                json!({"mission_id":identity.mission_id}),
            ));
        }
        let attempt_id = merge_attempt_id(
            &identity.mission_id,
            execution_generation,
            binding.pull_request_number,
        );
        self.journal
            .with_mission_guard(&identity.mission_id, |guard| {
                let verification = self.verify_claimed_request(
                    scope,
                    issue_number,
                    allocation_id,
                    execution_generation,
                    request,
                )?;
                if binding.request_sha256 != verification.request_sha256 {
                    return Err(error(
                        "merge_attempt_binding_mismatch",
                        "rebuild the merge binding from the exact durably claimed admission",
                        json!({"mission_id":identity.mission_id}),
                    ));
                }
                let expected_started = MergeAttemptStartedV1 {
                    schema_version: 1,
                    mission_id: identity.mission_id.clone(),
                    attempt_id: attempt_id.clone(),
                    allocation_id: verification.allocation_id.clone(),
                    execution_generation,
                    request_sha256: verification.request_sha256.clone(),
                    binding: binding.clone(),
                };
                let (state, unresolved_target) = guard
                    .transact(|events| {
                        validate_mission_journal(events)?;
                        Ok(JournalTransaction::Return((
                            merge_attempt_state(events, &attempt_id)?,
                            unresolved_merge_attempt_for_scope(events, &binding.scope_sha256)?,
                        )))
                    })
                    .map_err(map_journal_transaction_error)?;
                let Some((started, resolved)) = state else {
                    if let Some(unresolved) = unresolved_target {
                        return Err(AgentRuntimeErrorV1::new(
                            "execution_merge_target_unresolved",
                            true,
                            "recover the blocking pull request through authenticated readback under its admitted mission before continuing on this base branch",
                            json!({"mission_id":identity.mission_id,"pull_request_number":binding.pull_request_number,"blocking_pull_request_number":unresolved.binding.pull_request_number}),
                        ));
                    }
                    return Ok(None);
                };
                if started != expected_started {
                    return Err(error(
                        "merge_attempt_binding_mismatch",
                        "use a new host-approved execution generation for changed merge inputs",
                        json!({"mission_id":identity.mission_id,"pull_request_number":binding.pull_request_number}),
                    ));
                }
                if let Some(resolved) = resolved.as_ref() {
                    if resolved.resolution.is_terminal() {
                        return Ok(Some(resolved.resolution.clone()));
                    }
                }

                let resolution = readback(&verification)?;
                validate_merge_resolution(&resolution)?;
                if !resolution.is_terminal() {
                    return Ok(Some(resolution));
                }
                let resolved = MergeAttemptResolvedV1 {
                    schema_version: 1,
                    mission_id: identity.mission_id.clone(),
                    attempt_id: attempt_id.clone(),
                    binding_sha256: merge_binding_sha256(&binding)?,
                    resolution: resolution.clone(),
                };
                guard
                    .transact(|events| {
                        validate_mission_journal(events)?;
                        match merge_attempt_state(events, &attempt_id)? {
                            Some((started, Some(existing)))
                                if started == expected_started
                                    && existing.resolution.is_terminal()
                                    && existing == resolved =>
                            {
                                Ok(JournalTransaction::Return(existing.resolution))
                            }
                            Some((started, existing))
                                if started == expected_started
                                    && existing.as_ref().is_none_or(|entry| {
                                        !entry.resolution.is_terminal()
                                    }) =>
                            {
                                let idempotency_key = if existing.is_some() {
                                    merge_recovered_resolution_idempotency_key(&attempt_id)
                                } else {
                                    merge_resolution_idempotency_key(&attempt_id)
                                };
                                Ok(JournalTransaction::Append {
                                    idempotency_key,
                                    kind: "mission_merge_attempt_resolved_v1".to_owned(),
                                    payload: serde_json::to_value(resolved).map_err(|_| {
                                        error(
                                            "merge_attempt_serialization_failed",
                                            "rebuild the versioned merge resolution before recording it",
                                            json!({"mission_id":identity.mission_id}),
                                        )
                                    })?,
                                    result: resolution.clone(),
                                })
                            }
                            _ => Err(error(
                                "merge_attempt_journal_invalid",
                                "restore a verified journal snapshot before resolving this merge attempt",
                                json!({"mission_id":identity.mission_id}),
                            )),
                        }
                    })
                    .map(Some)
                    .map_err(map_journal_transaction_error)
            })
            .map_err(map_journal_transaction_error)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MergeAttemptStartedV1 {
    schema_version: u32,
    mission_id: String,
    attempt_id: String,
    allocation_id: String,
    execution_generation: u64,
    request_sha256: String,
    binding: MergeAttemptBindingV1,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MergeAttemptResolvedV1 {
    schema_version: u32,
    mission_id: String,
    attempt_id: String,
    binding_sha256: String,
    resolution: MergeAttemptResolutionV1,
}

fn merge_scope_sha256(scope: &BaseBranchScope) -> String {
    sha256_fields(&[
        b"agent-forge-merge-scope-v1",
        scope.origin().as_bytes(),
        scope.owner().as_bytes(),
        scope.repository().as_bytes(),
        scope.branch().as_bytes(),
    ])
}

fn validate_merge_binding_shape(
    binding: &MergeAttemptBindingV1,
) -> Result<(), AgentRuntimeErrorV1> {
    if binding.schema_version != 1
        || !valid_lower_sha256(&binding.scope_sha256)
        || binding.issue_number == 0
        || binding.pull_request_number == 0
        || !valid_git_object_id(&binding.base_commit)
        || !valid_git_object_id(&binding.candidate_commit)
        || !valid_lower_sha256(&binding.requirement_sha256)
        || !valid_lower_sha256(&binding.policy_sha256)
        || !valid_lower_sha256(&binding.proof_sha256)
        || !valid_lower_sha256(&binding.review_sha256)
        || !valid_lower_sha256(&binding.request_sha256)
        || !valid_lower_sha256(&binding.admission_sha256)
    {
        return Err(error(
            "merge_attempt_binding_invalid",
            "rebuild the versioned merge binding with positive identifiers and canonical Git/SHA-256 object IDs",
            json!({}),
        ));
    }
    Ok(())
}

fn validate_merge_resolution(
    resolution: &MergeAttemptResolutionV1,
) -> Result<(), AgentRuntimeErrorV1> {
    let valid = resolution.schema_version == 1
        && match resolution.outcome {
            MergeAttemptOutcomeV1::Merged => {
                resolution.terminal
                    && resolution.retryable != Some(true)
                    && resolution.reason_code.is_none()
                    && resolution
                        .merge_commit
                        .as_deref()
                        .is_some_and(valid_git_object_id)
                    && resolution
                        .merge_tree_manifest_sha256
                        .as_deref()
                        .is_some_and(valid_lower_sha256)
                    && resolution
                        .observation_sha256
                        .as_deref()
                        .is_some_and(valid_lower_sha256)
            }
            MergeAttemptOutcomeV1::Rejected => {
                resolution.terminal
                    && resolution
                        .reason_code
                        .as_deref()
                        .is_some_and(valid_allocation_id)
                    && resolution.merge_commit.is_none()
                    && resolution.merge_tree_manifest_sha256.is_none()
                    && resolution.observation_sha256.is_none()
            }
            MergeAttemptOutcomeV1::Inconclusive => {
                resolution
                    .reason_code
                    .as_deref()
                    .is_some_and(valid_allocation_id)
                    && resolution.merge_commit.is_none()
                    && resolution.merge_tree_manifest_sha256.is_none()
                    && resolution.observation_sha256.is_none()
            }
        };
    if !valid {
        return Err(error(
            "merge_attempt_resolution_invalid",
            "provide a versioned merged result with exact readback hashes or an inconclusive reason code",
            json!({}),
        ));
    }
    Ok(())
}

fn valid_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn merge_attempt_id(mission_id: &str, generation: u64, pull_request_number: u64) -> String {
    format!("mission-merge-attempt-v1:{mission_id}:{generation}:{pull_request_number}")
}

fn merge_resolution_idempotency_key(attempt_id: &str) -> String {
    format!("{attempt_id}:resolved")
}

fn merge_recovered_resolution_idempotency_key(attempt_id: &str) -> String {
    format!("{attempt_id}:resolved-after-recovery")
}

fn merge_binding_sha256(binding: &MergeAttemptBindingV1) -> Result<String, AgentRuntimeErrorV1> {
    serde_json::to_vec(binding)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| {
            error(
                "merge_attempt_serialization_failed",
                "rebuild the versioned merge binding before recording the attempt",
                json!({}),
            )
        })
}

fn invalid_merge_attempt_record(mission_id: &str) -> AgentRuntimeErrorV1 {
    error(
        "merge_attempt_journal_invalid",
        "restore a verified journal snapshot before resolving or retrying this merge attempt",
        json!({"mission_id":mission_id}),
    )
}

fn parse_merge_attempt_started(
    event: &JournalEvent,
) -> Result<MergeAttemptStartedV1, AgentRuntimeErrorV1> {
    let started: MergeAttemptStartedV1 = serde_json::from_value(event.payload.clone())
        .map_err(|_| invalid_merge_attempt_record("unknown"))?;
    if event.kind != "mission_merge_attempt_started_v1"
        || started.schema_version != 1
        || !started
            .mission_id
            .strip_prefix("mission-v1-")
            .is_some_and(valid_lower_sha256)
        || started.attempt_id
            != merge_attempt_id(
                &started.mission_id,
                started.execution_generation,
                started.binding.pull_request_number,
            )
        || !valid_allocation_id(&started.allocation_id)
        || started.execution_generation == 0
        || !valid_lower_sha256(&started.request_sha256)
        || started.request_sha256 != started.binding.request_sha256
        || validate_merge_binding_shape(&started.binding).is_err()
        || event.idempotency_key != started.attempt_id
    {
        return Err(invalid_merge_attempt_record(&started.mission_id));
    }
    Ok(started)
}

fn parse_merge_attempt_resolved(
    event: &JournalEvent,
) -> Result<MergeAttemptResolvedV1, AgentRuntimeErrorV1> {
    let resolved: MergeAttemptResolvedV1 = serde_json::from_value(event.payload.clone())
        .map_err(|_| invalid_merge_attempt_record("unknown"))?;
    if event.kind != "mission_merge_attempt_resolved_v1"
        || resolved.schema_version != 1
        || !resolved
            .mission_id
            .strip_prefix("mission-v1-")
            .is_some_and(valid_lower_sha256)
        || resolved.attempt_id.trim().is_empty()
        || !valid_lower_sha256(&resolved.binding_sha256)
        || validate_merge_resolution(&resolved.resolution).is_err()
        || ![
            merge_resolution_idempotency_key(&resolved.attempt_id),
            merge_recovered_resolution_idempotency_key(&resolved.attempt_id),
        ]
        .contains(&event.idempotency_key)
    {
        return Err(invalid_merge_attempt_record(&resolved.mission_id));
    }
    Ok(resolved)
}

fn validate_merge_attempt_history(events: &[JournalEvent]) -> Result<(), AgentRuntimeErrorV1> {
    let mut starts = std::collections::BTreeMap::<String, MergeAttemptStartedV1>::new();
    let mut resolutions = std::collections::BTreeMap::<String, MergeAttemptResolvedV1>::new();
    for event in events {
        match event.kind.as_str() {
            "mission_merge_attempt_started_v1" => {
                let started = parse_merge_attempt_started(event)?;
                if starts
                    .insert(started.attempt_id.clone(), started.clone())
                    .is_some()
                {
                    return Err(invalid_merge_attempt_record(&started.mission_id));
                }
            }
            "mission_merge_attempt_resolved_v1" => {
                let resolved = parse_merge_attempt_resolved(event)?;
                let Some(started) = starts.get(&resolved.attempt_id) else {
                    return Err(invalid_merge_attempt_record(&resolved.mission_id));
                };
                let valid_transition = match resolutions.get(&resolved.attempt_id) {
                    None => {
                        event.idempotency_key
                            == merge_resolution_idempotency_key(&resolved.attempt_id)
                    }
                    Some(previous) => {
                        !previous.resolution.is_terminal()
                            && resolved.resolution.is_terminal()
                            && event.idempotency_key
                                == merge_recovered_resolution_idempotency_key(&resolved.attempt_id)
                    }
                };
                if started.mission_id != resolved.mission_id
                    || merge_binding_sha256(&started.binding)? != resolved.binding_sha256
                    || !valid_transition
                {
                    return Err(invalid_merge_attempt_record(&resolved.mission_id));
                }
                resolutions.insert(resolved.attempt_id.clone(), resolved.clone());
            }
            _ => {}
        }
    }

    for (attempt_id, started) in &starts {
        let allocation =
            allocation_for_generation(events, &started.mission_id, started.execution_generation)?
                .ok_or_else(|| invalid_merge_attempt_record(&started.mission_id))?;
        let claimed_request_sha256 =
            claim_request_sha256(events, &started.mission_id, started.execution_generation)?;
        if allocation.allocation_id != started.allocation_id
            || allocation.requirement_sha256 != started.binding.requirement_sha256
            || claimed_request_sha256.as_deref() != Some(started.request_sha256.as_str())
            || started.binding.request_sha256 != started.request_sha256
            || started.attempt_id != *attempt_id
        {
            return Err(invalid_merge_attempt_record(&started.mission_id));
        }
    }
    Ok(())
}

fn merge_attempt_state(
    events: &[JournalEvent],
    attempt_id: &str,
) -> Result<Option<(MergeAttemptStartedV1, Option<MergeAttemptResolvedV1>)>, AgentRuntimeErrorV1> {
    let mut started = None;
    let mut resolved = None;
    for event in events {
        match event.kind.as_str() {
            "mission_merge_attempt_started_v1" => {
                let value = parse_merge_attempt_started(event)?;
                if value.attempt_id == attempt_id {
                    started = Some(value);
                }
            }
            "mission_merge_attempt_resolved_v1" => {
                let value = parse_merge_attempt_resolved(event)?;
                if value.attempt_id == attempt_id {
                    resolved = Some(value);
                }
            }
            _ => {}
        }
    }
    match (started, resolved) {
        (None, None) => Ok(None),
        (Some(started), resolved) => Ok(Some((started, resolved))),
        (None, Some(resolved)) => Err(invalid_merge_attempt_record(&resolved.mission_id)),
    }
}

fn unresolved_merge_attempt_for_mission(
    events: &[JournalEvent],
    mission_id: &str,
) -> Result<Option<String>, AgentRuntimeErrorV1> {
    let mut started = std::collections::BTreeMap::<String, MergeAttemptStartedV1>::new();
    let mut resolved = std::collections::BTreeSet::<String>::new();
    for event in events {
        match event.kind.as_str() {
            "mission_merge_attempt_started_v1" => {
                let attempt = parse_merge_attempt_started(event)?;
                started.insert(attempt.attempt_id.clone(), attempt);
            }
            "mission_merge_attempt_resolved_v1" => {
                let attempt = parse_merge_attempt_resolved(event)?;
                if attempt.resolution.is_terminal() {
                    resolved.insert(attempt.attempt_id);
                }
            }
            _ => {}
        }
    }
    Ok(started
        .into_iter()
        .find(|(attempt_id, attempt)| {
            attempt.mission_id == mission_id && !resolved.contains(attempt_id)
        })
        .map(|(attempt_id, _)| attempt_id))
}

fn unresolved_merge_attempt_for_scope(
    events: &[JournalEvent],
    scope_sha256: &str,
) -> Result<Option<MergeAttemptStartedV1>, AgentRuntimeErrorV1> {
    let mut started = std::collections::BTreeMap::<String, MergeAttemptStartedV1>::new();
    let mut resolved = std::collections::BTreeSet::<String>::new();
    for event in events {
        match event.kind.as_str() {
            "mission_merge_attempt_started_v1" => {
                let attempt = parse_merge_attempt_started(event)?;
                started.insert(attempt.attempt_id.clone(), attempt);
            }
            "mission_merge_attempt_resolved_v1" => {
                let attempt = parse_merge_attempt_resolved(event)?;
                if attempt.resolution.is_terminal() {
                    resolved.insert(attempt.attempt_id);
                }
            }
            _ => {}
        }
    }
    Ok(started
        .into_iter()
        .find(|(attempt_id, attempt)| {
            attempt.binding.scope_sha256 == scope_sha256 && !resolved.contains(attempt_id)
        })
        .map(|(_, attempt)| attempt))
}

fn validate_lease_against_events(
    events: &[JournalEvent],
    scope: &BaseBranchScope,
    issue_number: u64,
    lease: &MissionExecutionLeaseV1,
    expected_holder_id: &str,
    now_unix_ms: u64,
) -> Result<(), AgentRuntimeErrorV1> {
    if lease.schema_version != 1
        || !valid_allocation_id(&lease.allocation_id)
        || !valid_allocation_id(&lease.lease_id)
        || !valid_allocation_id(&lease.holder_id)
        || !valid_sha256(&lease.requirement_sha256)
        || !valid_sha256(&lease.request_sha256)
        || lease.runtime_timeout_ms == 0
        || lease.ttl_ms > lease.runtime_timeout_ms
        || lease.ttl_ms == 0
        || lease
            .expires_at_unix_ms
            .checked_sub(lease.issued_at_unix_ms)
            != Some(lease.ttl_ms)
        || lease.fencing_token == 0
    {
        return Err(error(
            "execution_lease_invalid",
            "reload the versioned lease from the trusted host journal",
            json!({}),
        ));
    }
    if !valid_allocation_id(expected_holder_id) || lease.holder_id != expected_holder_id {
        return Err(error(
            "execution_lease_holder_mismatch",
            "validate this lease only for the holder identity assigned by the trusted host",
            json!({"mission_id":lease.mission_id}),
        ));
    }
    let identity = derive_mission_identity_v1(
        scope,
        issue_number,
        &lease.requirement_sha256,
        lease.execution_generation,
    )?;
    if lease.mission_id != identity.mission_id {
        return Err(error(
            "execution_lease_binding_mismatch",
            "use a lease for this exact issue and protected requirement revision",
            json!({"mission_id":identity.mission_id}),
        ));
    }
    let current_generation = highest_known_generation(events, &lease.mission_id)?;
    if current_generation != lease.execution_generation {
        return Err(error(
            "execution_lease_generation_superseded",
            "discard this lease because a newer host-allocated generation superseded it",
            json!({
                "mission_id":lease.mission_id,
                "lease_generation":lease.execution_generation,
                "current_generation":current_generation,
            }),
        ));
    }
    let allocation =
        allocation_for_generation(events, &lease.mission_id, lease.execution_generation)?
            .ok_or_else(|| {
                error(
                    "execution_allocation_missing",
                    "restore the matching generation allocation before validating this lease",
                    json!({"mission_id":lease.mission_id}),
                )
            })?;
    if allocation.allocation_id != lease.allocation_id
        || allocation.requirement_sha256 != lease.requirement_sha256
    {
        return Err(error(
            "execution_lease_binding_mismatch",
            "use a lease bound to the current allocation and requirement hash",
            json!({"mission_id":lease.mission_id}),
        ));
    }
    if claim_request_sha256(events, &lease.mission_id, lease.execution_generation)?.as_deref()
        != Some(lease.request_sha256.as_str())
    {
        return Err(error(
            "execution_claim_binding_mismatch",
            "restore the exact request claim bound to this lease",
            json!({"mission_id":lease.mission_id}),
        ));
    }
    let mission_history = execution_lease_history(events, &lease.mission_id)?;
    let history = mission_history
        .iter()
        .filter(|entry| entry.execution_generation == lease.execution_generation)
        .collect::<Vec<_>>();
    validate_lease_history_binding(
        &history
            .iter()
            .map(|lease| (*lease).clone())
            .collect::<Vec<_>>(),
        &allocation.allocation_id,
        &allocation.requirement_sha256,
        &lease.request_sha256,
        lease.runtime_timeout_ms,
    )?;
    let current = history.last().ok_or_else(|| {
        error(
            "execution_lease_missing",
            "acquire a lease through the trusted host before validating it",
            json!({"mission_id":lease.mission_id}),
        )
    })?;
    if current.lease_id != lease.lease_id
        || current.holder_id != lease.holder_id
        || current.fencing_token != lease.fencing_token
    {
        return Err(error(
            "execution_lease_fenced",
            "discard this stale worker and acquire a new lease through the host",
            json!({
                "mission_id":lease.mission_id,
                "current_fencing_token":current.fencing_token,
            }),
        ));
    }
    if now_unix_ms < current.issued_at_unix_ms {
        return Err(error(
            "execution_lease_clock_regressed",
            "confirm the host clock before authorizing worker activity",
            json!({"mission_id":lease.mission_id}),
        ));
    }
    let runtime_deadline_unix_ms = history
        .first()
        .and_then(|first| {
            first
                .issued_at_unix_ms
                .checked_add(first.runtime_timeout_ms)
        })
        .ok_or_else(|| invalid_generation_record(&lease.mission_id))?;
    if now_unix_ms >= runtime_deadline_unix_ms {
        return Err(error(
            "execution_lease_deadline_exceeded",
            "the absolute runtime deadline has passed; start a new host-approved generation",
            json!({
                "mission_id":lease.mission_id,
                "execution_generation":lease.execution_generation,
                "runtime_deadline_unix_ms":runtime_deadline_unix_ms,
            }),
        ));
    }
    if now_unix_ms >= current.expires_at_unix_ms {
        return Err(error(
            "execution_lease_expired",
            "checkpoint the work and acquire a new lease before continuing",
            json!({
                "mission_id":lease.mission_id,
                "fencing_token":current.fencing_token,
                "expires_at_unix_ms":current.expires_at_unix_ms,
            }),
        ));
    }
    Ok(())
}

fn map_journal_transaction_error(
    error: JournalTransactionError<AgentRuntimeErrorV1>,
) -> AgentRuntimeErrorV1 {
    match error {
        JournalTransactionError::Journal(error) => journal_error(error),
        JournalTransactionError::Operation(error) => error,
    }
}

/// Fail closed if any reserved mission event in the shared journal is malformed,
/// even when it belongs to a different mission. The file may also contain
/// unrelated webhook events, which are intentionally ignored here.
fn validate_mission_journal(events: &[JournalEvent]) -> Result<(), AgentRuntimeErrorV1> {
    for event in events {
        match event.kind.as_str() {
            "mission_generation_allocated_v1" => {
                parse_generation_allocation(event)?;
            }
            "mission_execution_claim_v1" => {
                parse_claim_generation(event)?;
            }
            "mission_execution_lease_acquired_v1" => {
                parse_execution_lease(event)?;
            }
            "mission_tool_operation_intent_v1" => {
                parse_workspace_write_intent(event)?;
            }
            "mission_tool_operation_result_v1" => {
                parse_workspace_write_result(event)?;
            }
            "mission_verification_cpu_reserved_v1" => {
                parse_verification_cpu_reservation(event)?;
            }
            "mission_verification_cpu_resolved_v1" => {
                parse_verification_cpu_resolution(event)?;
            }
            "mission_merge_attempt_started_v1" => {
                parse_merge_attempt_started(event)?;
            }
            "mission_merge_attempt_resolved_v1" => {
                parse_merge_attempt_resolved(event)?;
            }
            _ => {}
        }
    }
    validate_workspace_write_history(events)?;
    validate_mission_verification_cpu_history(events)?;
    validate_merge_attempt_history(events)?;
    Ok(())
}

#[derive(Default)]
struct WorkspaceWriteOperationLedger {
    intents: std::collections::BTreeMap<String, WorkspaceWriteIntentV1>,
    results: std::collections::BTreeMap<String, WorkspaceWriteResultV1>,
}

fn validate_unresolved_workspace_write(
    ledger: &WorkspaceWriteOperationLedger,
    mission_id: &str,
    execution_generation: u64,
    requested_input: &Value,
    request_id: &Value,
    selected_operation_id: Option<&str>,
    observed_source_sha256: &str,
) -> Result<(), AgentRuntimeErrorV1> {
    if ledger.intents.values().any(|intent| {
        intent.mission_id == mission_id
            && intent.execution_generation == execution_generation
            && &intent.request_id == request_id
            && selected_operation_id != Some(intent.operation_id.as_str())
    }) {
        return Err(error(
            "tool_operation_request_id_conflict",
            "use a new JSON-RPC correlation ID for a different workspace-write input",
            json!({"mission_id":mission_id}),
        ));
    }
    let unresolved = ledger
        .intents
        .values()
        .filter(|intent| {
            intent.mission_id == mission_id
                && intent.execution_generation == execution_generation
                && !ledger.results.contains_key(&intent.operation_id)
        })
        .collect::<Vec<_>>();
    if unresolved.len() > 1 {
        return Err(error(
            "tool_operation_recovery_ambiguous",
            "inspect the candidate and durable write history before replaying an operation with multiple unresolved intents",
            json!({"mission_id":mission_id,"unresolved_operation_ids":unresolved.iter().map(|intent| intent.operation_id.as_str()).collect::<Vec<_>>()}),
        ));
    }
    let Some(pending) = unresolved.first() else {
        return Ok(());
    };
    if selected_operation_id == Some(pending.operation_id.as_str()) {
        return Ok(());
    }
    if &pending.input == requested_input {
        return Err(error(
            "tool_operation_effect_diverged",
            "inspect the candidate against the unresolved durable write intent; do not claim success while the source matches neither its recorded pre-state nor post-state",
            json!({
                "mission_id":mission_id,
                "operation_id":pending.operation_id,
                "observed_source_sha256":observed_source_sha256,
                "expected_before_sha256":pending.before_source_sha256,
                "expected_after_sha256":pending.after_source_sha256,
            }),
        ));
    }
    Err(error(
        "tool_operation_recovery_pending",
        "replay the exact unresolved workspace.write_text input under the current lease before requesting a different write",
        json!({
            "mission_id":mission_id,
            "operation_id":pending.operation_id,
            "observed_source_sha256":observed_source_sha256,
            "expected_before_sha256":pending.before_source_sha256,
            "expected_after_sha256":pending.after_source_sha256,
        }),
    ))
}

fn validate_jsonrpc_request_id(request_id: &Value) -> Result<(), AgentRuntimeErrorV1> {
    let request_id_bytes = serde_json::to_vec(request_id).map_err(|_| {
        error(
            "tool_operation_request_id_invalid",
            "use a bounded JSON-RPC string or integer ID for the workspace write",
            json!({}),
        )
    })?;
    if request_id_bytes.len() > MAX_JSONRPC_REQUEST_ID_BYTES
        || !matches!(request_id, Value::String(value) if !value.trim().is_empty())
            && !matches!(request_id, Value::Number(value) if value.as_i64().is_some() || value.as_u64().is_some())
    {
        return Err(error(
            "tool_operation_request_id_invalid",
            "use a bounded non-empty JSON-RPC string or integer ID for the workspace write",
            json!({"max_request_id_bytes":MAX_JSONRPC_REQUEST_ID_BYTES}),
        ));
    }
    Ok(())
}

fn workspace_write_operation_id(
    mission_id: &str,
    execution_generation: u64,
    input_sha256: &str,
    before_source_sha256: &str,
) -> Result<String, AgentRuntimeErrorV1> {
    if !valid_mission_id_v1(mission_id)
        || execution_generation == 0
        || !valid_sha256(input_sha256)
        || !valid_sha256(before_source_sha256)
    {
        return Err(error(
            "tool_operation_input_invalid",
            "derive the host operation key from an admitted mission, generation, input hash, and source pre-state",
            json!({}),
        ));
    }
    Ok(format!(
        "mission-workspace-write-v1:{mission_id}:{execution_generation}:{}:{}",
        input_sha256.to_ascii_lowercase(),
        before_source_sha256.to_ascii_lowercase()
    ))
}

fn workspace_write_result_key(operation_id: &str) -> String {
    format!("mission-workspace-write-result-v1:{operation_id}")
}

fn validate_workspace_write_input(
    input: &Value,
) -> Result<(String, String, String), AgentRuntimeErrorV1> {
    let Some(object) = input.as_object() else {
        return Err(invalid_workspace_write_input());
    };
    if object.len() != 2 || !object.contains_key("path") || !object.contains_key("content") {
        return Err(invalid_workspace_write_input());
    }
    let Some(path) = object.get("path").and_then(Value::as_str) else {
        return Err(invalid_workspace_write_input());
    };
    let Some(content) = object.get("content").and_then(Value::as_str) else {
        return Err(invalid_workspace_write_input());
    };
    if path != "src/lib.rs" || content.len() > MAX_WORKSPACE_WRITE_CONTENT_BYTES {
        return Err(error(
            "tool_operation_input_invalid",
            "write only src/lib.rs and keep the exact UTF-8 content within the host's 64 KiB bound",
            json!({"path":path,"max_content_bytes":MAX_WORKSPACE_WRITE_CONTENT_BYTES}),
        ));
    }
    let input_bytes = serde_json::to_vec(input).map_err(|_| invalid_workspace_write_input())?;
    if input_bytes.len()
        > MAX_WORKSPACE_WRITE_CONTENT_BYTES
            .saturating_mul(6)
            .saturating_add(64)
    {
        return Err(invalid_workspace_write_input());
    }
    Ok((
        path.to_owned(),
        sha256(&input_bytes),
        sha256(content.as_bytes()),
    ))
}

fn invalid_workspace_write_input() -> AgentRuntimeErrorV1 {
    error(
        "tool_operation_input_invalid",
        "provide exactly {path, content} for the bounded src/lib.rs write",
        json!({"max_content_bytes":MAX_WORKSPACE_WRITE_CONTENT_BYTES}),
    )
}

fn valid_mcp_success_result_body(body: &Value) -> bool {
    body.is_object()
        && body.get("content").is_some_and(Value::is_array)
        && body.get("isError") == Some(&Value::Bool(false))
}

fn parse_workspace_write_intent(
    event: &JournalEvent,
) -> Result<WorkspaceWriteIntentV1, AgentRuntimeErrorV1> {
    let record: WorkspaceWriteIntentV1 = serde_json::from_value(event.payload.clone())
        .map_err(|_| invalid_tool_operation_record("unknown"))?;
    let (path, input_sha256, after_source_sha256) =
        validate_workspace_write_input(&record.input)
            .map_err(|_| invalid_tool_operation_record(&record.mission_id))?;
    validate_jsonrpc_request_id(&record.request_id)
        .map_err(|_| invalid_tool_operation_record(&record.mission_id))?;
    let operation_id = workspace_write_operation_id(
        &record.mission_id,
        record.execution_generation,
        &input_sha256,
        &record.before_source_sha256,
    )
    .map_err(|_| invalid_tool_operation_record(&record.mission_id))?;
    if event.kind != "mission_tool_operation_intent_v1"
        || event.idempotency_key != operation_id
        || record.schema_version != 1
        || !valid_mission_id_v1(&record.mission_id)
        || record.execution_generation == 0
        || record.operation_id != operation_id
        || !valid_allocation_id(&record.allocation_id)
        || !valid_sha256(&record.request_sha256)
        || !valid_allocation_id(&record.lease_id)
        || !valid_allocation_id(&record.holder_id)
        || record.fencing_token == 0
        || record.tool_id != "workspace.write_text"
        || record.path != path
        || record.input_sha256 != input_sha256
        || !valid_sha256(&record.before_source_sha256)
        || record.before_source_sha256 != record.before_source_sha256.to_ascii_lowercase()
        || record.after_source_sha256 != after_source_sha256
    {
        return Err(invalid_tool_operation_record(&record.mission_id));
    }
    Ok(record)
}

fn parse_workspace_write_result(
    event: &JournalEvent,
) -> Result<WorkspaceWriteResultV1, AgentRuntimeErrorV1> {
    let record: WorkspaceWriteResultV1 = serde_json::from_value(event.payload.clone())
        .map_err(|_| invalid_tool_operation_record("unknown"))?;
    validate_jsonrpc_request_id(&record.request_id)
        .map_err(|_| invalid_tool_operation_record(&record.mission_id))?;
    let operation_id = workspace_write_operation_id(
        &record.mission_id,
        record.execution_generation,
        &record.input_sha256,
        &record.before_source_sha256,
    )
    .map_err(|_| invalid_tool_operation_record(&record.mission_id))?;
    let result_bytes = serde_json::to_vec(&record.result_body)
        .map_err(|_| invalid_tool_operation_record(&record.mission_id))?;
    if event.kind != "mission_tool_operation_result_v1"
        || event.idempotency_key != workspace_write_result_key(&operation_id)
        || record.schema_version != 1
        || !valid_mission_id_v1(&record.mission_id)
        || record.execution_generation == 0
        || record.operation_id != operation_id
        || !valid_allocation_id(&record.allocation_id)
        || !valid_sha256(&record.request_sha256)
        || !valid_sha256(&record.input_sha256)
        || record.input_sha256 != record.input_sha256.to_ascii_lowercase()
        || !valid_sha256(&record.before_source_sha256)
        || record.before_source_sha256 != record.before_source_sha256.to_ascii_lowercase()
        || !valid_sha256(&record.observed_source_sha256)
        || record.observed_source_sha256 != record.observed_source_sha256.to_ascii_lowercase()
        || !valid_allocation_id(&record.lease_id)
        || !valid_allocation_id(&record.holder_id)
        || record.fencing_token == 0
        || result_bytes.len() > MAX_WORKSPACE_WRITE_RESPONSE_BYTES
        || record.result_sha256 != sha256(&result_bytes)
        || !valid_mcp_success_result_body(&record.result_body)
    {
        return Err(invalid_tool_operation_record(&record.mission_id));
    }
    Ok(record)
}

fn workspace_write_lease_key(
    mission_id: &str,
    execution_generation: u64,
    allocation_id: &str,
    request_sha256: &str,
    lease_id: &str,
    holder_id: &str,
    fencing_token: u64,
) -> (String, u64, String, String, String, String, u64) {
    (
        mission_id.to_owned(),
        execution_generation,
        allocation_id.to_owned(),
        request_sha256.to_owned(),
        lease_id.to_owned(),
        holder_id.to_owned(),
        fencing_token,
    )
}

fn workspace_write_operation_ledger(
    events: &[JournalEvent],
) -> Result<WorkspaceWriteOperationLedger, AgentRuntimeErrorV1> {
    let mut ledger = WorkspaceWriteOperationLedger::default();
    let mut leases = std::collections::BTreeSet::new();
    let mut request_ids = std::collections::BTreeMap::new();
    for event in events {
        match event.kind.as_str() {
            "mission_execution_lease_acquired_v1" => {
                let lease = parse_execution_lease(event)?;
                leases.insert(workspace_write_lease_key(
                    &lease.mission_id,
                    lease.execution_generation,
                    &lease.allocation_id,
                    &lease.request_sha256,
                    &lease.lease_id,
                    &lease.holder_id,
                    lease.fencing_token,
                ));
            }
            "mission_tool_operation_intent_v1" => {
                let intent = parse_workspace_write_intent(event)?;
                let lease_key = workspace_write_lease_key(
                    &intent.mission_id,
                    intent.execution_generation,
                    &intent.allocation_id,
                    &intent.request_sha256,
                    &intent.lease_id,
                    &intent.holder_id,
                    intent.fencing_token,
                );
                if !leases.contains(&lease_key)
                    || ledger
                        .intents
                        .insert(intent.operation_id.clone(), intent.clone())
                        .is_some()
                {
                    return Err(invalid_tool_operation_record(&intent.mission_id));
                }
                let request_id_key = (
                    intent.mission_id.clone(),
                    intent.execution_generation,
                    serde_json::to_string(&intent.request_id)
                        .map_err(|_| invalid_tool_operation_record(&intent.mission_id))?,
                );
                if request_ids
                    .insert(request_id_key, intent.operation_id.clone())
                    .is_some_and(|existing| existing != intent.operation_id)
                {
                    return Err(invalid_tool_operation_record(&intent.mission_id));
                }
            }
            "mission_tool_operation_result_v1" => {
                let result = parse_workspace_write_result(event)?;
                let Some(intent) = ledger.intents.get(&result.operation_id) else {
                    return Err(invalid_tool_operation_record(&result.mission_id));
                };
                let lease_key = workspace_write_lease_key(
                    &result.mission_id,
                    result.execution_generation,
                    &result.allocation_id,
                    &result.request_sha256,
                    &result.lease_id,
                    &result.holder_id,
                    result.fencing_token,
                );
                if !leases.contains(&lease_key)
                    || result.mission_id != intent.mission_id
                    || result.execution_generation != intent.execution_generation
                    || result.request_id != intent.request_id
                    || result.allocation_id != intent.allocation_id
                    || result.request_sha256 != intent.request_sha256
                    || result.input_sha256 != intent.input_sha256
                    || result.before_source_sha256 != intent.before_source_sha256
                    || result.observed_source_sha256 != intent.after_source_sha256
                    || result.fencing_token < intent.fencing_token
                    || ledger
                        .results
                        .insert(result.operation_id.clone(), result.clone())
                        .is_some()
                {
                    return Err(invalid_tool_operation_record(&result.mission_id));
                }
            }
            _ => {}
        }
    }
    Ok(ledger)
}

fn validate_workspace_write_history(events: &[JournalEvent]) -> Result<(), AgentRuntimeErrorV1> {
    workspace_write_operation_ledger(events).map(|_| ())
}

fn invalid_tool_operation_record(mission_id: &str) -> AgentRuntimeErrorV1 {
    error(
        "tool_operation_journal_invalid",
        "restore a verified mission journal before replaying or authorizing a workspace write",
        json!({"mission_id":mission_id}),
    )
}

#[derive(Default)]
struct MissionVerificationCpuLedger {
    policy: Option<(String, u64)>,
    reservations: Vec<MissionVerificationCpuReservationRecordV1>,
    resolutions: Vec<MissionVerificationCpuResolutionRecordV1>,
    charged_cpu_ms: u64,
}

fn verification_cpu_reservation_key(mission_id: &str, reservation_id: &str) -> String {
    format!("mission-verification-cpu-reserved-v1:{mission_id}:{reservation_id}")
}

fn verification_cpu_resolution_key(mission_id: &str, reservation_id: &str) -> String {
    format!("mission-verification-cpu-resolved-v1:{mission_id}:{reservation_id}")
}

fn parse_verification_cpu_reservation(
    event: &JournalEvent,
) -> Result<MissionVerificationCpuReservationRecordV1, AgentRuntimeErrorV1> {
    let record: MissionVerificationCpuReservationRecordV1 =
        serde_json::from_value(event.payload.clone()).map_err(|_| {
            invalid_verification_cpu_record(
                event
                    .payload
                    .get("mission_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
            )
        })?;
    if event.kind != "mission_verification_cpu_reserved_v1"
        || event.idempotency_key
            != verification_cpu_reservation_key(&record.mission_id, &record.reservation_id)
        || record.schema_version != 1
        || !valid_mission_id_v1(&record.mission_id)
        || record.execution_generation == 0
        || !valid_allocation_id(&record.lease_id)
        || !valid_allocation_id(&record.holder_id)
        || record.fencing_token == 0
        || !valid_sha256(&record.request_sha256)
        || !valid_allocation_id(&record.reservation_id)
        || !valid_sha256(&record.policy_sha256)
        || record.mission_cpu_budget_ms == 0
        || record.reserved_cpu_ms == 0
        || record.reserved_cpu_ms > record.mission_cpu_budget_ms
    {
        return Err(invalid_verification_cpu_record(&record.mission_id));
    }
    Ok(record)
}

fn parse_verification_cpu_resolution(
    event: &JournalEvent,
) -> Result<MissionVerificationCpuResolutionRecordV1, AgentRuntimeErrorV1> {
    let record: MissionVerificationCpuResolutionRecordV1 =
        serde_json::from_value(event.payload.clone()).map_err(|_| {
            invalid_verification_cpu_record(
                event
                    .payload
                    .get("mission_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
            )
        })?;
    if event.kind != "mission_verification_cpu_resolved_v1"
        || event.idempotency_key
            != verification_cpu_resolution_key(&record.mission_id, &record.reservation_id)
        || record.schema_version != 1
        || !valid_mission_id_v1(&record.mission_id)
        || record.execution_generation == 0
        || !valid_allocation_id(&record.lease_id)
        || record.fencing_token == 0
        || !valid_allocation_id(&record.reservation_id)
        || !valid_sha256(&record.policy_sha256)
        || record.charged_cpu_ms == 0
    {
        return Err(invalid_verification_cpu_record(&record.mission_id));
    }
    Ok(record)
}

fn mission_verification_cpu_ledger(
    events: &[JournalEvent],
    mission_id: &str,
) -> Result<MissionVerificationCpuLedger, AgentRuntimeErrorV1> {
    let mut ledger = MissionVerificationCpuLedger::default();
    let mut resolutions = std::collections::BTreeMap::new();
    for event in events {
        match event.kind.as_str() {
            "mission_verification_cpu_reserved_v1" => {
                let reservation = parse_verification_cpu_reservation(event)?;
                if reservation.mission_id == mission_id {
                    ledger.policy.get_or_insert_with(|| {
                        (
                            reservation.policy_sha256.clone(),
                            reservation.mission_cpu_budget_ms,
                        )
                    });
                    ledger.reservations.push(reservation);
                }
            }
            "mission_verification_cpu_resolved_v1" => {
                let resolution = parse_verification_cpu_resolution(event)?;
                if resolution.mission_id == mission_id {
                    resolutions.insert(resolution.reservation_id.clone(), resolution.clone());
                    ledger.resolutions.push(resolution);
                }
            }
            _ => {}
        }
    }
    for reservation in &ledger.reservations {
        let charge = resolutions
            .get(&reservation.reservation_id)
            .map(|resolution| resolution.charged_cpu_ms)
            .unwrap_or(reservation.reserved_cpu_ms);
        ledger.charged_cpu_ms = ledger
            .charged_cpu_ms
            .checked_add(charge)
            .ok_or_else(|| invalid_verification_cpu_record(mission_id))?;
    }
    Ok(ledger)
}

fn validate_mission_verification_cpu_history(
    events: &[JournalEvent],
) -> Result<(), AgentRuntimeErrorV1> {
    let mut reservations = std::collections::BTreeMap::new();
    let mut resolutions = std::collections::BTreeMap::new();
    let mut policies = std::collections::BTreeMap::new();
    let mut leases = std::collections::BTreeSet::new();

    for event in events {
        match event.kind.as_str() {
            "mission_execution_lease_acquired_v1" => {
                let lease = parse_execution_lease(event)?;
                leases.insert((
                    lease.mission_id,
                    lease.execution_generation,
                    lease.lease_id,
                    lease.holder_id,
                    lease.fencing_token,
                    lease.request_sha256,
                ));
            }
            "mission_verification_cpu_reserved_v1" => {
                let reservation = parse_verification_cpu_reservation(event)?;
                let lease_key = (
                    reservation.mission_id.clone(),
                    reservation.execution_generation,
                    reservation.lease_id.clone(),
                    reservation.holder_id.clone(),
                    reservation.fencing_token,
                    reservation.request_sha256.clone(),
                );
                if !leases.contains(&lease_key) {
                    return Err(invalid_verification_cpu_record(&reservation.mission_id));
                }
                let policy = policies
                    .entry(reservation.mission_id.clone())
                    .or_insert_with(|| {
                        (
                            reservation.policy_sha256.clone(),
                            reservation.mission_cpu_budget_ms,
                        )
                    });
                if policy.0 != reservation.policy_sha256
                    || policy.1 != reservation.mission_cpu_budget_ms
                    || reservations
                        .insert(
                            (
                                reservation.mission_id.clone(),
                                reservation.reservation_id.clone(),
                            ),
                            reservation.clone(),
                        )
                        .is_some()
                {
                    return Err(invalid_verification_cpu_record(&reservation.mission_id));
                }
            }
            "mission_verification_cpu_resolved_v1" => {
                let resolution = parse_verification_cpu_resolution(event)?;
                let key = (
                    resolution.mission_id.clone(),
                    resolution.reservation_id.clone(),
                );
                let Some(reservation) = reservations.get(&key) else {
                    return Err(invalid_verification_cpu_record(&resolution.mission_id));
                };
                if resolution.execution_generation != reservation.execution_generation
                    || resolution.lease_id != reservation.lease_id
                    || resolution.fencing_token != reservation.fencing_token
                    || resolution.policy_sha256 != reservation.policy_sha256
                    || resolution.charged_cpu_ms > reservation.reserved_cpu_ms
                    || resolutions.insert(key, resolution.clone()).is_some()
                {
                    return Err(invalid_verification_cpu_record(&resolution.mission_id));
                }
            }
            _ => {}
        }
    }

    let mut charged_by_mission = std::collections::BTreeMap::new();
    for ((mission_id, reservation_id), reservation) in reservations {
        let charged = resolutions
            .get(&(mission_id.clone(), reservation_id))
            .map(|resolution| resolution.charged_cpu_ms)
            .unwrap_or(reservation.reserved_cpu_ms);
        let total = charged_by_mission
            .entry(mission_id.clone())
            .or_insert(0_u64);
        *total = total
            .checked_add(charged)
            .ok_or_else(|| invalid_verification_cpu_record(&mission_id))?;
        let limit = policies
            .get(&mission_id)
            .map(|(_, limit)| *limit)
            .ok_or_else(|| invalid_verification_cpu_record(&mission_id))?;
        if *total > limit {
            return Err(invalid_verification_cpu_record(&mission_id));
        }
    }
    Ok(())
}

fn invalid_verification_cpu_record(mission_id: &str) -> AgentRuntimeErrorV1 {
    error(
        "mission_verification_cpu_journal_invalid",
        "restore a verified mission journal snapshot before running or authorizing verifier work",
        json!({"mission_id":mission_id}),
    )
}

fn allocation_for_request(
    events: &[JournalEvent],
    mission_id: &str,
    allocation_id: &str,
) -> Result<Option<MissionGenerationAllocationV1>, AgentRuntimeErrorV1> {
    let mut found = None;
    for event in events.iter().filter(|event| {
        event.kind == "mission_generation_allocated_v1"
            && event.payload.get("mission_id").and_then(Value::as_str) == Some(mission_id)
            && event.payload.get("allocation_id").and_then(Value::as_str) == Some(allocation_id)
    }) {
        let allocation = parse_generation_allocation(event)?;
        if found.is_some() {
            return Err(error(
                "execution_generation_journal_invalid",
                "restore a verified journal snapshot before admitting more executions",
                json!({"mission_id":mission_id,"allocation_id":allocation_id}),
            ));
        }
        found = Some(allocation);
    }
    Ok(found)
}

fn highest_known_generation(
    events: &[JournalEvent],
    mission_id: &str,
) -> Result<u64, AgentRuntimeErrorV1> {
    let mut highest = 0;
    for event in events {
        match event.kind.as_str() {
            "mission_generation_allocated_v1"
                if event.payload.get("mission_id").and_then(Value::as_str) == Some(mission_id) =>
            {
                highest = highest.max(parse_generation_allocation(event)?.execution_generation);
            }
            "mission_execution_claim_v1"
                if event.payload.get("mission_id").and_then(Value::as_str) == Some(mission_id) =>
            {
                highest = highest.max(parse_claim_generation(event)?);
            }
            "mission_execution_lease_acquired_v1" => {
                let lease = parse_execution_lease(event)?;
                if lease.mission_id == mission_id {
                    highest = highest.max(lease.execution_generation);
                }
            }
            _ => {}
        }
    }
    Ok(highest)
}

fn allocation_for_generation(
    events: &[JournalEvent],
    mission_id: &str,
    execution_generation: u64,
) -> Result<Option<MissionGenerationAllocationV1>, AgentRuntimeErrorV1> {
    let mut found = None;
    for event in events.iter().filter(|event| {
        event.kind == "mission_generation_allocated_v1"
            && event.payload.get("mission_id").and_then(Value::as_str) == Some(mission_id)
    }) {
        let allocation = parse_generation_allocation(event)?;
        if allocation.execution_generation == execution_generation {
            if found.is_some() {
                return Err(invalid_generation_record(mission_id));
            }
            found = Some(allocation);
        }
    }
    Ok(found)
}

fn lease_idempotency_key(mission_id: &str, generation: u64, lease_id: &str) -> String {
    format!("mission-execution-lease-v1:{mission_id}:{generation}:{lease_id}")
}

fn execution_lease_payload(lease: &MissionExecutionLeaseV1) -> Value {
    json!({
        "schema_version": lease.schema_version,
        "mission_id": lease.mission_id,
        "execution_generation": lease.execution_generation,
        "allocation_id": lease.allocation_id,
        "lease_id": lease.lease_id,
        "holder_id": lease.holder_id,
        "requirement_sha256": lease.requirement_sha256,
        "request_sha256": lease.request_sha256,
        "runtime_timeout_ms": lease.runtime_timeout_ms,
        "fencing_token": lease.fencing_token,
        "issued_at_unix_ms": lease.issued_at_unix_ms,
        "expires_at_unix_ms": lease.expires_at_unix_ms,
        "ttl_ms": lease.ttl_ms,
    })
}

fn execution_lease_history(
    events: &[JournalEvent],
    mission_id: &str,
) -> Result<Vec<MissionExecutionLeaseV1>, AgentRuntimeErrorV1> {
    let mut history = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.kind == "mission_execution_lease_acquired_v1")
    {
        let lease = parse_execution_lease(event)?;
        if lease.mission_id == mission_id {
            let expected_fence = history
                .last()
                .map(|previous: &MissionExecutionLeaseV1| previous.fencing_token)
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| invalid_generation_record(mission_id))?;
            if lease.fencing_token != expected_fence {
                return Err(invalid_generation_record(mission_id));
            }
            history.push(lease);
        }
    }
    Ok(history)
}

fn validate_lease_history_binding(
    history: &[MissionExecutionLeaseV1],
    allocation_id: &str,
    requirement_sha256: &str,
    request_sha256: &str,
    runtime_timeout_ms: u64,
) -> Result<(), AgentRuntimeErrorV1> {
    if history.iter().any(|lease| {
        lease.allocation_id != allocation_id
            || lease.requirement_sha256 != requirement_sha256
            || lease.request_sha256 != request_sha256
            || lease.runtime_timeout_ms != runtime_timeout_ms
    }) {
        let mission_id = history
            .last()
            .map(|lease| lease.mission_id.as_str())
            .unwrap_or("unknown");
        return Err(invalid_generation_record(mission_id));
    }
    Ok(())
}

fn parse_execution_lease(
    event: &JournalEvent,
) -> Result<MissionExecutionLeaseV1, AgentRuntimeErrorV1> {
    let schema_version = event
        .payload
        .get("schema_version")
        .and_then(Value::as_u64)
        .filter(|version| *version == 1)
        .ok_or_else(|| invalid_generation_record("unknown"))? as u32;
    let mission_id = event
        .payload
        .get("mission_id")
        .and_then(Value::as_str)
        .filter(|value| value.strip_prefix("mission-v1-").is_some_and(valid_sha256))
        .ok_or_else(|| invalid_generation_record("unknown"))?
        .to_owned();
    let execution_generation = event
        .payload
        .get("execution_generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| invalid_generation_record(&mission_id))?;
    let allocation_id = event
        .payload
        .get("allocation_id")
        .and_then(Value::as_str)
        .filter(|value| valid_allocation_id(value))
        .ok_or_else(|| invalid_generation_record(&mission_id))?
        .to_owned();
    let lease_id = event
        .payload
        .get("lease_id")
        .and_then(Value::as_str)
        .filter(|value| valid_allocation_id(value))
        .ok_or_else(|| invalid_generation_record(&mission_id))?
        .to_owned();
    let holder_id = event
        .payload
        .get("holder_id")
        .and_then(Value::as_str)
        .filter(|value| valid_allocation_id(value))
        .ok_or_else(|| invalid_generation_record(&mission_id))?
        .to_owned();
    let requirement_sha256 = event
        .payload
        .get("requirement_sha256")
        .and_then(Value::as_str)
        .filter(|value| valid_sha256(value))
        .ok_or_else(|| invalid_generation_record(&mission_id))?
        .to_ascii_lowercase();
    let request_sha256 = event
        .payload
        .get("request_sha256")
        .and_then(Value::as_str)
        .filter(|value| valid_sha256(value))
        .ok_or_else(|| invalid_generation_record(&mission_id))?
        .to_ascii_lowercase();
    let runtime_timeout_ms = event
        .payload
        .get("runtime_timeout_ms")
        .and_then(Value::as_u64)
        .filter(|timeout| *timeout > 0)
        .ok_or_else(|| invalid_generation_record(&mission_id))?;
    let fencing_token = event
        .payload
        .get("fencing_token")
        .and_then(Value::as_u64)
        .filter(|token| *token > 0)
        .ok_or_else(|| invalid_generation_record(&mission_id))?;
    let issued_at_unix_ms = event
        .payload
        .get("issued_at_unix_ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_generation_record(&mission_id))?;
    let expires_at_unix_ms = event
        .payload
        .get("expires_at_unix_ms")
        .and_then(Value::as_u64)
        .filter(|expires| *expires > issued_at_unix_ms)
        .ok_or_else(|| invalid_generation_record(&mission_id))?;
    let ttl_ms = event
        .payload
        .get("ttl_ms")
        .and_then(Value::as_u64)
        .filter(|ttl| *ttl > 0 && issued_at_unix_ms.checked_add(*ttl) == Some(expires_at_unix_ms))
        .ok_or_else(|| invalid_generation_record(&mission_id))?;
    if event.idempotency_key != lease_idempotency_key(&mission_id, execution_generation, &lease_id)
    {
        return Err(invalid_generation_record(&mission_id));
    }
    Ok(MissionExecutionLeaseV1 {
        schema_version,
        mission_id,
        execution_generation,
        allocation_id,
        lease_id,
        holder_id,
        requirement_sha256,
        request_sha256,
        runtime_timeout_ms,
        fencing_token,
        issued_at_unix_ms,
        expires_at_unix_ms,
        ttl_ms,
        replayed: false,
    })
}

fn claim_request_sha256(
    events: &[JournalEvent],
    mission_id: &str,
    execution_generation: u64,
) -> Result<Option<String>, AgentRuntimeErrorV1> {
    let mut found = None;
    for event in events.iter().filter(|event| {
        event.kind == "mission_execution_claim_v1"
            && event.payload.get("mission_id").and_then(Value::as_str) == Some(mission_id)
    }) {
        let generation = parse_claim_generation(event)?;
        if generation == execution_generation {
            if found.is_some() {
                return Err(invalid_generation_record(mission_id));
            }
            found = Some(
                event
                    .payload
                    .get("request_sha256")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid_generation_record(mission_id))?
                    .to_owned(),
            );
        }
    }
    Ok(found)
}

fn parse_claim_generation(event: &JournalEvent) -> Result<u64, AgentRuntimeErrorV1> {
    let mission_id = event
        .payload
        .get("mission_id")
        .and_then(Value::as_str)
        .filter(|value| value.strip_prefix("mission-v1-").is_some_and(valid_sha256))
        .ok_or_else(|| invalid_generation_record("unknown"))?;
    if event.payload.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(invalid_generation_record(mission_id));
    }
    let generation = event
        .payload
        .get("execution_generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| invalid_generation_record(mission_id))?;
    if event
        .payload
        .get("request_sha256")
        .and_then(Value::as_str)
        .is_none_or(|value| !valid_sha256(value))
    {
        return Err(invalid_generation_record(mission_id));
    }
    let digest = mission_id
        .strip_prefix("mission-v1-")
        .expect("mission identity shape validated");
    let expected_key = format!("mission-execution-claim-v1:execution-v1-{digest}-{generation}");
    if event.idempotency_key != expected_key {
        return Err(invalid_generation_record(mission_id));
    }
    Ok(generation)
}

fn parse_generation_allocation(
    event: &JournalEvent,
) -> Result<MissionGenerationAllocationV1, AgentRuntimeErrorV1> {
    let schema_version = event
        .payload
        .get("schema_version")
        .and_then(Value::as_u64)
        .filter(|version| *version == 1)
        .ok_or_else(|| invalid_generation_record("unknown"))? as u32;
    let mission_id = event
        .payload
        .get("mission_id")
        .and_then(Value::as_str)
        .filter(|value| value.strip_prefix("mission-v1-").is_some_and(valid_sha256))
        .ok_or_else(|| invalid_generation_record("unknown"))?
        .to_owned();
    let allocation_id = event
        .payload
        .get("allocation_id")
        .and_then(Value::as_str)
        .filter(|value| valid_allocation_id(value))
        .ok_or_else(|| invalid_generation_record(&mission_id))?
        .to_owned();
    let requirement_sha256 = event
        .payload
        .get("requirement_sha256")
        .and_then(Value::as_str)
        .filter(|value| valid_sha256(value))
        .ok_or_else(|| invalid_generation_record(&mission_id))?
        .to_ascii_lowercase();
    let execution_generation = event
        .payload
        .get("execution_generation")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| invalid_generation_record(&mission_id))?;
    let expected_key = format!("mission-generation-v1:{mission_id}:{execution_generation}");
    if event.idempotency_key != expected_key {
        return Err(invalid_generation_record(&mission_id));
    }
    Ok(MissionGenerationAllocationV1 {
        schema_version,
        mission_id,
        allocation_id,
        requirement_sha256,
        execution_generation,
        replayed: false,
    })
}

fn invalid_generation_record(mission_id: &str) -> AgentRuntimeErrorV1 {
    error(
        "execution_generation_journal_invalid",
        "restore a verified journal snapshot before admitting more executions",
        json!({"mission_id":mission_id}),
    )
}

fn valid_allocation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn journal_error(error: JournalError) -> AgentRuntimeErrorV1 {
    match error {
        JournalError::Io(_) => AgentRuntimeErrorV1::new(
            "execution_journal_io_failure",
            false,
            "inspect the private journal path and storage; retry only after an operator confirms the cause is transient",
            json!({}),
        ),
        JournalError::Corrupt { line } => AgentRuntimeErrorV1::new(
            "execution_journal_corrupt",
            false,
            "restore a verified journal snapshot before admitting more executions",
            json!({"line":line}),
        ),
        JournalError::InboxTooLarge { maximum_bytes } => AgentRuntimeErrorV1::new(
            "execution_journal_capacity_exceeded",
            false,
            "preserve a backup and migrate the verified journal before admitting more executions",
            json!({"maximum_bytes":maximum_bytes}),
        ),
        JournalError::IdempotencyConflict => AgentRuntimeErrorV1::new(
            "execution_idempotency_conflict",
            false,
            "keep the same request for this generation or use a new host-approved generation",
            json!({}),
        ),
        JournalError::LockBusy => AgentRuntimeErrorV1::new(
            "execution_mission_busy",
            true,
            "retry after the active mission or journal transaction releases its lock",
            json!({}),
        ),
        JournalError::InvalidEvent => AgentRuntimeErrorV1::new(
            "execution_claim_invalid",
            false,
            "inspect the claim identity and journal event schema",
            json!({}),
        ),
    }
}

/// Derive a stable mission ID for one Forgejo issue and requirement revision.
/// The supplied requirement hash must already have been loaded from a trusted,
/// protected mission record; this function validates its shape, not provenance.
pub fn derive_mission_identity_v1(
    scope: &BaseBranchScope,
    issue_number: u64,
    requirement_sha256: &str,
    execution_generation: u64,
) -> Result<MissionIdentityV1, AgentRuntimeErrorV1> {
    if issue_number == 0 {
        return Err(error(
            "mission_source_invalid",
            "provide a positive Forgejo issue number",
            json!({"issue_number":issue_number}),
        ));
    }
    if !valid_sha256(requirement_sha256) {
        return Err(error(
            "requirement_hash_invalid",
            "provide the SHA-256 of the protected requirement revision",
            json!({}),
        ));
    }
    if execution_generation == 0 {
        return Err(error(
            "execution_generation_invalid",
            "use a positive host-assigned execution generation",
            json!({"execution_generation":execution_generation}),
        ));
    }

    let issue_number_bytes = issue_number.to_be_bytes();
    let requirement_hash = requirement_sha256.to_ascii_lowercase();
    let mission_digest = sha256_fields(&[
        b"agent-forge-mission-v1",
        scope.origin().as_bytes(),
        scope.owner().as_bytes(),
        scope.repository().as_bytes(),
        b"issue",
        &issue_number_bytes,
        requirement_hash.as_bytes(),
    ]);
    Ok(MissionIdentityV1 {
        schema_version: 1,
        mission_id: format!("mission-v1-{mission_digest}"),
        idempotency_key: format!("execution-v1-{mission_digest}-{execution_generation}"),
        execution_generation,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextPacketV1 {
    pub manifest: ContextManifest,
    /// Exact context text delivered to the runtime; `citations` cover the complete string.
    pub text: String,
    /// UTF-8 byte ranges in `text`, each linked to one manifest source.
    pub citations: Vec<ContextCitationV1>,
}

impl ContextPacketV1 {
    /// Generic estimate used when no provider tokenizer is available. This
    /// byte-based heuristic is not a provider-exact token count or a hard
    /// provider-side limit.
    pub fn estimate_tokens_for_text(text: &str) -> u64 {
        text.len().div_ceil(4) as u64
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextCitationV1 {
    /// Inclusive UTF-8 byte offset into `ContextPacketV1::text`.
    pub text_start_byte: usize,
    /// Exclusive UTF-8 byte offset into `ContextPacketV1::text`.
    pub text_end_byte: usize,
    /// Index into `ContextPacketV1::manifest.sources`.
    pub source_index: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceRefV1 {
    pub id: String,
    pub base_commit: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeBudgetV1 {
    pub max_tool_calls: u32,
    pub timeout_ms: u64,
    /// Requested limit. A returned usage claim alone cannot prove the provider enforced it.
    pub max_output_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilitiesV1 {
    pub read_paths: Vec<String>,
    pub write_paths: Vec<String>,
    pub tools: Vec<String>,
    pub budget: RuntimeBudgetV1,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentExecutionRequestV1 {
    pub schema_version: u32,
    pub mission_id: String,
    /// Stable for a single logical execution. Reuse with different request bytes is a conflict.
    pub idempotency_key: String,
    pub requirement_sha256: String,
    /// Exact UTF-8 JSON bytes of the protected, versioned requirements document.
    pub requirements_json: String,
    /// Current issue text, labels, and milestone from the same API read used
    /// for admission. This is optional context only and never selects or
    /// changes `requirements_json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untrusted_work_item: Option<UntrustedWorkItemContentV1>,
    /// Host-evaluated requirements-review provenance. Required by schema-v3
    /// requirements and absent for legacy schema-v2 requests. This is audit
    /// data carried by the trusted host, not independent proof by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement_arbitration: Option<crate::forgejo::RequirementArbitrationDecisionV1>,
    pub context: ContextPacketV1,
    pub workspace: WorkspaceRefV1,
    pub capabilities: RuntimeCapabilitiesV1,
}

pub const MAX_UNTRUSTED_WORK_ITEM_TITLE_BYTES: usize = 1_024;
/// A stricter runtime bound than the reconcile projection's storage bound keeps
/// request text inside the context budget even when the source issue is large.
pub const MAX_UNTRUSTED_WORK_ITEM_BODY_BYTES: usize = 4_096;
pub const MAX_UNTRUSTED_WORK_ITEM_LABELS: usize = 256;
pub const MAX_UNTRUSTED_WORK_ITEM_LABEL_NAME_BYTES: usize = MAX_FORGEJO_LABEL_NAME_BYTES;
pub const MAX_UNTRUSTED_WORK_ITEM_MILESTONE_TITLE_BYTES: usize = 1_024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UntrustedWorkItemLabelV1 {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UntrustedWorkItemMilestoneStateV1 {
    Open,
    Closed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UntrustedWorkItemMilestoneValueV1 {
    pub id: u64,
    pub title: String,
    pub state: UntrustedWorkItemMilestoneStateV1,
}

/// Milestone state read from the current issue API. `Unknown` means the API
/// omitted the field; `Unassigned` means it explicitly returned JSON null.
/// The enclosing work-item source and trust fields also apply to this value.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "observation",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum UntrustedWorkItemMilestoneV1 {
    Unknown,
    Unassigned,
    Assigned(UntrustedWorkItemMilestoneValueV1),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    rename_all = "snake_case",
    tag = "kind",
    content = "text",
    deny_unknown_fields
)]
pub enum UntrustedWorkItemBodyV1 {
    ExplicitNull,
    Text(String),
}

/// User-controlled issue context bound to the current Forgejo issue snapshot.
/// `content_sha256` records the exact structured value; it does not attest
/// correctness, authorship, or atomicity with later reads.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UntrustedWorkItemContentV1 {
    pub schema_version: u32,
    pub source: String,
    pub trust: String,
    pub origin: String,
    pub owner: String,
    pub repository: String,
    pub issue_number: u64,
    pub source_url: String,
    pub updated_at: String,
    pub title: String,
    pub body: UntrustedWorkItemBodyV1,
    /// Present only when the current issue API supplied a known label set.
    /// An empty vector means the set is known to be empty; omission is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<UntrustedWorkItemLabelV1>>,
    /// Present only in schema 3; it preserves absent versus explicit-null API
    /// observations and contains only the bounded current milestone projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<UntrustedWorkItemMilestoneV1>,
    pub content_sha256: String,
}

impl UntrustedWorkItemContentV1 {
    pub fn new(
        scope: &BaseBranchScope,
        issue_number: u64,
        updated_at: impl Into<String>,
        title: impl Into<String>,
        body: UntrustedWorkItemBodyV1,
    ) -> Self {
        let mut content = Self {
            schema_version: 1,
            source: "current_forgejo_issue_api".to_owned(),
            trust: "untrusted_user_content".to_owned(),
            origin: scope.origin().to_owned(),
            owner: scope.owner().to_owned(),
            repository: scope.repository().to_owned(),
            issue_number,
            source_url: format!(
                "{}/api/v1/repos/{}/{}/issues/{issue_number}",
                scope.origin(),
                scope.owner(),
                scope.repository()
            ),
            updated_at: updated_at.into(),
            title: title.into(),
            body,
            labels: None,
            milestone: None,
            content_sha256: String::new(),
        };
        content.content_sha256 = content.calculate_sha256();
        content
    }

    pub fn with_labels(mut self, labels: Vec<UntrustedWorkItemLabelV1>) -> Self {
        self.labels = Some(labels);
        if self.milestone.is_none() {
            self.schema_version = 2;
        }
        self.content_sha256 = self.calculate_sha256();
        self
    }

    pub fn with_milestone(mut self, milestone: UntrustedWorkItemMilestoneV1) -> Self {
        self.schema_version = 3;
        self.milestone = Some(milestone);
        self.content_sha256 = self.calculate_sha256();
        self
    }

    fn calculate_sha256(&self) -> String {
        // A JSON tuple gives each field an unambiguous, stable position without
        // relying on a delimiter that might occur in user-controlled text.
        let bytes = match self.schema_version {
            1 => serde_json::to_vec(&(
                self.schema_version,
                &self.source,
                &self.trust,
                &self.origin,
                &self.owner,
                &self.repository,
                self.issue_number,
                &self.source_url,
                &self.updated_at,
                &self.title,
                &self.body,
            )),
            2 => serde_json::to_vec(&(
                self.schema_version,
                &self.source,
                &self.trust,
                &self.origin,
                &self.owner,
                &self.repository,
                self.issue_number,
                &self.source_url,
                &self.updated_at,
                &self.title,
                &self.body,
                &self.labels,
            )),
            3 => serde_json::to_vec(&(
                self.schema_version,
                &self.source,
                &self.trust,
                &self.origin,
                &self.owner,
                &self.repository,
                self.issue_number,
                &self.source_url,
                &self.updated_at,
                &self.title,
                &self.body,
                &self.labels,
                &self.milestone,
            )),
            _ => Ok(Vec::new()),
        }
        .unwrap_or_default();
        sha256(&bytes)
    }

    pub fn estimated_tokens(&self) -> u64 {
        serde_json::to_vec(self)
            .map(|bytes| bytes.len().div_ceil(4) as u64)
            .unwrap_or(u64::MAX)
    }

    fn validate(&self, issue_number: u64) -> Result<(), &'static str> {
        if !matches!(self.schema_version, 1 | 2 | 3)
            || self.source != "current_forgejo_issue_api"
            || self.trust != "untrusted_user_content"
            || self.issue_number != issue_number
            || self.issue_number == 0
            || self.owner.trim().is_empty()
            || self.repository.trim().is_empty()
            || self.updated_at.trim().is_empty()
            || self.updated_at.len() > 128
            || self.updated_at.chars().any(char::is_control)
            || !valid_forgejo_timestamp(&self.updated_at)
            || self.title.trim().is_empty()
            || self.title.len() > MAX_UNTRUSTED_WORK_ITEM_TITLE_BYTES
            || self.title.chars().any(char::is_control)
        {
            return Err("work item source, identity, timestamp, or title is invalid");
        }
        let body_bytes_valid = match &self.body {
            UntrustedWorkItemBodyV1::ExplicitNull => true,
            UntrustedWorkItemBodyV1::Text(text) => {
                text.len() <= MAX_UNTRUSTED_WORK_ITEM_BODY_BYTES
                    && !text.chars().any(|character| {
                        character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                    })
            }
        };
        if !body_bytes_valid {
            return Err("work item body is invalid or exceeds the runtime bound");
        }
        let labels_valid = match (self.schema_version, self.labels.as_ref()) {
            (1, None) | (3, None) => true,
            (2 | 3, Some(labels)) if labels.len() <= MAX_UNTRUSTED_WORK_ITEM_LABELS => {
                let mut previous_id = 0;
                labels.iter().all(|label| {
                    let valid = label.id > previous_id
                        && !label.name.trim().is_empty()
                        && label.name.len() <= MAX_UNTRUSTED_WORK_ITEM_LABEL_NAME_BYTES
                        && !label.name.chars().any(char::is_control);
                    previous_id = label.id;
                    valid
                })
            }
            _ => false,
        };
        if !labels_valid {
            return Err("work item labels are invalid or exceed the runtime bound");
        }
        let milestone_valid = match (self.schema_version, self.milestone.as_ref()) {
            (1 | 2, None) => true,
            (
                3,
                Some(
                    UntrustedWorkItemMilestoneV1::Unknown
                    | UntrustedWorkItemMilestoneV1::Unassigned,
                ),
            ) => true,
            (3, Some(UntrustedWorkItemMilestoneV1::Assigned(value))) => {
                value.id > 0
                    && !value.title.trim().is_empty()
                    && value.title.len() <= MAX_UNTRUSTED_WORK_ITEM_MILESTONE_TITLE_BYTES
                    && !value.title.chars().any(char::is_control)
            }
            _ => false,
        };
        if !milestone_valid {
            return Err("work item milestone is invalid or exceeds the runtime bound");
        }
        let scope = BaseBranchScope::new(&self.origin, &self.owner, &self.repository, "main")
            .map_err(|_| "work item Forgejo scope is invalid")?;
        let expected_source_url = format!(
            "{}/api/v1/repos/{}/{}/issues/{}",
            scope.origin(),
            scope.owner(),
            scope.repository(),
            self.issue_number
        );
        if expected_source_url.len() > 2_048 || self.source_url != expected_source_url {
            return Err("work item source route exceeds its bound");
        }
        if self.content_sha256 != self.calculate_sha256() {
            return Err("work item content digest does not match its fields");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CandidateRefV1 {
    pub workspace_id: String,
    pub base_commit: String,
    pub changed_paths: Vec<String>,
    /// A locator/integrity hint for the proposed diff, not a correctness proof.
    pub diff_sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionOutcomeV1 {
    CandidateReady,
    NeedsClarification,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentUsageV1 {
    /// Runtime-reported usage, not an independently verified host or provider receipt.
    /// Missing values stay unknown; reported zero is still only a claim.
    /// Input token count; `None` means unavailable, not zero.
    pub input_tokens: Option<u64>,
    /// Output token count; `None` means unavailable, not zero.
    pub output_tokens: Option<u64>,
    /// Runtime-reported USD in micro-units; `None` means unavailable, not free.
    pub cost_microusd: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentExecutionResultV1 {
    pub schema_version: u32,
    pub mission_id: String,
    /// Binds this result to the request's host-assigned logical execution generation.
    #[serde(default)]
    pub idempotency_key: String,
    pub outcome: AgentExecutionOutcomeV1,
    pub candidate: Option<CandidateRefV1>,
    pub clarification: Option<String>,
    pub usage: AgentUsageV1,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentRuntimeErrorV1 {
    pub code: String,
    pub retryable: bool,
    pub action: String,
    pub details: Value,
}

impl AgentRuntimeErrorV1 {
    pub fn new(
        code: impl Into<String>,
        retryable: bool,
        action: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            code: code.into(),
            retryable,
            action: action.into(),
            details,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCallReceiptV1 {
    pub tool_id: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub elapsed_ms: u64,
    pub success: bool,
    pub error_code: Option<String>,
}

/// Host-owned port. Implementations enforce capabilities and retain one receipt per call,
/// including refused calls, independently of claims returned by the agent runtime.
pub trait AgentToolPortV1 {
    fn invoke(&mut self, tool_id: &str, input: Value) -> Result<Value, AgentRuntimeErrorV1>;

    /// Invoke a model tool while preserving its JSON-RPC correlation ID for
    /// host-owned idempotency. Existing adapters may ignore the ID; durable
    /// effectful adapters should override this method.
    fn invoke_with_request_id(
        &mut self,
        tool_id: &str,
        input: Value,
        _request_id: &Value,
    ) -> Result<Value, AgentRuntimeErrorV1> {
        self.invoke(tool_id, input)
    }

    fn receipts(&self) -> &[ToolCallReceiptV1];
}

/// A runtime adapter. It has no Forgejo merge authority or verifier key by contract.
/// Implementations must return the same result for repeated identical keys and reject
/// reuse of a key with a different request. Durable replay storage belongs to the host.
pub trait AgentRuntimeV1 {
    fn execute(
        &mut self,
        request: &AgentExecutionRequestV1,
        tools: &mut dyn AgentToolPortV1,
    ) -> Result<AgentExecutionResultV1, AgentRuntimeErrorV1>;
}

impl AgentExecutionRequestV1 {
    pub fn validate(&self) -> Result<(), AgentRuntimeErrorV1> {
        self.validate_inner(true)
    }

    /// Validate request content before the trusted host persists an execution
    /// generation. The mission ID and idempotency key are assigned only after
    /// allocation; every other request and context invariant is checked here.
    pub(crate) fn validate_before_allocation(&self) -> Result<(), AgentRuntimeErrorV1> {
        self.validate_inner(false)
    }

    fn validate_inner(&self, require_identity: bool) -> Result<(), AgentRuntimeErrorV1> {
        if !matches!(
            self.schema_version,
            AGENT_RUNTIME_SCHEMA_VERSION
                | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V2
                | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3
                | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4
        ) {
            return Err(error(
                "schema_version_unsupported",
                "use supported agent request schema version 1, 2, 3, or 4",
                json!({"actual":self.schema_version,"supported":[AGENT_RUNTIME_SCHEMA_VERSION,AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V2,AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3,AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4]}),
            ));
        }
        if require_identity
            && (self.mission_id.trim().is_empty() || self.idempotency_key.trim().is_empty())
        {
            return Err(error(
                "execution_identity_missing",
                "provide mission_id and idempotency_key",
                json!({}),
            ));
        }
        if !valid_sha256(&self.requirement_sha256) {
            return Err(error(
                "requirement_hash_invalid",
                "provide the SHA-256 of the protected requirement revision",
                json!({}),
            ));
        }
        if sha256(self.requirements_json.as_bytes()) != self.requirement_sha256 {
            return Err(error(
                "requirement_hash_mismatch",
                "reload the exact protected requirements document for this hash",
                json!({"requirement_sha256":self.requirement_sha256}),
            ));
        }
        let requirements: Requirements =
            crate::strict_json::parse_value_rejecting_duplicate_members(
                self.requirements_json.as_bytes(),
            )
            .and_then(serde_json::from_value)
            .map_err(|parse_error| {
                error(
                    "requirements_document_invalid",
                    "provide a valid versioned requirements JSON document",
                    json!({"error":parse_error.to_string()}),
                )
            })?;
        let arbitration_valid = match (self.schema_version, requirements.schema_version) {
            (
                AGENT_RUNTIME_SCHEMA_VERSION
                | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3
                | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4,
                2,
            ) => {
                requirements.source_pull_request_number.is_none()
                    && requirements.decisions.is_empty()
                    && self.requirement_arbitration.is_none()
            }
            (
                AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V2
                | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3
                | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4,
                3,
            ) => {
                let Some(decision) = self.requirement_arbitration.as_ref() else {
                    return Err(error(
                        "requirements_arbitration_missing",
                        "admit schema-v3 requirements through the host-reviewed pull-request path and include its provenance receipt",
                        json!({"requirement_id":requirements.id,"revision":requirements.revision}),
                    ));
                };
                requirements.ambiguities.is_empty()
                    && !requirements.decisions.is_empty()
                    && requirements.decisions.iter().all(|item| {
                        !item.ambiguity.trim().is_empty()
                            && !item.resolution.trim().is_empty()
                            && !item.rationale.trim().is_empty()
                    })
                    && requirements.source_pull_request_number
                        == Some(decision.source_pull_request_number)
                    && decision.schema_version == 1
                    && decision.source_pull_request_number > 0
                    && valid_git_revision(&decision.source_merge_commit_sha)
                    && valid_git_revision(&decision.source_head_sha)
                    && decision.requirements_sha256 == self.requirement_sha256
                    && decision.policy_revision > 0
                    && valid_sha256(&decision.policy_sha256)
                    && valid_sha256(&decision.review_snapshot_sha256)
                    && !decision.reviewer_ids.is_empty()
                    && decision.reviewer_ids.len() == decision.approved_review_ids.len()
                    && decision.reviewer_ids.iter().all(|id| *id > 0)
                    && decision.approved_review_ids.iter().all(|id| *id > 0)
                    && decision
                        .reviewer_ids
                        .iter()
                        .copied()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == decision.reviewer_ids.len()
                    && decision
                        .approved_review_ids
                        .iter()
                        .copied()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == decision.approved_review_ids.len()
            }
            _ => false,
        };
        let issue_context_schema_valid =
            match (self.schema_version, self.untrusted_work_item.as_ref()) {
                (
                    AGENT_RUNTIME_SCHEMA_VERSION | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V2,
                    None,
                ) => true,
                (
                    AGENT_RUNTIME_SCHEMA_VERSION | AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V2,
                    Some(work_item),
                ) => work_item.schema_version == 1,
                (AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3, Some(work_item)) => {
                    work_item.schema_version == 2
                }
                (AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V3, None) => false,
                (AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4, Some(work_item)) => {
                    work_item.schema_version == 3
                }
                (AGENT_EXECUTION_REQUEST_SCHEMA_VERSION_V4, None) => false,
                _ => false,
            };
        if !issue_context_schema_valid {
            return Err(error(
                "work_item_context_schema_unsupported",
                "use request schemas 1 or 2 for legacy issue text, schema 3 with labels, or schema 4 with the milestone-capable issue context envelope",
                json!({"request_schema_version":self.schema_version,"work_item_schema_version":self.untrusted_work_item.as_ref().map(|work_item| work_item.schema_version)}),
            ));
        }
        if !matches!(requirements.schema_version, 2 | 3)
            || !arbitration_valid
            || requirements
                .source_issue_number
                .is_none_or(|number| number == 0)
            || requirements.id.trim().is_empty()
            || requirements.revision == 0
            || requirements.request.trim().is_empty()
            || requirements.acceptance.is_empty()
            || requirements
                .acceptance
                .iter()
                .chain(requirements.ambiguities.iter())
                .any(|item| item.trim().is_empty())
        {
            return Err(error(
                "requirements_document_invalid",
                "provide valid version 2 requirements or host-reviewed version 3 requirements with a positive source_issue_number, revision, and acceptance criteria",
                json!({"requirement_id":requirements.id,"revision":requirements.revision,"source_issue_number":requirements.source_issue_number}),
            ));
        }
        if self.workspace.id.trim().is_empty() || !valid_git_revision(&self.workspace.base_commit) {
            return Err(error(
                "workspace_reference_invalid",
                "provide a workspace ID and full base commit",
                json!({"workspace_id":self.workspace.id}),
            ));
        }
        if self.context.manifest.source_commit != self.workspace.base_commit
            || self.context.manifest.requirement_sha256 != self.requirement_sha256
        {
            return Err(error(
                "context_stale",
                "rebuild the context packet for this workspace base and requirement revision",
                json!({
                    "context_commit":self.context.manifest.source_commit,
                    "workspace_base":self.workspace.base_commit,
                    "context_requirement_sha256":self.context.manifest.requirement_sha256,
                    "requirement_sha256":self.requirement_sha256,
                }),
            ));
        }
        if self.context.text.trim().is_empty() {
            return Err(error(
                "context_empty",
                "provide a sourced context packet or request clarification",
                json!({}),
            ));
        }
        self.context
            .manifest
            .validate(
                &self.workspace.base_commit,
                &self.requirement_sha256,
                self.context.manifest.token_budget,
            )
            .map_err(|reason| {
                error(
                    "context_invalid",
                    "rebuild or repair the context manifest before execution",
                    json!({"reason":reason}),
                )
            })?;
        let minimum_context_estimate =
            ContextPacketV1::estimate_tokens_for_text(&self.context.text);
        if self.context.manifest.estimated_tokens < minimum_context_estimate {
            return Err(error(
                "context_token_estimate_underreported",
                "rebuild the context manifest with an estimate at least as large as the generic UTF-8 byte heuristic",
                json!({
                    "estimated_tokens": self.context.manifest.estimated_tokens,
                    "minimum_heuristic_estimate": minimum_context_estimate,
                    "context_bytes": self.context.text.len(),
                    "token_budget": self.context.manifest.token_budget,
                }),
            ));
        }
        if let Some(work_item) = &self.untrusted_work_item {
            let Some(source_issue_number) = requirements.source_issue_number else {
                return Err(error(
                    "work_item_context_binding_invalid",
                    "use a protected version 2 requirement bound to a source issue",
                    json!({}),
                ));
            };
            if work_item.issue_number != source_issue_number {
                return Err(error(
                    "work_item_context_binding_invalid",
                    "reload current issue context for the same issue bound by the protected requirement",
                    json!({"requirement_issue_number":source_issue_number,"context_issue_number":work_item.issue_number}),
                ));
            }
            work_item.validate(source_issue_number).map_err(|reason| {
                error(
                    "work_item_context_integrity_invalid",
                    "re-read the bounded current issue projection from the configured Forgejo API",
                    json!({"reason":reason}),
                )
            })?;
            if self
                .context
                .manifest
                .estimated_tokens
                .saturating_add(work_item.estimated_tokens())
                > self.context.manifest.token_budget
            {
                return Err(error(
                    "work_item_context_budget_exceeded",
                    "omit the optional issue text or reduce other context before requesting execution",
                    json!({"context_tokens":self.context.manifest.estimated_tokens,"work_item_tokens":work_item.estimated_tokens(),"token_budget":self.context.manifest.token_budget}),
                ));
            }
        }

        let mut cited_sources = vec![false; self.context.manifest.sources.len()];
        let mut previous_end = 0;
        if self.context.citations.is_empty() {
            return Err(error(
                "context_citation_missing",
                "attach at least one text range to a commit-pinned source",
                json!({}),
            ));
        }
        for citation in &self.context.citations {
            let start = citation.text_start_byte;
            let end = citation.text_end_byte;
            if start != previous_end
                || start >= end
                || end > self.context.text.len()
                || !self.context.text.is_char_boundary(start)
                || !self.context.text.is_char_boundary(end)
                || self.context.text[start..end].trim().is_empty()
            {
                return Err(error(
                    "context_citation_invalid",
                    "provide contiguous, ordered UTF-8 ranges covering the context text",
                    json!({"start_byte":start,"end_byte":end,"text_bytes":self.context.text.len()}),
                ));
            }
            let Some(cited) = cited_sources.get_mut(citation.source_index) else {
                return Err(error(
                    "context_citation_source_invalid",
                    "link each citation to a source in the context manifest",
                    json!({"source_index":citation.source_index}),
                ));
            };
            *cited = true;
            previous_end = end;
        }
        if previous_end != self.context.text.len() {
            return Err(error(
                "context_citation_incomplete",
                "cite the complete context text without uncited gaps",
                json!({"covered_bytes":previous_end,"text_bytes":self.context.text.len()}),
            ));
        }
        if cited_sources.iter().any(|cited| !cited) {
            return Err(error(
                "context_source_uncited",
                "cite every source included in the context manifest",
                json!({}),
            ));
        }

        for path in self
            .capabilities
            .read_paths
            .iter()
            .chain(self.capabilities.write_paths.iter())
        {
            if !valid_relative_path(path) {
                return Err(error(
                    "capability_path_invalid",
                    "declare only normalized relative workspace paths",
                    json!({"path":path}),
                ));
            }
        }
        if self.capabilities.tools.is_empty()
            || self
                .capabilities
                .tools
                .iter()
                .any(|tool| tool.trim().is_empty())
            || self.capabilities.budget.max_tool_calls == 0
            || self.capabilities.budget.timeout_ms == 0
            || self.capabilities.budget.max_output_tokens == 0
        {
            return Err(error(
                "runtime_capability_invalid",
                "provide at least one named tool and positive execution budgets",
                json!({}),
            ));
        }
        if self.context.manifest.sources.iter().any(|source| {
            !valid_relative_path(&source.path)
                || !valid_sha256(&source.file_sha256)
                || !self.capabilities.read_paths.contains(&source.path)
        }) {
            return Err(error(
                "context_source_invalid_or_not_readable",
                "provide valid commit-pinned sources inside the read capability set",
                json!({}),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_for_issue_number(
        &self,
        issue_number: u64,
    ) -> Result<(), AgentRuntimeErrorV1> {
        self.validate()?;
        let requirements: Requirements =
            crate::strict_json::parse_value_rejecting_duplicate_members(
                self.requirements_json.as_bytes(),
            )
            .and_then(serde_json::from_value)
            .map_err(|_| {
                error(
                    "requirements_document_invalid",
                    "provide a valid versioned requirements JSON document",
                    json!({}),
                )
            })?;
        if requirements.source_issue_number != Some(issue_number) {
            return Err(error(
                "mission_requirement_issue_mismatch",
                "claim or lease only the issue named by the protected requirement document",
                json!({
                    "issue_number":issue_number,
                    "source_issue_number":requirements.source_issue_number,
                }),
            ));
        }
        Ok(())
    }
}

impl AgentExecutionResultV1 {
    /// Check the returned candidate reference against the request; this does not verify code
    /// or the provenance of runtime-reported token and cost fields. The output check below can
    /// reject an over-limit claim, but a low claim is not evidence that the budget was met.
    pub fn validate_for(
        &self,
        request: &AgentExecutionRequestV1,
    ) -> Result<(), AgentRuntimeErrorV1> {
        request.validate()?;
        if self.schema_version != AGENT_RUNTIME_SCHEMA_VERSION
            || self.mission_id != request.mission_id
            || self.idempotency_key != request.idempotency_key
        {
            return Err(error(
                "runtime_result_binding_mismatch",
                "return a result for this mission and execution using the supported schema",
                json!({
                    "mission_id":self.mission_id,
                    "expected_mission_id":request.mission_id,
                    "execution_key_matches":self.idempotency_key == request.idempotency_key,
                }),
            ));
        }
        if self
            .usage
            .output_tokens
            .is_some_and(|tokens| tokens > request.capabilities.budget.max_output_tokens)
        {
            return Err(error(
                "output_budget_exceeded",
                "reduce the response or request a larger declared output budget",
                json!({
                    "max_output_tokens":request.capabilities.budget.max_output_tokens,
                    "reported_output_tokens":self.usage.output_tokens,
                }),
            ));
        }
        match self.outcome {
            AgentExecutionOutcomeV1::CandidateReady => {
                let candidate = self.candidate.as_ref().ok_or_else(|| {
                    error(
                        "candidate_missing",
                        "return a candidate reference or request clarification",
                        json!({}),
                    )
                })?;
                if self.clarification.is_some()
                    || candidate.workspace_id != request.workspace.id
                    || candidate.base_commit != request.workspace.base_commit
                    || !valid_sha256(&candidate.diff_sha256)
                    || candidate.changed_paths.is_empty()
                    || candidate.changed_paths.iter().any(|path| {
                        !valid_relative_path(path)
                            || !request.capabilities.write_paths.contains(path)
                    })
                    || has_duplicates(&candidate.changed_paths)
                {
                    return Err(error(
                        "candidate_reference_invalid",
                        "return a candidate reference inside the authorized workspace write set",
                        json!({
                            "workspace_id":candidate.workspace_id,
                            "base_commit":candidate.base_commit,
                        }),
                    ));
                }
            }
            AgentExecutionOutcomeV1::NeedsClarification => {
                if self.candidate.is_some()
                    || self
                        .clarification
                        .as_ref()
                        .is_none_or(|question| question.trim().is_empty())
                {
                    return Err(error(
                        "clarification_result_invalid",
                        "return a concrete clarification question without a candidate",
                        json!({}),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn error(code: &str, action: &str, details: Value) -> AgentRuntimeErrorV1 {
    AgentRuntimeErrorV1::new(code, false, action, details)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn sha256_fields(fields: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    hex::encode(digest.finalize())
}

fn valid_git_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_relative_path(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.contains(['\\', '\0', ':'])
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn has_duplicates(paths: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    paths.iter().any(|path| !seen.insert(path))
}
