use agent_forge_core::{
    AppendResult, ContextManifest, ContextSource, EventJournal,
    agent_runtime::{
        ContextCitationV1, ContextPacketV1, MissionExecutionJournalV1, RuntimeBudgetV1,
        RuntimeCapabilitiesV1,
    },
    forgejo::{
        BaseBranchScope, PullRequestReviewScopeV1, PullRequestReviewV1,
        PullRequestReviewsSnapshotV1, RequirementSourcePullRequestV1, ReviewUserV1,
    },
    mission_admission::{
        IssueSnapshotV1, MissionAdmissionBindingV2, MissionAdmissionBindingV3,
        MissionAdmissionSourceV1, MissionArbitrationPolicyScopeV1, MissionArbitrationPolicyV1,
        MissionArbitrationSourceV1, MissionIssueStateV1, ProtectedBranchHeadV1, SourceFailureV1,
        admit_and_claim_mission_v2, admit_reconciled_issue_v3,
    },
    sha256,
    webhook::{ingest_forgejo_delivery_attested, read_journaled_webhook_event},
};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<sha2::Sha256>;
const ISSUE: u64 = 41;
const PR: u64 = 77;
const BASE: &str = "c9d4bd2a70d10f4bc5d233d37318a8b0d42a0001";
const HEAD: &str = "e7a0672a04f51c3f430a57780a2de5fb2fdf0958";
const MERGE: &str = "b3a0672a04f51c3f430a57780a2de5fb2fdf0958";
const SECRET: &[u8] = b"local-webhook-fixture-secret";
const REQUIREMENTS: &[u8] = br#"{"schema_version":3,"id":"REQ-001","revision":2,"source_issue_number":41,"source_pull_request_number":77,"request":"Implement a duration parser.","acceptance":["250ms parses to 250 milliseconds."],"ambiguities":[],"decisions":[{"ambiguity":"Bare integer units","resolution":"Bare integers mean milliseconds.","rationale":"This preserves the existing CLI convention."}]}"#;

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "requirement-arbitration-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scope() -> BaseBranchScope {
    BaseBranchScope::new(
        "http://127.0.0.1:3130",
        "forge-admin",
        "agent-forge-core",
        "main",
    )
    .unwrap()
}

fn review_scope() -> PullRequestReviewScopeV1 {
    PullRequestReviewScopeV1::new(
        "http://127.0.0.1:3130",
        "forge-admin",
        "agent-forge-core",
        PR,
    )
    .unwrap()
}

fn policy_bytes() -> Vec<u8> {
    serde_json::to_vec(&MissionArbitrationPolicyV1 {
        schema_version: 1,
        revision: 4,
        scope: MissionArbitrationPolicyScopeV1 {
            origin: "http://127.0.0.1:3130".into(),
            owner: "forge-admin".into(),
            repository: "agent-forge-core".into(),
            branch: "main".into(),
        },
        required_approvals: 1,
        authorized_reviewer_ids: vec![9001],
    })
    .unwrap()
}

fn context() -> ContextPacketV1 {
    let requirement_sha256 = sha256(REQUIREMENTS);
    let text = "Read src/lib.rs:1 for the parser API.".to_owned();
    ContextPacketV1 {
        manifest: ContextManifest {
            schema_version: 1,
            source_commit: BASE.into(),
            requirement_sha256,
            token_budget: 100,
            estimated_tokens: 15,
            sources: vec![ContextSource {
                path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 1,
                file_sha256: "b".repeat(64),
                provenance: format!("git:{BASE}:src/lib.rs#L1"),
            }],
        },
        text: text.clone(),
        citations: vec![ContextCitationV1 {
            text_start_byte: 0,
            text_end_byte: text.len(),
            source_index: 0,
        }],
    }
}

fn capabilities() -> RuntimeCapabilitiesV1 {
    RuntimeCapabilitiesV1 {
        read_paths: vec!["src/lib.rs".into()],
        write_paths: vec!["src/lib.rs".into()],
        tools: vec!["workspace.read_text".into()],
        budget: RuntimeBudgetV1 {
            max_tool_calls: 1,
            timeout_ms: 1000,
            max_output_tokens: 100,
        },
    }
}

fn binding() -> MissionAdmissionBindingV3 {
    MissionAdmissionBindingV3 {
        schema_version: 3,
        issue_number: ISSUE,
        requirement_id: "REQ-001".into(),
        requirement_revision: 2,
        allocation_id: "arbitration-admission-001".into(),
    }
}

fn review(state: &str, official: bool, stale: bool, reviewer: u64) -> PullRequestReviewV1 {
    PullRequestReviewV1 {
        id: 123,
        user: Some(ReviewUserV1 { id: reviewer }),
        state: state.into(),
        commit_id: HEAD.into(),
        pull_request_url: review_scope().pull_request_url(),
        stale,
        official,
        dismissed: false,
    }
}

struct FixtureSource {
    reviews: Vec<PullRequestReviewV1>,
    requirement_at_merge: Vec<u8>,
    pull_request_merged: bool,
    pull_request_base_branch: String,
    reported_review_total_count: Option<u64>,
    change_pr_after_review_read: bool,
    add_review_after_first_snapshot: bool,
    pr_reads: Cell<u32>,
    review_reads: Cell<u32>,
    reads: Cell<u32>,
}

impl FixtureSource {
    fn record(&self) {
        self.reads.set(self.reads.get() + 1);
    }

    fn authorized() -> Self {
        Self {
            reviews: vec![review("APPROVED", true, false, 9001)],
            requirement_at_merge: REQUIREMENTS.to_vec(),
            pull_request_merged: true,
            pull_request_base_branch: "main".into(),
            reported_review_total_count: None,
            change_pr_after_review_read: false,
            add_review_after_first_snapshot: false,
            pr_reads: Cell::new(0),
            review_reads: Cell::new(0),
            reads: Cell::new(0),
        }
    }
}

impl MissionAdmissionSourceV1 for FixtureSource {
    fn read_issue(
        &self,
        scope: &BaseBranchScope,
        issue_number: u64,
    ) -> Result<IssueSnapshotV1, SourceFailureV1> {
        self.record();
        Ok(IssueSnapshotV1 {
            number: issue_number,
            html_url: format!(
                "{}/{}/{}/issues/{issue_number}",
                scope.origin(),
                scope.owner(),
                scope.repository()
            ),
            state: MissionIssueStateV1::Open,
            work_item_content: None,
        })
    }

    fn read_protected_branch_head(
        &self,
        scope: &BaseBranchScope,
    ) -> Result<ProtectedBranchHeadV1, SourceFailureV1> {
        self.record();
        Ok(ProtectedBranchHeadV1 {
            branch: scope.branch().into(),
            commit: BASE.into(),
            protected: true,
        })
    }

    fn read_requirement_file_at(
        &self,
        _scope: &BaseBranchScope,
        commit: &str,
        path: &str,
    ) -> Result<Vec<u8>, SourceFailureV1> {
        self.record();
        if path != "requirements/REQ-001.json" {
            return Err(SourceFailureV1::NotFound);
        }
        match commit {
            BASE => Ok(REQUIREMENTS.to_vec()),
            MERGE => Ok(self.requirement_at_merge.clone()),
            _ => Err(SourceFailureV1::NotFound),
        }
    }

    fn list_requirement_paths_at(
        &self,
        _scope: &BaseBranchScope,
        commit: &str,
    ) -> Result<Vec<String>, SourceFailureV1> {
        self.record();
        if commit == BASE {
            Ok(vec!["requirements/REQ-001.json".into()])
        } else {
            Err(SourceFailureV1::NotFound)
        }
    }
}

impl MissionArbitrationSourceV1 for FixtureSource {
    fn read_requirement_source_pull_request(
        &self,
        _scope: &BaseBranchScope,
        number: u64,
    ) -> Result<RequirementSourcePullRequestV1, SourceFailureV1> {
        self.record();
        let read_number = self.pr_reads.get();
        self.pr_reads.set(read_number + 1);
        Ok(RequirementSourcePullRequestV1 {
            scope: PullRequestReviewScopeV1::new(
                "http://127.0.0.1:3130",
                "forge-admin",
                "agent-forge-core",
                number,
            )
            .unwrap(),
            merged: self.pull_request_merged,
            base_branch: self.pull_request_base_branch.clone(),
            head_sha: if self.change_pr_after_review_read && read_number > 0 {
                BASE.into()
            } else {
                HEAD.into()
            },
            merge_commit_sha: MERGE.into(),
        })
    }

    fn read_requirement_source_reviews(
        &self,
        _scope: &BaseBranchScope,
        _number: u64,
    ) -> Result<PullRequestReviewsSnapshotV1, SourceFailureV1> {
        self.record();
        let read_number = self.review_reads.get();
        self.review_reads.set(read_number + 1);
        let mut reviews = self.reviews.clone();
        if self.add_review_after_first_snapshot && read_number > 0 {
            let mut comment = review("COMMENT", true, false, 77);
            comment.id = 124;
            reviews.push(comment);
        }
        Ok(PullRequestReviewsSnapshotV1 {
            scope: review_scope(),
            total_count: self
                .reported_review_total_count
                .unwrap_or(reviews.len() as u64),
            reviews,
        })
    }
}

impl agent_forge_core::webhook::reconcile::ForgejoCurrentState for FixtureSource {
    fn read_current(
        &self,
        resource: &agent_forge_core::webhook::reconcile::ResourceRef,
    ) -> Result<Value, agent_forge_core::webhook::reconcile::ApiReadError> {
        self.record();
        Ok(json!({
            "number": resource.number().unwrap_or_default(),
            "state": "open",
            "updated_at": "2026-10-27T09:00:00Z",
            "html_url": self.expected_html_url(resource),
        }))
    }

    fn expected_html_url(
        &self,
        resource: &agent_forge_core::webhook::reconcile::ResourceRef,
    ) -> String {
        format!(
            "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/{}",
            resource.number().unwrap_or_default()
        )
    }
}

fn event(scratch: &ScratchDir) -> agent_forge_core::webhook::JournaledWebhookEventV1 {
    let journal = EventJournal::open_webhook_inbox(scratch.0.join("inbox.jsonl"));
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": { "full_name": "forge-admin/agent-forge-core" },
        "issue": { "number": ISSUE, "state": "open" }
    }))
    .unwrap();
    let mut mac = HmacSha256::new_from_slice(SECRET).unwrap();
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let key = SigningKey::from_bytes(&[17; 32]);
    match ingest_forgejo_delivery_attested(
        &journal,
        SECRET,
        &key,
        "arbitration-delivery-001",
        "issues",
        &signature,
        &body,
    )
    .unwrap()
    {
        AppendResult::Appended(_) => {
            read_journaled_webhook_event(&journal, "arbitration-delivery-001").unwrap()
        }
        AppendResult::Duplicate(_) => unreachable!(),
    }
}

fn admit(
    source: &FixtureSource,
    scratch: &ScratchDir,
) -> Result<
    agent_forge_core::mission_admission::WebhookMissionAdmissionReceiptV3,
    agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1,
> {
    let policy = policy_bytes();
    admit_reconciled_issue_v3(
        &event(scratch),
        source,
        &scope(),
        &binding(),
        &sha256(REQUIREMENTS),
        &sha256(&policy),
        &policy,
        source,
        context(),
        "workspace-41",
        capabilities(),
        &MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl")),
    )
}

#[test]
fn merged_requirement_pr_and_current_authorized_review_are_recorded_before_admission() {
    let scratch = ScratchDir::new();
    let source = FixtureSource::authorized();
    let receipt = admit(&source, &scratch).unwrap();

    let arbitration = receipt.admission.arbitration;
    assert_eq!(arbitration.source_pull_request_number, PR);
    assert_eq!(arbitration.source_merge_commit_sha, MERGE);
    assert_eq!(arbitration.source_head_sha, HEAD);
    assert_eq!(arbitration.requirements_sha256, sha256(REQUIREMENTS));
    assert_eq!(arbitration.policy_revision, 4);
    assert_eq!(arbitration.reviewer_ids, vec![9001]);
    assert!(
        receipt
            .admission
            .admission
            .request
            .requirement_arbitration
            .is_some()
    );
    assert_eq!(
        receipt.admission.admission.allocation.execution_generation,
        1
    );

    let mut forged_request = receipt.admission.admission.request.clone();
    forged_request.requirement_arbitration = None;
    assert_eq!(
        forged_request.validate().unwrap_err().code,
        "requirements_arbitration_missing"
    );
}

#[test]
fn missing_authorized_review_refuses_before_generation_allocation() {
    let scratch = ScratchDir::new();
    let source = FixtureSource {
        reviews: vec![review("APPROVED", true, false, 11)],
        ..FixtureSource::authorized()
    };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_arbitration_insufficient")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stale_or_missing_pr_merge_bytes_refuse_before_generation_allocation() {
    let scratch = ScratchDir::new();
    let source = FixtureSource { requirement_at_merge: br#"{"schema_version":3,"id":"REQ-001","revision":2,"source_issue_number":41,"source_pull_request_number":77,"request":"Different content.","acceptance":["250ms parses to 250 milliseconds."],"ambiguities":[],"decisions":[{"ambiguity":"Bare integer units","resolution":"Bare integers mean milliseconds.","rationale":"This preserves the existing CLI convention."}]}"#.to_vec(), ..FixtureSource::authorized() };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_source_mismatch")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn incomplete_review_pages_refuse_even_when_the_visible_page_has_approval() {
    let scratch = ScratchDir::new();
    let source = FixtureSource {
        reported_review_total_count: Some(2),
        ..FixtureSource::authorized()
    };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_review_list_incomplete")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pr_ref_change_during_review_pagination_makes_the_decision_inconclusive() {
    let scratch = ScratchDir::new();
    let source = FixtureSource {
        change_pr_after_review_read: true,
        ..FixtureSource::authorized()
    };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_review_snapshot_changed")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stale_review_on_an_old_pr_head_does_not_authorize_the_decision() {
    let scratch = ScratchDir::new();
    let mut stale = review("APPROVED", true, true, 9001);
    stale.commit_id = BASE.into();
    let source = FixtureSource {
        reviews: vec![stale],
        ..FixtureSource::authorized()
    };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_arbitration_insufficient")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn merged_pr_review_marked_stale_after_merge_still_requires_exact_reviewed_head() {
    let scratch = ScratchDir::new();
    let source = FixtureSource {
        reviews: vec![review("APPROVED", true, true, 9001)],
        ..FixtureSource::authorized()
    };

    let receipt = admit(&source, &scratch).unwrap();

    assert_eq!(receipt.admission.arbitration.source_head_sha, HEAD);
    assert_eq!(receipt.admission.arbitration.reviewer_ids, vec![9001]);
    assert_eq!(receipt.admission.arbitration.approved_review_ids, vec![123]);
}

#[test]
fn changed_review_pages_between_complete_reads_refuse_admission() {
    let scratch = ScratchDir::new();
    let source = FixtureSource {
        add_review_after_first_snapshot: true,
        ..FixtureSource::authorized()
    };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_review_snapshot_changed")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stale_policy_digest_refuses_before_any_generation_is_allocated() {
    let scratch = ScratchDir::new();
    let source = FixtureSource::authorized();
    let policy = policy_bytes();
    let error = admit_reconciled_issue_v3(
        &event(&scratch),
        &source,
        &scope(),
        &binding(),
        &sha256(REQUIREMENTS),
        &"0".repeat(64),
        &policy,
        &source,
        context(),
        "workspace-41",
        capabilities(),
        &MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl")),
    )
    .unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_arbitration_hash_invalid")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn host_arbitration_policy_for_another_fixed_scope_refuses_before_allocation() {
    let scratch = ScratchDir::new();
    let source = FixtureSource::authorized();
    let mut policy: MissionArbitrationPolicyV1 = serde_json::from_slice(&policy_bytes()).unwrap();
    policy.scope.repository = "another-repository".into();
    let wrong_scope_policy = serde_json::to_vec(&policy).unwrap();
    let journal = MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"));

    let error = admit_reconciled_issue_v3(
        &event(&scratch),
        &source,
        &scope(),
        &binding(),
        &sha256(REQUIREMENTS),
        &sha256(&wrong_scope_policy),
        &wrong_scope_policy,
        &source,
        context(),
        "workspace-41",
        capabilities(),
        &journal,
    )
    .unwrap_err();

    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_arbitration_invalid")
    );
    assert!(journal.replay().unwrap().is_empty());
}

#[test]
fn schema_v2_admission_cannot_bypass_the_schema_v3_review_gate() {
    let scratch = ScratchDir::new();
    let source = FixtureSource::authorized();
    let v2_binding = MissionAdmissionBindingV2 {
        schema_version: 2,
        issue_number: ISSUE,
        requirement_id: "REQ-001".into(),
        requirement_revision: 2,
        allocation_id: "v2-cannot-admit-v3".into(),
    };
    let error = admit_and_claim_mission_v2(
        &scope(),
        &v2_binding,
        &source,
        context(),
        "workspace-41",
        capabilities(),
        &MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl")),
    )
    .unwrap_err();
    assert_eq!(error.code, "mission_requirement_binding_required");
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unmerged_source_pr_refuses_even_when_its_review_is_approved() {
    let scratch = ScratchDir::new();
    let source = FixtureSource {
        pull_request_merged: false,
        ..FixtureSource::authorized()
    };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_source_pr_invalid")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn duplicated_review_identity_refuses_the_complete_snapshot() {
    let scratch = ScratchDir::new();
    let duplicate = review("COMMENT", true, false, 12);
    let source = FixtureSource {
        reviews: vec![review("APPROVED", true, false, 9001), duplicate],
        ..FixtureSource::authorized()
    };
    let error = admit(&source, &scratch).unwrap_err();
    assert!(
        matches!(error, agent_forge_core::mission_admission::WebhookMissionAdmissionErrorV1::Admission(ref e) if e.code == "mission_requirement_arbitration_invalid")
    );
    assert!(
        MissionExecutionJournalV1::open(scratch.0.join("execution.jsonl"))
            .replay()
            .unwrap()
            .is_empty()
    );
}
