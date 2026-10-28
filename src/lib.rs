use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<Sha256>;

pub mod agent_runtime;
pub mod forgejo;
pub mod mission_admission;
pub mod mission_profile;
pub mod strict_json;
pub mod supervisor_capability;
pub mod webhook;

#[derive(Debug, PartialEq, Eq)]
pub enum ParseDurationError {
    Empty,
    InvalidNumber,
    Overflow,
}

/// Parse a non-negative whole duration. No suffix and `ms` mean milliseconds;
/// `s` and `m` are scaled using checked arithmetic.
pub fn parse_duration_ms(input: &str) -> Result<u64, ParseDurationError> {
    let lower = input.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Err(ParseDurationError::Empty);
    }
    let (number, multiplier) = if let Some(value) = lower.strip_suffix("ms") {
        (value, 1_u64)
    } else if let Some(value) = lower.strip_suffix('s') {
        (value, 1_000_u64)
    } else if let Some(value) = lower.strip_suffix('m') {
        (value, 60_000_u64)
    } else {
        (lower.as_str(), 1_u64)
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseDurationError::InvalidNumber);
    }
    let value = number
        .parse::<u64>()
        .map_err(|_| ParseDurationError::Overflow)?;
    value
        .checked_mul(multiplier)
        .ok_or(ParseDurationError::Overflow)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    pub schema_version: u32,
    pub id: String,
    pub revision: u64,
    /// Protected Forgejo issue number authorizing this requirement document.
    /// Present only in schema version 2; legacy documents remain readable but
    /// cannot be admitted as new missions without this link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_issue_number: Option<u64>,
    /// Protected pull request whose merged revision recorded schema-v3
    /// arbitration. The number is an untrusted pointer until the host reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_pull_request_number: Option<u64>,
    pub request: String,
    pub acceptance: Vec<String>,
    pub ambiguities: Vec<String>,
    /// Structured ambiguity resolution and rationale, preserved in the exact
    /// protected requirement bytes. The Forgejo review gate authenticates the
    /// decision record; these strings alone do not establish authorship.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<RequirementDecisionV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequirementDecisionV1 {
    pub ambiguity: String,
    pub resolution: String,
    pub rationale: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub revision: u64,
    pub verifier_id: String,
    pub environment: String,
    pub acceptance_command: Vec<String>,
    pub commands: Vec<Vec<String>>,
    pub protected_criteria: Vec<String>,
    pub timeout_seconds: u64,
    /// Optional Forgejo review policy. Absence means that no merge can pass
    /// the independent approval gate; deployments must populate it at a
    /// protected policy revision before enabling any merge authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_approval_policy: Option<forgejo::MergeApprovalPolicyV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextSource {
    pub path: String,
    pub line_start: u64,
    pub line_end: u64,
    pub file_sha256: String,
    pub provenance: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextManifest {
    pub schema_version: u32,
    pub source_commit: String,
    pub requirement_sha256: String,
    pub token_budget: u64,
    /// Provider estimate; request validation rejects values below the generic
    /// `ceil(context_text_utf8_bytes / 4)` heuristic. It is not tokenizer-exact.
    pub estimated_tokens: u64,
    pub sources: Vec<ContextSource>,
}

impl ContextManifest {
    pub fn validate(
        &self,
        commit: &str,
        requirement_sha256: &str,
        token_budget: u64,
    ) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported context schema".into());
        }
        if self.source_commit != commit {
            return Err("context is stale for source commit".into());
        }
        if self.requirement_sha256 != requirement_sha256 {
            return Err("context is stale for requirements revision".into());
        }
        if self.token_budget != token_budget || self.estimated_tokens > token_budget {
            return Err("context token budget exceeded or changed".into());
        }
        if self.sources.is_empty()
            || self.sources.iter().any(|s| {
                s.path.is_empty()
                    || s.line_start == 0
                    || s.line_end < s.line_start
                    || s.file_sha256.len() != 64
                    || s.provenance.is_empty()
            })
        {
            return Err("context must contain valid commit-pinned source references".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissionState {
    Requested,
    Planned,
    Executing,
    CandidateReady,
    Verifying,
    Rejected,
    Approved,
    Merged,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MissionSnapshot {
    pub revision: u64,
    pub state: MissionState,
}

impl MissionSnapshot {
    /// Compare-and-set revision prevents delayed or reordered events from moving
    /// a mission backwards after a newer transition has already been applied.
    pub fn transition(
        &mut self,
        expected_revision: u64,
        next: MissionState,
    ) -> Result<u64, String> {
        if expected_revision != self.revision {
            return Err(format!(
                "stale mission revision: expected {}, current {}",
                expected_revision, self.revision
            ));
        }
        transition_mission(self.state, next)?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| "mission revision overflow".to_owned())?;
        self.state = next;
        Ok(self.revision)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AuthorizationLease {
    pub subject: String,
    pub capabilities: Vec<String>,
    pub expires_at_unix: u64,
}

impl AuthorizationLease {
    pub fn require(&self, capability: &str, now_unix: u64) -> Result<(), String> {
        if now_unix >= self.expires_at_unix {
            return Err("authorization expired; renew through the authority".into());
        }
        if !self
            .capabilities
            .iter()
            .any(|granted| granted == capability)
        {
            return Err(format!("capability denied: {capability}"));
        }
        Ok(())
    }
}

pub fn transition_mission(from: MissionState, to: MissionState) -> Result<(), String> {
    use MissionState::*;
    let allowed = matches!(
        (from, to),
        (Requested, Planned)
            | (Planned, Executing)
            | (Executing, CandidateReady)
            | (CandidateReady, Verifying)
            | (Verifying, Rejected)
            | (Rejected, Executing)
            | (Verifying, Approved)
            | (Approved, Merged)
            | (Requested, Cancelled)
            | (Planned, Cancelled)
            | (Executing, Cancelled)
            | (CandidateReady, Cancelled)
            | (Verifying, Cancelled)
            | (Rejected, Cancelled)
    );
    if allowed {
        Ok(())
    } else {
        Err(format!("transition interdite: {from:?} -> {to:?}"))
    }
}

pub fn read_requirements(path: &Path) -> Result<Requirements, String> {
    let bytes = fs::read(path).map_err(|e| format!("read requirements: {e}"))?;
    let value: Requirements =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse requirements: {e}"))?;
    if !matches!(
        (value.schema_version, value.source_issue_number),
        (1, None) | (2, Some(1..=u64::MAX))
    ) || value.id.is_empty()
        || value.revision == 0
        || value.acceptance.is_empty()
    {
        return Err("requirements schema/id/revision/acceptance invalides".into());
    }
    Ok(value)
}

pub fn read_policy(path: &Path) -> Result<Policy, String> {
    let bytes = fs::read(path).map_err(|e| format!("read policy: {e}"))?;
    let value: Policy = serde_json::from_slice(&bytes).map_err(|e| format!("parse policy: {e}"))?;
    if value.schema_version != 1
        || value.revision == 0
        || value.verifier_id.is_empty()
        || value.timeout_seconds == 0
    {
        return Err("policy schema/revision/verifier/timeout invalides".into());
    }
    Ok(value)
}

pub fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(sha256(&bytes))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProofEvidence {
    pub schema_version: u32,
    pub requirement_sha256: String,
    pub policy_sha256: String,
    pub candidate_commit: String,
    pub base_commit: String,
    pub merge_commit: Option<String>,
    pub verifier_id: String,
    pub environment: String,
    pub commands: Vec<Vec<String>>,
    pub artifact_sha256: Vec<String>,
    pub outcome: Outcome,
    pub detail: String,
}

/// Strict V2 wire representation of the V1 evidence fields. The evidence
/// semantics remain schema version 1; a separate DTO keeps V2 from accepting
/// fields that a later implementation might interpret without signing.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetProofEvidenceV2 {
    pub schema_version: u32,
    pub requirement_sha256: String,
    pub policy_sha256: String,
    pub candidate_commit: String,
    pub base_commit: String,
    pub merge_commit: Option<String>,
    pub verifier_id: String,
    pub environment: String,
    pub commands: Vec<Vec<String>>,
    pub artifact_sha256: Vec<String>,
    pub outcome: Outcome,
    pub detail: String,
}

impl From<ProofEvidence> for TargetProofEvidenceV2 {
    fn from(evidence: ProofEvidence) -> Self {
        Self {
            schema_version: evidence.schema_version,
            requirement_sha256: evidence.requirement_sha256,
            policy_sha256: evidence.policy_sha256,
            candidate_commit: evidence.candidate_commit,
            base_commit: evidence.base_commit,
            merge_commit: evidence.merge_commit,
            verifier_id: evidence.verifier_id,
            environment: evidence.environment,
            commands: evidence.commands,
            artifact_sha256: evidence.artifact_sha256,
            outcome: evidence.outcome,
            detail: evidence.detail,
        }
    }
}

impl From<&TargetProofEvidenceV2> for ProofEvidence {
    fn from(evidence: &TargetProofEvidenceV2) -> Self {
        Self {
            schema_version: evidence.schema_version,
            requirement_sha256: evidence.requirement_sha256.clone(),
            policy_sha256: evidence.policy_sha256.clone(),
            candidate_commit: evidence.candidate_commit.clone(),
            base_commit: evidence.base_commit.clone(),
            merge_commit: evidence.merge_commit.clone(),
            verifier_id: evidence.verifier_id.clone(),
            environment: evidence.environment.clone(),
            commands: evidence.commands.clone(),
            artifact_sha256: evidence.artifact_sha256.clone(),
            outcome: evidence.outcome.clone(),
            detail: evidence.detail.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SignedProof {
    pub evidence: ProofEvidence,
    pub mac_sha256: String,
}

pub const TARGET_BOUND_PROOF_V2_SCHEMA_VERSION: u32 = 2;
const TARGET_BOUND_PROOF_V2_DOMAIN: &[u8] = b"agent-forge/target-bound-proof/v2\0";

/// Exact Forgejo target authenticated by a target-bound V2 proof.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProofTargetV2 {
    pub profile_selector: String,
    pub origin: String,
    pub owner: String,
    pub repository: String,
    pub base_branch: String,
}

impl ProofTargetV2 {
    pub fn new(
        profile_selector: impl Into<String>,
        origin: impl Into<String>,
        owner: impl Into<String>,
        repository: impl Into<String>,
        base_branch: impl Into<String>,
    ) -> Result<Self, String> {
        let target = Self {
            profile_selector: profile_selector.into(),
            origin: origin.into(),
            owner: owner.into(),
            repository: repository.into(),
            base_branch: base_branch.into(),
        };
        target.validate()?;
        Ok(target)
    }

    fn validate(&self) -> Result<(), String> {
        if self.profile_selector.is_empty()
            || self.profile_selector.len() > 128
            || !self.profile_selector.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            })
        {
            return Err("invalid target profile selector".to_owned());
        }
        let scope = crate::forgejo::BaseBranchScope::new(
            &self.origin,
            &self.owner,
            &self.repository,
            &self.base_branch,
        )
        .map_err(|_| "invalid target scope".to_owned())?;
        if scope.origin() != self.origin
            || scope.owner() != self.owner
            || scope.repository() != self.repository
            || scope.branch() != self.base_branch
        {
            return Err("target scope must use canonical values".to_owned());
        }
        Ok(())
    }
}

/// Target-bound V2 proof. Its target MAC covers the target and all evidence
/// fields with a domain separator; it is not a wrapper around portable V1.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetBoundProofV2 {
    pub schema_version: u32,
    pub target: ProofTargetV2,
    pub evidence: TargetProofEvidenceV2,
    pub target_mac_sha256: String,
}

/// Local V2 policy: verifier keys must contain at least 256 bits of key material.
pub const MIN_TARGET_BOUND_PROOF_KEY_BYTES: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExpectedBinding {
    pub requirement_sha256: String,
    pub policy_sha256: String,
    pub candidate_commit: String,
    pub base_commit: String,
    pub merge_commit: Option<String>,
    pub verifier_id: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GateError {
    MissingProof,
    InvalidSignature,
    InvalidVerifierKey,
    InvalidTarget,
    WrongTarget,
    UnsupportedProofVersion,
    StaleBinding(&'static str),
    OutcomeFailed,
    OutcomeInconclusive,
}

pub fn sign_proof(evidence: ProofEvidence, key: &[u8]) -> Result<SignedProof, String> {
    let bytes = serde_json::to_vec(&evidence).map_err(|e| e.to_string())?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| e.to_string())?;
    mac.update(&bytes);
    Ok(SignedProof {
        evidence,
        mac_sha256: hex::encode(mac.finalize().into_bytes()),
    })
}

#[derive(Serialize)]
struct TargetBoundProofMacPayloadV2<'a> {
    schema_version: u32,
    target: &'a ProofTargetV2,
    evidence: &'a TargetProofEvidenceV2,
}

fn target_bound_proof_payload_v2(
    target: &ProofTargetV2,
    evidence: &TargetProofEvidenceV2,
) -> Result<Vec<u8>, String> {
    let payload = TargetBoundProofMacPayloadV2 {
        schema_version: TARGET_BOUND_PROOF_V2_SCHEMA_VERSION,
        target,
        evidence,
    };
    serde_json::to_vec(&payload).map_err(|error| error.to_string())
}

fn target_bound_proof_mac_v2(
    target: &ProofTargetV2,
    evidence: &TargetProofEvidenceV2,
    key: &[u8],
) -> Result<String, String> {
    let bytes = target_bound_proof_payload_v2(target, evidence)?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|error| error.to_string())?;
    mac.update(TARGET_BOUND_PROOF_V2_DOMAIN);
    mac.update(&bytes);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub fn sign_target_bound_proof_v2(
    evidence: ProofEvidence,
    target: ProofTargetV2,
    key: &[u8],
) -> Result<TargetBoundProofV2, String> {
    if key.len() < MIN_TARGET_BOUND_PROOF_KEY_BYTES {
        return Err(format!(
            "target-bound V2 verifier key must be at least {MIN_TARGET_BOUND_PROOF_KEY_BYTES} bytes"
        ));
    }
    target.validate()?;
    if evidence.schema_version != 1 {
        return Err("target-bound proof evidence schema_version must be 1".to_owned());
    }
    let evidence = TargetProofEvidenceV2::from(evidence);
    let target_mac_sha256 = target_bound_proof_mac_v2(&target, &evidence, key)?;
    Ok(TargetBoundProofV2 {
        schema_version: TARGET_BOUND_PROOF_V2_SCHEMA_VERSION,
        target,
        evidence,
        target_mac_sha256,
    })
}

pub fn verify_proof(
    proof: Option<&SignedProof>,
    key: &[u8],
    expected: &ExpectedBinding,
) -> Result<(), GateError> {
    let proof = proof.ok_or(GateError::MissingProof)?;
    let bytes = serde_json::to_vec(&proof.evidence).map_err(|_| GateError::InvalidSignature)?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| GateError::InvalidSignature)?;
    mac.update(&bytes);
    let provided = hex::decode(&proof.mac_sha256).map_err(|_| GateError::InvalidSignature)?;
    mac.verify_slice(&provided)
        .map_err(|_| GateError::InvalidSignature)?;
    verify_proof_evidence(&proof.evidence, expected)
}

pub fn verify_target_bound_proof_v2(
    proof: Option<&TargetBoundProofV2>,
    key: &[u8],
    expected_target: &ProofTargetV2,
    expected: &ExpectedBinding,
) -> Result<(), GateError> {
    let proof = proof.ok_or(GateError::MissingProof)?;
    if proof.schema_version != TARGET_BOUND_PROOF_V2_SCHEMA_VERSION {
        return Err(GateError::UnsupportedProofVersion);
    }
    if key.len() < MIN_TARGET_BOUND_PROOF_KEY_BYTES {
        return Err(GateError::InvalidVerifierKey);
    }
    expected_target
        .validate()
        .map_err(|_| GateError::InvalidTarget)?;
    proof
        .target
        .validate()
        .map_err(|_| GateError::InvalidTarget)?;
    if &proof.target != expected_target {
        return Err(GateError::WrongTarget);
    }
    let bytes = target_bound_proof_payload_v2(&proof.target, &proof.evidence)
        .map_err(|_| GateError::InvalidSignature)?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| GateError::InvalidSignature)?;
    mac.update(TARGET_BOUND_PROOF_V2_DOMAIN);
    mac.update(&bytes);
    let provided =
        hex::decode(&proof.target_mac_sha256).map_err(|_| GateError::InvalidSignature)?;
    mac.verify_slice(&provided)
        .map_err(|_| GateError::InvalidSignature)?;
    let evidence = ProofEvidence::from(&proof.evidence);
    verify_proof_evidence(&evidence, expected)
}

fn verify_proof_evidence(
    actual: &ProofEvidence,
    expected: &ExpectedBinding,
) -> Result<(), GateError> {
    if actual.schema_version != 1 {
        return Err(GateError::StaleBinding("schema"));
    }
    if actual.requirement_sha256 != expected.requirement_sha256 {
        return Err(GateError::StaleBinding("requirements"));
    }
    if actual.policy_sha256 != expected.policy_sha256 {
        return Err(GateError::StaleBinding("policy"));
    }
    if actual.candidate_commit != expected.candidate_commit {
        return Err(GateError::StaleBinding("candidate"));
    }
    if actual.base_commit != expected.base_commit {
        return Err(GateError::StaleBinding("base"));
    }
    if actual.merge_commit != expected.merge_commit {
        return Err(GateError::StaleBinding("merge result"));
    }
    if actual.verifier_id != expected.verifier_id {
        return Err(GateError::StaleBinding("verifier identity"));
    }
    if actual.verifier_id.is_empty()
        || actual.environment.is_empty()
        || actual.commands.is_empty()
        || actual.artifact_sha256.is_empty()
    {
        return Err(GateError::InvalidSignature);
    }
    match actual.outcome {
        Outcome::Pass => Ok(()),
        Outcome::Fail => Err(GateError::OutcomeFailed),
        Outcome::Inconclusive => Err(GateError::OutcomeInconclusive),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct JournalEvent {
    pub sequence: u64,
    pub event_id: String,
    pub idempotency_key: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub previous_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_attestation: Option<JournalEventAttestationV1>,
    pub hash: String,
}

#[derive(Clone, Copy)]
struct JournalAppendOptions<'a> {
    mission_id: Option<&'a str>,
    webhook_authority: bool,
    attestation_signer: Option<&'a SigningKey>,
    maximum_bytes: Option<u64>,
}

/// A receiver signature over the complete durable journal identity of a
/// webhook event. The trusted public key is configured outside the journal.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct JournalEventAttestationV1 {
    pub schema_version: u32,
    pub algorithm: String,
    pub key_id: String,
    pub signature_hex: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalAttestationError {
    Missing,
    Unsupported,
    UntrustedKey,
    InvalidSignature,
}

const JOURNAL_ATTESTATION_DOMAIN_V1: &str = "agent-forge/webhook-journal-attestation/v1";

#[derive(Debug, PartialEq, Eq)]
pub enum AppendResult {
    Appended(JournalEvent),
    Duplicate(JournalEvent),
}

/// Decision made while the journal's exclusive lock is held.
pub enum JournalTransaction<T> {
    Return(T),
    Append {
        idempotency_key: String,
        kind: String,
        payload: serde_json::Value,
        result: T,
    },
}

pub enum JournalTransactionError<E> {
    Journal(JournalError),
    Operation(E),
}

#[derive(Debug, PartialEq, Eq)]
pub enum JournalError {
    Io(String),
    Corrupt { line: usize },
    InboxTooLarge { maximum_bytes: u64 },
    IdempotencyConflict,
    LockBusy,
    InvalidEvent,
}

pub struct EventJournal {
    path: PathBuf,
    maximum_bytes: Option<u64>,
}

/// Capability for journal transitions performed while an outer per-mission
/// lock is held. Keeping this separate from `EventJournal` prevents merge
/// callers from accidentally releasing that lock between durable transitions
/// and their bounded host-owned effect.
pub(crate) struct MissionJournalGuard<'a> {
    journal: &'a EventJournal,
    mission_id: &'a str,
}

impl MissionJournalGuard<'_> {
    pub(crate) fn transact<T, E>(
        &self,
        operation: impl FnOnce(&[JournalEvent]) -> Result<JournalTransaction<T>, E>,
    ) -> Result<T, JournalTransactionError<E>> {
        self.journal
            .transact_with_authority(Some(self.mission_id), true, operation)
    }
}

/// Maximum time a journal operation waits for its shared file lock.
pub const EVENT_JOURNAL_LOCK_WAIT_MS: u64 = 5_000;
/// Provisional cumulative limit for the local Forgejo webhook inbox.
pub const MAX_WEBHOOK_INBOX_BYTES: u64 = 64 * 1024 * 1024;

struct FileLock(File);

impl FileLock {
    fn acquire(path: &Path) -> Result<Self, JournalError> {
        Self::acquire_with_timeout(path, Duration::from_millis(EVENT_JOURNAL_LOCK_WAIT_MS))
    }

    fn try_acquire(path: &Path) -> Result<Self, JournalError> {
        Self::acquire_with_timeout(path, Duration::ZERO)
    }

    fn acquire_with_timeout(path: &Path, wait_timeout: Duration) -> Result<Self, JournalError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        options.mode(0o600);
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        let file = options.open(path).map_err(io_err)?;
        let metadata = file.metadata().map_err(io_err)?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(JournalError::InvalidEvent);
        }
        if metadata.mode() & 0o077 != 0 {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(io_err)?;
        }
        // Probe with LOCK_NB so contention never traps a worker in an
        // unbounded kernel wait. The deadline covers lock contention only;
        // filesystem opens and journal I/O have their own failure modes.
        let started = Instant::now();
        let mut first_attempt = true;
        loop {
            // Keep one immediate attempt even for a zero timeout. Every
            // retry after that must begin before the deadline; otherwise a
            // descheduled waiter could acquire a newly released lock late.
            if !first_attempt && started.elapsed() >= wait_timeout {
                return Err(JournalError::LockBusy);
            }
            first_attempt = false;
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                // flock is released by the kernel if the process exits, so
                // replay can resume after interruption.
                return Ok(Self(file));
            }

            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::WouldBlock {
                return Err(io_err(error));
            }
            let remaining = wait_timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(JournalError::LockBusy);
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }

    fn try_acquire_private(path: &Path) -> Result<Self, JournalError> {
        let parent = path.parent().ok_or(JournalError::InvalidEvent)?;
        ensure_private_lock_directory(parent)?;
        Self::try_acquire(path)
    }
}

fn ensure_private_lock_directory(path: &Path) -> Result<(), JournalError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(JournalError::InvalidEvent);
            }
            if metadata.mode() & 0o077 != 0 {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_err)?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return ensure_private_lock_directory(path);
                }
                Err(error) => return Err(io_err(error)),
            }
        }
        Err(error) => return Err(io_err(error)),
    }
    Ok(())
}

impl Drop for FileLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn io_err(error: std::io::Error) -> JournalError {
    JournalError::Io(error.to_string())
}

impl EventJournal {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            maximum_bytes: None,
        }
    }

    /// Open a journal with a cumulative byte limit. Appends that would cross
    /// the limit fail before writing; replay rejects oversized files before
    /// loading them into memory.
    pub fn open_with_max_bytes(path: impl Into<PathBuf>, maximum_bytes: u64) -> Self {
        Self {
            path: path.into(),
            maximum_bytes: Some(maximum_bytes),
        }
    }

    /// Open the local webhook inbox with its compiled-in cumulative limit.
    pub fn open_webhook_inbox(path: impl Into<PathBuf>) -> Self {
        Self::open_with_max_bytes(path, MAX_WEBHOOK_INBOX_BYTES)
    }

    fn canonical_journal_path(&self) -> Result<PathBuf, JournalError> {
        let path = if self.path.is_absolute() {
            self.path.clone()
        } else {
            std::env::current_dir().map_err(io_err)?.join(&self.path)
        };
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(io_err)?;
        let canonical_parent = fs::canonicalize(parent).map_err(io_err)?;
        let name = path.file_name().ok_or(JournalError::InvalidEvent)?;
        let candidate = canonical_parent.join(name);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.nlink() == 1 =>
            {
                fs::canonicalize(candidate).map_err(io_err)
            }
            Ok(_) => Err(JournalError::InvalidEvent),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(candidate),
            Err(error) => Err(io_err(error)),
        }
    }

    fn lock_path(&self) -> Result<PathBuf, JournalError> {
        let mut path = self.canonical_journal_path()?.into_os_string();
        path.push(".lock");
        Ok(PathBuf::from(path))
    }

    fn mission_lock_path(&self, mission_id: &str) -> Result<PathBuf, JournalError> {
        if !valid_mission_id_v1(mission_id) {
            return Err(JournalError::InvalidEvent);
        }
        let mut directory = self.canonical_journal_path()?.into_os_string();
        directory.push(".mission-locks");
        let directory = PathBuf::from(directory);
        ensure_private_lock_directory(&directory)?;
        Ok(directory.join(format!("{mission_id}.lock")))
    }

    pub fn replay(&self) -> Result<Vec<JournalEvent>, JournalError> {
        let _lock = FileLock::acquire(&self.lock_path()?)?;
        self.read_locked()
    }

    /// Replay a bounded snapshot without repairing an incomplete tail.
    ///
    /// Decision authorities use this when they must inspect durable evidence
    /// without changing its source of truth. Unlike `replay`, any record that
    /// is missing its final newline is treated as corruption and left byte for
    /// byte untouched. Generic journals are bounded by the webhook inbox limit
    /// when no smaller explicit limit was configured.
    pub fn replay_read_only(&self) -> Result<Vec<JournalEvent>, JournalError> {
        let _lock = FileLock::acquire(&self.lock_path()?)?;
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(io_err(error)),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err(JournalError::InvalidEvent);
        }
        let maximum_bytes = self
            .maximum_bytes
            .unwrap_or(MAX_WEBHOOK_INBOX_BYTES)
            .min(MAX_WEBHOOK_INBOX_BYTES);
        self.read_locked_with_options(Some(maximum_bytes), false)
    }

    /// Replay an inbox using the webhook limit even when the caller received
    /// a generic `EventJournal::open` handle.
    pub fn replay_webhook_inbox(&self) -> Result<Vec<JournalEvent>, JournalError> {
        let _lock = FileLock::acquire(&self.lock_path()?)?;
        self.read_locked_with_limit(Some(
            self.maximum_bytes
                .unwrap_or(MAX_WEBHOOK_INBOX_BYTES)
                .min(MAX_WEBHOOK_INBOX_BYTES),
        ))
    }

    pub(crate) fn try_replay(&self) -> Result<Vec<JournalEvent>, JournalError> {
        let _lock = FileLock::try_acquire(&self.lock_path()?)?;
        self.read_locked()
    }

    /// Run a read/decision/optional append while holding the same exclusive lock
    /// used by replay and append. The closure must not call a blocking operation
    /// on this EventJournal (such as replay, append or transact) because it would
    /// wait for the lock held by this call. Typed mission transitions use a
    /// nonblocking lock attempt and return `LockBusy` instead.
    pub fn transact<T, E>(
        &self,
        operation: impl FnOnce(&[JournalEvent]) -> Result<JournalTransaction<T>, E>,
    ) -> Result<T, JournalTransactionError<E>> {
        self.transact_with_authority(None, false, operation)
    }

    /// Serialize one typed mission transition with a short, nonblocking
    /// per-mission flock, then try the shared journal flock for the durable
    /// read/decision/append. Contention at either lock returns `LockBusy` rather
    /// than waiting while holding the other lock.
    pub(crate) fn transact_mission<T, E>(
        &self,
        mission_id: &str,
        operation: impl FnOnce(&[JournalEvent]) -> Result<JournalTransaction<T>, E>,
    ) -> Result<T, JournalTransactionError<E>> {
        let mission_lock_path = self
            .mission_lock_path(mission_id)
            .map_err(JournalTransactionError::Journal)?;
        let _mission_lock = FileLock::try_acquire_private(&mission_lock_path)
            .map_err(JournalTransactionError::Journal)?;
        self.transact_with_authority(Some(mission_id), true, operation)
    }

    /// Hold only the mission lock around a caller that performs a journal read
    /// followed by a short, host-owned effect. The caller must not re-enter a
    /// mission transition for this ID from inside the effect.
    pub(crate) fn with_mission_lock<T, E>(
        &self,
        mission_id: &str,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, JournalTransactionError<E>> {
        let mission_lock_path = self
            .mission_lock_path(mission_id)
            .map_err(JournalTransactionError::Journal)?;
        let _mission_lock = FileLock::try_acquire_private(&mission_lock_path)
            .map_err(JournalTransactionError::Journal)?;
        operation().map_err(JournalTransactionError::Operation)
    }

    /// Hold a mission lock while the caller performs multiple short journal
    /// transitions around a bounded host effect. Each `guard.transact` takes
    /// the shared journal lock only for its read/append and reserves mission
    /// event kinds to the supplied mission identity.
    pub(crate) fn with_mission_guard<T, E>(
        &self,
        mission_id: &str,
        operation: impl FnOnce(&MissionJournalGuard<'_>) -> Result<T, E>,
    ) -> Result<T, JournalTransactionError<E>> {
        let mission_lock_path = self
            .mission_lock_path(mission_id)
            .map_err(JournalTransactionError::Journal)?;
        let _mission_lock = FileLock::try_acquire_private(&mission_lock_path)
            .map_err(JournalTransactionError::Journal)?;
        let guard = MissionJournalGuard {
            journal: self,
            mission_id,
        };
        operation(&guard).map_err(JournalTransactionError::Operation)
    }

    fn transact_with_authority<T, E>(
        &self,
        mission_id: Option<&str>,
        nonblocking: bool,
        operation: impl FnOnce(&[JournalEvent]) -> Result<JournalTransaction<T>, E>,
    ) -> Result<T, JournalTransactionError<E>> {
        let lock_path = self.lock_path().map_err(JournalTransactionError::Journal)?;
        let _lock = if nonblocking {
            FileLock::try_acquire(&lock_path)
        } else {
            FileLock::acquire(&lock_path)
        }
        .map_err(JournalTransactionError::Journal)?;
        let events = self
            .read_locked()
            .map_err(JournalTransactionError::Journal)?;
        match operation(&events).map_err(JournalTransactionError::Operation)? {
            JournalTransaction::Return(result) => Ok(result),
            JournalTransaction::Append {
                idempotency_key,
                kind,
                payload,
                result,
            } => {
                self.append_locked(
                    &events,
                    &idempotency_key,
                    &kind,
                    payload,
                    JournalAppendOptions {
                        mission_id,
                        webhook_authority: false,
                        attestation_signer: None,
                        maximum_bytes: self.maximum_bytes,
                    },
                )
                .map_err(JournalTransactionError::Journal)?;
                Ok(result)
            }
        }
    }

    fn read_locked(&self) -> Result<Vec<JournalEvent>, JournalError> {
        self.read_locked_with_limit(self.maximum_bytes)
    }

    fn read_locked_with_limit(
        &self,
        maximum_bytes: Option<u64>,
    ) -> Result<Vec<JournalEvent>, JournalError> {
        self.read_locked_with_options(maximum_bytes, true)
    }

    fn read_locked_with_options(
        &self,
        maximum_bytes: Option<u64>,
        repair_incomplete_tail: bool,
    ) -> Result<Vec<JournalEvent>, JournalError> {
        let mut repair_file: Option<File> = None;
        let bytes = match maximum_bytes {
            Some(maximum_bytes) => {
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
                let file = match options.open(&self.path) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(Vec::new());
                    }
                    Err(error) => return Err(io_err(error)),
                };
                let metadata = file.metadata().map_err(io_err)?;
                if !metadata.is_file()
                    || metadata.uid() != unsafe { libc::geteuid() }
                    || metadata.nlink() != 1
                {
                    return Err(JournalError::InvalidEvent);
                }
                if metadata.len() > maximum_bytes {
                    return Err(JournalError::InboxTooLarge { maximum_bytes });
                }
                let capacity = usize::try_from(metadata.len())
                    .map_err(|_| JournalError::InboxTooLarge { maximum_bytes })?;
                let mut bytes = Vec::with_capacity(capacity);
                let read_file = file.try_clone().map_err(io_err)?;
                read_file
                    .take(maximum_bytes.saturating_add(1))
                    .read_to_end(&mut bytes)
                    .map_err(io_err)?;
                if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum_bytes {
                    return Err(JournalError::InboxTooLarge { maximum_bytes });
                }
                repair_file = Some(file);
                bytes
            }
            None => {
                if !self.path.exists() {
                    return Ok(Vec::new());
                }
                fs::read(&self.path).map_err(io_err)?
            }
        };
        let mut events = Vec::new();
        let mut prev = "0".repeat(64);
        let mut ids = HashMap::new();
        let mut valid_bytes = 0_u64;
        let mut add_final_newline = false;
        let scanned_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        for (index, chunk) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
            let terminated = chunk.last() == Some(&b'\n');
            if !terminated && !repair_incomplete_tail {
                return Err(JournalError::Corrupt { line: index + 1 });
            }
            let line_bytes = if terminated {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            let line = match std::str::from_utf8(line_bytes) {
                Ok(line) => line,
                Err(_) if !terminated => {
                    if let Some(file) = repair_file.as_ref() {
                        self.repair_incomplete_tail(file, valid_bytes, scanned_bytes)?;
                    } else {
                        let file = OpenOptions::new()
                            .write(true)
                            .open(&self.path)
                            .map_err(io_err)?;
                        Self::truncate_journal_file(&file, valid_bytes)?;
                    }
                    return Ok(events);
                }
                Err(_) => return Err(JournalError::Corrupt { line: index + 1 }),
            };
            if line.trim().is_empty() {
                return Err(JournalError::Corrupt { line: index + 1 });
            }
            let event: JournalEvent = match serde_json::from_str(line) {
                Ok(event) => event,
                Err(_) if !terminated => {
                    // A process may stop while serializing the final record.
                    // Discard only an unterminated invalid tail; never repair a
                    // complete line whose checksum/chain is invalid.
                    if let Some(file) = repair_file.as_ref() {
                        self.repair_incomplete_tail(file, valid_bytes, scanned_bytes)?;
                    } else {
                        let file = OpenOptions::new()
                            .write(true)
                            .open(&self.path)
                            .map_err(io_err)?;
                        Self::truncate_journal_file(&file, valid_bytes)?;
                    }
                    return Ok(events);
                }
                Err(_) => return Err(JournalError::Corrupt { line: index + 1 }),
            };
            if event.sequence != (events.len() as u64 + 1)
                || event.previous_hash != prev
                || event.hash != event_hash(&event)
            {
                return Err(JournalError::Corrupt { line: index + 1 });
            }
            if ids
                .insert(event.idempotency_key.clone(), event.sequence)
                .is_some()
            {
                return Err(JournalError::Corrupt { line: index + 1 });
            }
            prev = event.hash.clone();
            events.push(event);
            valid_bytes += chunk.len() as u64;
            if !terminated {
                add_final_newline = true;
            }
        }
        if add_final_newline {
            if let Some(limit) = maximum_bytes {
                let verified_file = repair_file.as_ref().ok_or(JournalError::InvalidEvent)?;
                self.append_final_newline(verified_file, valid_bytes, limit)?;
            } else {
                let mut file = OpenOptions::new()
                    .append(true)
                    .open(&self.path)
                    .map_err(io_err)?;
                file.write_all(b"\n").map_err(io_err)?;
                file.sync_all().map_err(io_err)?;
            }
        }
        Ok(events)
    }

    fn open_verified_repair_file(
        &self,
        verified_file: &File,
        append: bool,
    ) -> Result<File, JournalError> {
        let verified_metadata = verified_file.metadata().map_err(io_err)?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        if append {
            options.append(true);
        }
        let file = options.open(&self.path).map_err(io_err)?;
        let metadata = file.metadata().map_err(io_err)?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.dev() != verified_metadata.dev()
            || metadata.ino() != verified_metadata.ino()
        {
            return Err(JournalError::InvalidEvent);
        }
        Ok(file)
    }

    fn repair_incomplete_tail(
        &self,
        verified_file: &File,
        valid_bytes: u64,
        expected_bytes: u64,
    ) -> Result<(), JournalError> {
        let file = self.open_verified_repair_file(verified_file, false)?;
        if file.metadata().map_err(io_err)?.len() != expected_bytes {
            return Err(JournalError::InvalidEvent);
        }
        Self::truncate_journal_file(&file, valid_bytes)
    }

    fn append_final_newline(
        &self,
        verified_file: &File,
        expected_bytes: u64,
        maximum_bytes: u64,
    ) -> Result<(), JournalError> {
        let mut file = self.open_verified_repair_file(verified_file, true)?;
        let actual_bytes = file.metadata().map_err(io_err)?.len();
        if actual_bytes > maximum_bytes {
            return Err(JournalError::InboxTooLarge { maximum_bytes });
        }
        if actual_bytes != expected_bytes {
            return Err(JournalError::InvalidEvent);
        }
        if expected_bytes
            .checked_add(1)
            .is_none_or(|repaired_size| repaired_size > maximum_bytes)
        {
            return Err(JournalError::InboxTooLarge { maximum_bytes });
        }
        file.write_all(b"\n").map_err(io_err)?;
        file.sync_all().map_err(io_err)
    }

    fn truncate_journal_file(file: &File, valid_bytes: u64) -> Result<(), JournalError> {
        file.set_len(valid_bytes).map_err(io_err)?;
        file.sync_all().map_err(io_err)
    }

    pub fn append(
        &self,
        idempotency_key: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<AppendResult, JournalError> {
        if idempotency_key.trim().is_empty() || kind.trim().is_empty() {
            return Err(JournalError::InvalidEvent);
        }
        let _lock = FileLock::acquire(&self.lock_path()?)?;
        let events = self.read_locked()?;
        self.append_locked(
            &events,
            idempotency_key,
            kind,
            payload,
            JournalAppendOptions {
                mission_id: None,
                webhook_authority: false,
                attestation_signer: None,
                maximum_bytes: self.maximum_bytes,
            },
        )
    }

    /// Append a webhook event only after the webhook module has authenticated
    /// and validated the incoming delivery. Generic journal APIs cannot write
    /// these reserved event kinds.
    pub(crate) fn append_validated_webhook(
        &self,
        idempotency_key: &str,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<AppendResult, JournalError> {
        self.append_validated_webhook_with_signer(idempotency_key, kind, payload, None)
    }

    /// Append an authenticated webhook and sign its final sequence/hash-chain
    /// identity while the same exclusive journal lock is held.
    pub(crate) fn append_validated_webhook_attested(
        &self,
        idempotency_key: &str,
        kind: &str,
        payload: serde_json::Value,
        signer: &SigningKey,
    ) -> Result<AppendResult, JournalError> {
        self.append_validated_webhook_with_signer(idempotency_key, kind, payload, Some(signer))
    }

    fn append_validated_webhook_with_signer(
        &self,
        idempotency_key: &str,
        kind: &str,
        payload: serde_json::Value,
        signer: Option<&SigningKey>,
    ) -> Result<AppendResult, JournalError> {
        if !webhook::is_supported_journal_kind(kind) {
            return Err(JournalError::InvalidEvent);
        }
        let _lock = FileLock::acquire(&self.lock_path()?)?;
        let maximum_bytes = Some(
            self.maximum_bytes
                .unwrap_or(MAX_WEBHOOK_INBOX_BYTES)
                .min(MAX_WEBHOOK_INBOX_BYTES),
        );
        let events = self.read_locked_with_limit(maximum_bytes)?;
        self.append_locked(
            &events,
            idempotency_key,
            kind,
            payload,
            JournalAppendOptions {
                mission_id: None,
                webhook_authority: true,
                attestation_signer: signer,
                maximum_bytes,
            },
        )
    }

    /// Append a host-authored quarantine disposition for one already-persisted
    /// invalid webhook. The original delivery remains untouched; the source
    /// identity and its derived validation failure are checked under the same
    /// journal lock used for the append.
    pub(crate) fn append_webhook_quarantine(
        &self,
        delivery_id: &str,
        source_sequence: u64,
        source_hash: &str,
    ) -> Result<AppendResult, JournalError> {
        if !webhook::valid_delivery_id_v1(delivery_id)
            || source_sequence == 0
            || !is_lower_hex_sha256(source_hash)
        {
            return Err(JournalError::InvalidEvent);
        }
        let _lock = FileLock::acquire(&self.lock_path()?)?;
        let maximum_bytes = Some(
            self.maximum_bytes
                .unwrap_or(MAX_WEBHOOK_INBOX_BYTES)
                .min(MAX_WEBHOOK_INBOX_BYTES),
        );
        let events = self.read_locked_with_options(maximum_bytes, false)?;
        let source = events
            .get(usize::try_from(source_sequence - 1).map_err(|_| JournalError::InvalidEvent)?)
            .filter(|event| event.sequence == source_sequence)
            .ok_or(JournalError::InvalidEvent)?;
        if source.hash != source_hash
            || source.idempotency_key != format!("forgejo-delivery-v1:{delivery_id}")
            || !webhook::is_supported_journal_kind(&source.kind)
        {
            return Err(JournalError::InvalidEvent);
        }
        let (reason_code, reason_field) = webhook::quarantine_reason_for_invalid_delivery(source)
            .ok_or(JournalError::InvalidEvent)?;
        let idempotency_key = format!("forgejo-webhook-quarantine-v1:{delivery_id}:{source_hash}");
        let payload = serde_json::json!({
            "schema_version": 1,
            "delivery_id": delivery_id,
            "source_sequence": source.sequence,
            "source_event_id": source.event_id,
            "source_event_hash": source.hash,
            "reason_code": reason_code,
            "reason_field": reason_field,
        });
        self.append_locked(
            &events,
            &idempotency_key,
            webhook::WEBHOOK_QUARANTINE_JOURNAL_KIND,
            payload,
            JournalAppendOptions {
                mission_id: None,
                webhook_authority: true,
                attestation_signer: None,
                maximum_bytes,
            },
        )
    }

    fn append_locked(
        &self,
        events: &[JournalEvent],
        idempotency_key: &str,
        kind: &str,
        payload: serde_json::Value,
        options: JournalAppendOptions<'_>,
    ) -> Result<AppendResult, JournalError> {
        if idempotency_key.trim().is_empty() || kind.trim().is_empty() {
            return Err(JournalError::InvalidEvent);
        }
        if is_reserved_mission_event_kind(kind) {
            if options.mission_id.is_none()
                || payload
                    .get("mission_id")
                    .and_then(serde_json::Value::as_str)
                    != options.mission_id
            {
                return Err(JournalError::InvalidEvent);
            }
        } else if options.mission_id.is_some() {
            return Err(JournalError::InvalidEvent);
        }
        if is_reserved_webhook_event_kind(kind) != options.webhook_authority {
            return Err(JournalError::InvalidEvent);
        }
        if let Some(existing) = events
            .iter()
            .find(|event| event.idempotency_key == idempotency_key)
        {
            if existing.kind == kind && existing.payload == payload {
                return Ok(AppendResult::Duplicate(existing.clone()));
            }
            return Err(JournalError::IdempotencyConflict);
        }
        let previous_hash = events
            .last()
            .map(|event| event.hash.clone())
            .unwrap_or_else(|| "0".repeat(64));
        let mut event = JournalEvent {
            sequence: events.len() as u64 + 1,
            event_id: format!("evt-{}", events.len() as u64 + 1),
            idempotency_key: idempotency_key.to_owned(),
            kind: kind.to_owned(),
            payload,
            previous_hash,
            receiver_attestation: None,
            hash: String::new(),
        };
        if let Some(signing_key) = options.attestation_signer {
            let verifying_key = signing_key.verifying_key();
            let key_id = sha256(verifying_key.as_bytes());
            let message = journal_attestation_message(&event, &key_id);
            let signature = signing_key.sign(&message);
            event.receiver_attestation = Some(JournalEventAttestationV1 {
                schema_version: 1,
                algorithm: "ed25519".to_owned(),
                key_id,
                signature_hex: hex::encode(signature.to_bytes()),
            });
        }
        event.hash = event_hash(&event);
        let serialized =
            serde_json::to_vec(&event).map_err(|error| JournalError::Io(error.to_string()))?;
        let append_bytes = u64::try_from(serialized.len())
            .ok()
            .and_then(|length| length.checked_add(1))
            .ok_or_else(|| JournalError::Io("journal append length overflow".to_owned()))?;
        if let Some(maximum_bytes) = options.maximum_bytes {
            let existing_bytes = match fs::symlink_metadata(&self.path) {
                Ok(metadata)
                    if metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.uid() == unsafe { libc::geteuid() }
                        && metadata.nlink() == 1 =>
                {
                    metadata.len()
                }
                Ok(_) => return Err(JournalError::InvalidEvent),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => return Err(io_err(error)),
            };
            if existing_bytes
                .checked_add(append_bytes)
                .is_none_or(|total| total > maximum_bytes)
            {
                return Err(JournalError::InboxTooLarge { maximum_bytes });
            }
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        let mut file_options = OpenOptions::new();
        file_options
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        let mut file = file_options.open(&self.path).map_err(io_err)?;
        if let Some(maximum_bytes) = options.maximum_bytes {
            let metadata = file.metadata().map_err(io_err)?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
            {
                return Err(JournalError::InvalidEvent);
            }
            let current_bytes = metadata.len();
            if current_bytes
                .checked_add(append_bytes)
                .is_none_or(|total| total > maximum_bytes)
            {
                return Err(JournalError::InboxTooLarge { maximum_bytes });
            }
        }
        file.write_all(&serialized).map_err(io_err)?;
        file.write_all(b"\n").map_err(io_err)?;
        file.sync_all().map_err(io_err)?;
        Ok(AppendResult::Appended(event))
    }
}

fn is_reserved_mission_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "mission_generation_allocated_v1"
            | "mission_execution_claim_v1"
            | "mission_execution_lease_acquired_v1"
            | "mission_tool_operation_intent_v1"
            | "mission_tool_operation_result_v1"
            | "mission_verification_cpu_reserved_v1"
            | "mission_verification_cpu_resolved_v1"
            | "mission_merge_attempt_started_v1"
            | "mission_merge_attempt_resolved_v1"
    )
}

fn is_reserved_webhook_event_kind(kind: &str) -> bool {
    webhook::is_reserved_journal_kind(kind)
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_mission_id_v1(value: &str) -> bool {
    value.strip_prefix("mission-v1-").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn event_hash(event: &JournalEvent) -> String {
    let mut value = serde_json::json!({
        "sequence": event.sequence,
        "event_id": event.event_id,
        "idempotency_key": event.idempotency_key,
        "kind": event.kind,
        "payload": event.payload,
        "previous_hash": event.previous_hash,
    });
    if let Some(attestation) = &event.receiver_attestation {
        value
            .as_object_mut()
            .expect("event hash is a JSON object")
            .insert(
                "receiver_attestation".to_owned(),
                serde_json::to_value(attestation).expect("journal event attestation serializes"),
            );
    }
    sha256(&serde_json::to_vec(&value).expect("JSON value serialization is infallible"))
}

/// Verify a journal-row signature against the trusted public keys supplied by
/// the host policy. The key is never loaded from the journal itself.
pub fn verify_journal_event_attestation(
    event: &JournalEvent,
    trusted_keys: &[VerifyingKey],
) -> Result<(), JournalAttestationError> {
    let attestation = event
        .receiver_attestation
        .as_ref()
        .ok_or(JournalAttestationError::Missing)?;
    if attestation.schema_version != 1 || attestation.algorithm != "ed25519" {
        return Err(JournalAttestationError::Unsupported);
    }
    if !is_lower_hex_sha256(&attestation.key_id)
        || attestation.signature_hex.len() != 128
        || !attestation
            .signature_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(JournalAttestationError::InvalidSignature);
    }
    let verifying_key = trusted_keys
        .iter()
        .find(|key| sha256(key.as_bytes()) == attestation.key_id)
        .ok_or(JournalAttestationError::UntrustedKey)?;
    let signature_bytes = hex::decode(&attestation.signature_hex)
        .map_err(|_| JournalAttestationError::InvalidSignature)?;
    let signature_array: [u8; 64] = signature_bytes
        .try_into()
        .map_err(|_| JournalAttestationError::InvalidSignature)?;
    let signature = Signature::from_bytes(&signature_array);
    let message = journal_attestation_message(event, &attestation.key_id);
    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|_| JournalAttestationError::InvalidSignature)
}

fn journal_attestation_message(event: &JournalEvent, key_id: &str) -> Vec<u8> {
    let message = serde_json::json!({
        "domain": JOURNAL_ATTESTATION_DOMAIN_V1,
        "schema_version": 1,
        "key_id": key_id,
        "sequence": event.sequence,
        "event_id": event.event_id,
        "idempotency_key": event.idempotency_key,
        "kind": event.kind,
        "payload": event.payload,
        "previous_hash": event.previous_hash,
    });
    serde_json::to_vec(&message).expect("journal attestation message serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    static TEMP_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let sequence = TEMP_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "agent-forge-test-{}-{}-{sequence}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn parallel_test_temp_directories_are_unique() {
        const DIRECTORY_COUNT: usize = 32;
        let paths = (0..DIRECTORY_COUNT)
            .map(|_| thread::spawn(temp_dir))
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        let mut unique_paths = paths.clone();
        unique_paths.sort();
        unique_paths.dedup();
        assert_eq!(unique_paths.len(), DIRECTORY_COUNT);
        assert!(paths.iter().all(|path| path.is_dir()));

        for path in paths {
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn bounded_tail_repairs_refuse_a_replaced_symlink_path() {
        let dir = temp_dir();
        let journal_path = dir.join("inbox.jsonl");
        let opened_path = dir.join("opened-inbox.jsonl");
        let protected_path = dir.join("protected.txt");
        fs::write(&journal_path, b"partial record").unwrap();
        fs::write(&protected_path, b"preserve this").unwrap();

        let opened = OpenOptions::new().read(true).open(&journal_path).unwrap();
        fs::rename(&journal_path, &opened_path).unwrap();
        std::os::unix::fs::symlink(&protected_path, &journal_path).unwrap();
        let journal = EventJournal::open_with_max_bytes(&journal_path, 1024);

        assert!(
            journal
                .repair_incomplete_tail(&opened, 0, b"partial record".len() as u64)
                .is_err()
        );
        assert!(
            journal
                .append_final_newline(&opened, b"partial record".len() as u64, 1024)
                .is_err()
        );

        assert_eq!(fs::read(&protected_path).unwrap(), b"preserve this");
        assert_eq!(
            fs::metadata(&opened_path).unwrap().len(),
            b"partial record".len() as u64
        );
        assert!(
            fs::symlink_metadata(&journal_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn bounded_replay_reads_a_valid_journal_without_write_permission() {
        let dir = temp_dir();
        let path = dir.join("inbox.jsonl");
        let journal = EventJournal::open_with_max_bytes(&path, 1024);
        journal
            .append("read-only-replay", "task", serde_json::json!({"ok":true}))
            .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();

        assert_eq!(journal.replay().unwrap().len(), 1);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn duration_parser_handles_units_and_errors() {
        assert_eq!(parse_duration_ms("250ms"), Ok(250));
        assert_eq!(parse_duration_ms("2S"), Ok(2_000));
        assert_eq!(parse_duration_ms("3m"), Ok(180_000));
        assert_eq!(parse_duration_ms("0042"), Ok(42));
        assert_eq!(
            parse_duration_ms("-1"),
            Err(ParseDurationError::InvalidNumber)
        );
        assert_eq!(parse_duration_ms(""), Err(ParseDurationError::Empty));
        assert_eq!(
            parse_duration_ms("999999999999999999999999m"),
            Err(ParseDurationError::Overflow)
        );
    }

    #[test]
    fn context_and_mission_state_are_pinned_and_bounded() {
        let context = ContextManifest {
            schema_version: 1,
            source_commit: "abc".into(),
            requirement_sha256: "req".into(),
            token_budget: 100,
            estimated_tokens: 20,
            sources: vec![ContextSource {
                path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                file_sha256: "f".repeat(64),
                provenance: "git:abc".into(),
            }],
        };
        assert!(context.validate("abc", "req", 100).is_ok());
        assert!(context.validate("new-commit", "req", 100).is_err());
        assert!(context.validate("abc", "req", 10).is_err());
        assert!(transition_mission(MissionState::Requested, MissionState::Planned).is_ok());
        assert!(transition_mission(MissionState::Executing, MissionState::Merged).is_err());
    }

    #[test]
    fn stale_events_cancellation_and_expired_or_forbidden_capabilities_are_denied() {
        let mut mission = MissionSnapshot {
            revision: 0,
            state: MissionState::Requested,
        };
        assert_eq!(mission.transition(0, MissionState::Planned), Ok(1));
        assert!(
            mission
                .transition(0, MissionState::Executing)
                .unwrap_err()
                .contains("stale mission revision")
        );
        assert_eq!(mission.state, MissionState::Planned);
        assert_eq!(mission.transition(1, MissionState::Executing), Ok(2));
        assert_eq!(mission.transition(2, MissionState::Cancelled), Ok(3));
        assert!(mission.transition(3, MissionState::Executing).is_err());
        let mut exhausted = MissionSnapshot {
            revision: u64::MAX,
            state: MissionState::Requested,
        };
        assert!(
            exhausted
                .transition(u64::MAX, MissionState::Planned)
                .unwrap_err()
                .contains("revision overflow")
        );
        assert_eq!(exhausted.state, MissionState::Requested);

        let agent = AuthorizationLease {
            subject: "fixture-agent".into(),
            capabilities: vec!["workspace.write".into()],
            expires_at_unix: 100,
        };
        assert!(agent.require("workspace.write", 99).is_ok());
        assert!(
            agent
                .require("merge", 99)
                .unwrap_err()
                .contains("capability denied")
        );
        assert!(
            agent
                .require("workspace.write", 100)
                .unwrap_err()
                .contains("expired")
        );
    }

    #[test]
    fn proof_requires_signature_pass_and_all_fresh_hashes() {
        let evidence = ProofEvidence {
            schema_version: 1,
            requirement_sha256: "r".into(),
            policy_sha256: "p".into(),
            candidate_commit: "c".into(),
            base_commit: "b".into(),
            merge_commit: None,
            verifier_id: "independent-v1".into(),
            environment: "isolated".into(),
            commands: vec![vec!["acceptance".into()]],
            artifact_sha256: vec!["a".into()],
            outcome: Outcome::Pass,
            detail: "checks passed".into(),
        };
        let signed = sign_proof(evidence.clone(), b"test-key").unwrap();
        let expected = ExpectedBinding {
            requirement_sha256: "r".into(),
            policy_sha256: "p".into(),
            candidate_commit: "c".into(),
            base_commit: "b".into(),
            merge_commit: None,
            verifier_id: "independent-v1".into(),
        };
        assert_eq!(verify_proof(Some(&signed), b"test-key", &expected), Ok(()));
        assert_eq!(
            verify_proof(None, b"test-key", &expected),
            Err(GateError::MissingProof)
        );
        let mut stale = expected.clone();
        stale.base_commit = "base-moved".into();
        assert_eq!(
            verify_proof(Some(&signed), b"test-key", &stale),
            Err(GateError::StaleBinding("base"))
        );
        let mut stale_candidate = expected.clone();
        stale_candidate.candidate_commit = "candidate-moved".into();
        assert_eq!(
            verify_proof(Some(&signed), b"test-key", &stale_candidate),
            Err(GateError::StaleBinding("candidate"))
        );
        let mut stale_requirements = expected.clone();
        stale_requirements.requirement_sha256 = "requirements-moved".into();
        assert_eq!(
            verify_proof(Some(&signed), b"test-key", &stale_requirements),
            Err(GateError::StaleBinding("requirements"))
        );
        let mut stale_policy = expected.clone();
        stale_policy.policy_sha256 = "policy-moved".into();
        assert_eq!(
            verify_proof(Some(&signed), b"test-key", &stale_policy),
            Err(GateError::StaleBinding("policy"))
        );
        let mut forged = signed.clone();
        forged.evidence.outcome = Outcome::Pass;
        forged.evidence.detail = "self reported".into();
        assert_eq!(
            verify_proof(Some(&forged), b"test-key", &expected),
            Err(GateError::InvalidSignature)
        );
        let failed = sign_proof(
            ProofEvidence {
                outcome: Outcome::Inconclusive,
                ..evidence
            },
            b"test-key",
        )
        .unwrap();
        assert_eq!(
            verify_proof(Some(&failed), b"test-key", &expected),
            Err(GateError::OutcomeInconclusive)
        );
    }

    fn target_bound_fixture() -> (ProofEvidence, ExpectedBinding, ProofTargetV2) {
        let evidence = ProofEvidence {
            schema_version: 1,
            requirement_sha256: "r".into(),
            policy_sha256: "p".into(),
            candidate_commit: "c".into(),
            base_commit: "b".into(),
            merge_commit: None,
            verifier_id: "independent-v1".into(),
            environment: "isolated".into(),
            commands: vec![vec!["acceptance".into()]],
            artifact_sha256: vec!["a".into()],
            outcome: Outcome::Pass,
            detail: "checks passed".into(),
        };
        let expected = ExpectedBinding {
            requirement_sha256: "r".into(),
            policy_sha256: "p".into(),
            candidate_commit: "c".into(),
            base_commit: "b".into(),
            merge_commit: None,
            verifier_id: "independent-v1".into(),
        };
        let target = ProofTargetV2::new(
            "merge-014-agent-forge-core",
            "http://127.0.0.1:3130",
            "forge-admin",
            "agent-forge-core",
            "main",
        )
        .unwrap();
        (evidence, expected, target)
    }

    #[test]
    fn target_bound_proof_v2_binds_target_and_evidence_without_changing_v1() {
        const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
        let (evidence, expected, target) = target_bound_fixture();

        let v1 = sign_proof(evidence.clone(), KEY).unwrap();
        let v1_json = serde_json::to_value(&v1).unwrap();
        let mut v1_keys = v1_json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        v1_keys.sort_unstable();
        assert_eq!(v1_keys, ["evidence", "mac_sha256"]);
        assert_eq!(
            serde_json::from_value::<SignedProof>(v1_json.clone()).unwrap(),
            v1
        );
        assert_eq!(verify_proof(Some(&v1), KEY, &expected), Ok(()));
        assert!(serde_json::from_value::<TargetBoundProofV2>(v1_json).is_err());

        let proof = sign_target_bound_proof_v2(evidence.clone(), target.clone(), KEY).unwrap();
        assert_eq!(proof.schema_version, TARGET_BOUND_PROOF_V2_SCHEMA_VERSION);
        let v2_json = serde_json::to_value(&proof).unwrap();
        assert!(serde_json::from_value::<SignedProof>(v2_json.clone()).is_err());
        assert_eq!(
            verify_target_bound_proof_v2(Some(&proof), KEY, &target, &expected),
            Ok(())
        );

        for wrong_target in [
            ProofTargetV2 {
                profile_selector: "mission-012-clamp".into(),
                ..target.clone()
            },
            ProofTargetV2 {
                origin: "http://127.0.0.1:3132".into(),
                ..target.clone()
            },
            ProofTargetV2 {
                owner: "another-owner".into(),
                ..target.clone()
            },
            ProofTargetV2 {
                repository: "another-repo".into(),
                ..target.clone()
            },
            ProofTargetV2 {
                base_branch: "release".into(),
                ..target.clone()
            },
        ] {
            assert_eq!(
                verify_target_bound_proof_v2(Some(&proof), KEY, &wrong_target, &expected),
                Err(GateError::WrongTarget)
            );
        }

        let mut changed_target = proof.clone();
        changed_target.target.repository = "another-repo".into();
        assert_eq!(
            verify_target_bound_proof_v2(Some(&changed_target), KEY, &target, &expected),
            Err(GateError::WrongTarget)
        );

        let mut changed_mac = proof.clone();
        changed_mac.target_mac_sha256.replace_range(0..1, "0");
        assert_eq!(
            verify_target_bound_proof_v2(Some(&changed_mac), KEY, &target, &expected),
            Err(GateError::InvalidSignature)
        );
        let mut changed_evidence = proof.clone();
        changed_evidence.evidence.detail = "self reported success".into();
        assert_eq!(
            verify_target_bound_proof_v2(Some(&changed_evidence), KEY, &target, &expected),
            Err(GateError::InvalidSignature)
        );
        let mut stale_expected = expected.clone();
        stale_expected.base_commit = "moved-base".into();
        assert_eq!(
            verify_target_bound_proof_v2(Some(&proof), KEY, &target, &stale_expected),
            Err(GateError::StaleBinding("base"))
        );
        for (binding_name, mutate) in [
            (
                "requirements",
                Box::new(|binding: &mut ExpectedBinding| {
                    binding.requirement_sha256 = "moved-requirements".into();
                }) as Box<dyn Fn(&mut ExpectedBinding)>,
            ),
            (
                "policy",
                Box::new(|binding: &mut ExpectedBinding| {
                    binding.policy_sha256 = "moved-policy".into();
                }),
            ),
            (
                "candidate",
                Box::new(|binding: &mut ExpectedBinding| {
                    binding.candidate_commit = "moved-candidate".into();
                }),
            ),
            (
                "merge result",
                Box::new(|binding: &mut ExpectedBinding| {
                    binding.merge_commit = Some("moved-merge".into());
                }),
            ),
            (
                "verifier identity",
                Box::new(|binding: &mut ExpectedBinding| {
                    binding.verifier_id = "another-verifier".into();
                }),
            ),
        ] {
            let mut stale = expected.clone();
            mutate(&mut stale);
            assert_eq!(
                verify_target_bound_proof_v2(Some(&proof), KEY, &target, &stale),
                Err(GateError::StaleBinding(binding_name))
            );
        }

        let mut wrong_version = proof.clone();
        wrong_version.schema_version = 3;
        assert_eq!(
            verify_target_bound_proof_v2(Some(&wrong_version), KEY, &target, &expected),
            Err(GateError::UnsupportedProofVersion)
        );
        let mut unknown_field = serde_json::to_value(&proof).unwrap();
        unknown_field["unrecognized"] = serde_json::json!(true);
        assert!(serde_json::from_value::<TargetBoundProofV2>(unknown_field).is_err());
        let mut unknown_evidence_field = v2_json.clone();
        unknown_evidence_field["evidence"]["unrecognized"] = serde_json::json!(true);
        assert!(serde_json::from_value::<TargetBoundProofV2>(unknown_evidence_field).is_err());
        let mut unknown_target_field = v2_json.clone();
        unknown_target_field["target"]["unrecognized"] = serde_json::json!(true);
        assert!(serde_json::from_value::<TargetBoundProofV2>(unknown_target_field).is_err());
        let mut missing_version = v2_json.clone();
        missing_version
            .as_object_mut()
            .unwrap()
            .remove("schema_version");
        assert!(serde_json::from_value::<TargetBoundProofV2>(missing_version).is_err());
        assert!(
            ProofTargetV2::new(
                "merge-014-agent-forge-core",
                "http://EXAMPLE.invalid/",
                "forge-admin",
                "agent-forge-core",
                "main",
            )
            .is_err()
        );

        let failed = sign_target_bound_proof_v2(
            ProofEvidence {
                outcome: Outcome::Fail,
                ..evidence
            },
            target.clone(),
            KEY,
        )
        .unwrap();
        assert_eq!(
            verify_target_bound_proof_v2(Some(&failed), KEY, &target, &expected),
            Err(GateError::OutcomeFailed)
        );
    }

    #[test]
    fn target_bound_v2_signer_and_verifier_reject_keys_shorter_than_32_bytes() {
        const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
        let (evidence, expected, target) = target_bound_fixture();
        let proof = sign_target_bound_proof_v2(evidence.clone(), target.clone(), KEY).unwrap();
        let empty_key: &[u8] = b"";
        let short_key = [0_u8; 31];

        for key in [empty_key, &short_key[..]] {
            assert_eq!(
                sign_target_bound_proof_v2(evidence.clone(), target.clone(), key).unwrap_err(),
                "target-bound V2 verifier key must be at least 32 bytes"
            );

            let mut proof_signed_with_short_key = proof.clone();
            proof_signed_with_short_key.target_mac_sha256 =
                target_bound_proof_mac_v2(&target, &proof_signed_with_short_key.evidence, key)
                    .unwrap();
            assert_eq!(
                verify_target_bound_proof_v2(
                    Some(&proof_signed_with_short_key),
                    key,
                    &target,
                    &expected
                ),
                Err(GateError::InvalidVerifierKey)
            );
        }
    }

    #[test]
    fn target_bound_v2_mac_covers_each_target_field() {
        const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
        let (evidence, expected, target) = target_bound_fixture();
        let proof = sign_target_bound_proof_v2(evidence, target.clone(), KEY).unwrap();
        let mutations: [fn(&mut ProofTargetV2); 5] = [
            |changed| changed.profile_selector = "mission-012-clamp".into(),
            |changed| changed.origin = "http://127.0.0.1:3132".into(),
            |changed| changed.owner = "another-owner".into(),
            |changed| changed.repository = "another-repo".into(),
            |changed| changed.base_branch = "release".into(),
        ];

        for mutate in mutations {
            let mut changed_target = target.clone();
            mutate(&mut changed_target);
            let mut changed_proof = proof.clone();
            changed_proof.target = changed_target.clone();

            assert_eq!(
                verify_target_bound_proof_v2(Some(&changed_proof), KEY, &changed_target, &expected),
                Err(GateError::InvalidSignature)
            );
        }
    }

    #[test]
    fn verifier_preserves_failure_and_inconclusive_outcomes() {
        let evidence = ProofEvidence {
            schema_version: 1,
            requirement_sha256: "r".into(),
            policy_sha256: "p".into(),
            candidate_commit: "c".into(),
            base_commit: "b".into(),
            merge_commit: None,
            verifier_id: "independent-v1".into(),
            environment: "isolated".into(),
            commands: vec![vec!["acceptance".into()]],
            artifact_sha256: vec!["a".into()],
            outcome: Outcome::Pass,
            detail: "checks completed".into(),
        };
        let expected = ExpectedBinding {
            requirement_sha256: "r".into(),
            policy_sha256: "p".into(),
            candidate_commit: "c".into(),
            base_commit: "b".into(),
            merge_commit: None,
            verifier_id: "independent-v1".into(),
        };
        let failed = sign_proof(
            ProofEvidence {
                outcome: Outcome::Fail,
                detail: "acceptance failed".into(),
                ..evidence.clone()
            },
            b"test-key",
        )
        .unwrap();
        let inconclusive = sign_proof(
            ProofEvidence {
                outcome: Outcome::Inconclusive,
                detail: "runner unavailable".into(),
                ..evidence
            },
            b"test-key",
        )
        .unwrap();

        assert_eq!(
            verify_proof(Some(&failed), b"test-key", &expected),
            Err(GateError::OutcomeFailed)
        );
        assert_eq!(
            verify_proof(Some(&inconclusive), b"test-key", &expected),
            Err(GateError::OutcomeInconclusive)
        );
    }

    #[test]
    fn journal_deduplicates_replays_and_serializes_concurrent_events() {
        let dir = temp_dir();
        let path = dir.join("events.jsonl");
        let journal = Arc::new(EventJournal::open(&path));
        assert!(matches!(
            journal.append(
                "hook-1",
                "forgejo.webhook.pull_request",
                serde_json::json!({"n":1})
            ),
            Ok(AppendResult::Appended(_))
        ));
        assert!(matches!(
            journal.append(
                "hook-1",
                "forgejo.webhook.pull_request",
                serde_json::json!({"n":1})
            ),
            Ok(AppendResult::Duplicate(_))
        ));
        assert_eq!(
            journal.append(
                "hook-1",
                "forgejo.webhook.pull_request",
                serde_json::json!({"n":2})
            ),
            Err(JournalError::IdempotencyConflict)
        );
        let barrier = Arc::new(Barrier::new(9));
        let mut joins = Vec::new();
        for i in 0..8 {
            let j = Arc::clone(&journal);
            let b = Arc::clone(&barrier);
            joins.push(thread::spawn(move || {
                b.wait();
                j.append(&format!("parallel-{i}"), "task", serde_json::json!({"i":i}))
                    .unwrap();
            }));
        }
        barrier.wait();
        for join in joins {
            join.join().unwrap();
        }
        let reopened = EventJournal::open(&path);
        let events = reopened.replay().unwrap();
        assert_eq!(events.len(), 9);
        assert_eq!(
            events
                .iter()
                .map(|e| &e.previous_hash)
                .skip(1)
                .zip(events.iter().map(|e| &e.hash))
                .all(|(a, b)| a == b),
            true
        );
        let valid_len = fs::metadata(&path).unwrap().len();
        let mut interrupted = OpenOptions::new().append(true).open(&path).unwrap();
        interrupted.write_all(b"{\"sequence\":").unwrap();
        interrupted.sync_all().unwrap();
        assert_eq!(reopened.replay().unwrap().len(), 9);
        assert_eq!(fs::metadata(&path).unwrap().len(), valid_len);
        assert!(matches!(
            reopened.append(
                "after-interruption",
                "mission.resume",
                serde_json::json!({"resumed":true})
            ),
            Ok(AppendResult::Appended(JournalEvent { sequence: 10, .. }))
        ));
        assert_eq!(EventJournal::open(&path).replay().unwrap().len(), 10);
        let mut damaged = fs::read_to_string(&path).unwrap();
        damaged = damaged.replace("\"kind\":\"task\"", "\"kind\":\"forged\"");
        fs::write(&path, damaged).unwrap();
        assert!(matches!(
            reopened.replay(),
            Err(JournalError::Corrupt { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }
}
