use agent_forge_core::{
    ExpectedBinding, GateError, Outcome, ProofEvidence, SignedProof,
    forgejo::{
        BaseBranchScope, MergePreflightContextV1, MergePreflightError, PullRequestSnapshotV1,
        validate_merge_preflight,
    },
    sign_proof,
};

const KEY: &[u8] = b"test verifier key only";

fn scope() -> BaseBranchScope {
    BaseBranchScope::new(
        "http://127.0.0.1:3130",
        "forge-admin",
        "agent-forge-core",
        "main",
    )
    .unwrap()
}

fn expected() -> ExpectedBinding {
    ExpectedBinding {
        requirement_sha256: "a".repeat(64),
        policy_sha256: "b".repeat(64),
        candidate_commit: "1".repeat(40),
        base_commit: "2".repeat(40),
        merge_commit: None,
        verifier_id: "independent-verifier".into(),
    }
}

fn proof(outcome: Outcome) -> SignedProof {
    sign_proof(
        ProofEvidence {
            schema_version: 1,
            requirement_sha256: "a".repeat(64),
            policy_sha256: "b".repeat(64),
            candidate_commit: "1".repeat(40),
            base_commit: "2".repeat(40),
            merge_commit: None,
            verifier_id: "independent-verifier".into(),
            environment: "isolated fixture".into(),
            commands: vec![vec!["cargo".into(), "test".into()]],
            artifact_sha256: vec!["c".repeat(64)],
            outcome,
            detail: "verified fixture".into(),
        },
        KEY,
    )
    .unwrap()
}

fn snapshot() -> PullRequestSnapshotV1 {
    PullRequestSnapshotV1 {
        origin: "http://127.0.0.1:3130".into(),
        owner: "forge-admin".into(),
        repository: "agent-forge-core".into(),
        number: 23,
        state: "open".into(),
        merged: false,
        draft: false,
        mergeable: Some(true),
        base_branch: "main".into(),
        base_sha: "2".repeat(40),
        head_sha: "1".repeat(40),
    }
}

fn forgejo_response(number: u64) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "state": "open",
        "merged": false,
        "draft": false,
        "mergeable": true,
        "html_url": format!("http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/{number}"),
        "base": {
            "ref": "main",
            "sha": "2222222222222222222222222222222222222222",
            "repo": {"full_name": "forge-admin/agent-forge-core"}
        },
        "head": {
            "sha": "1111111111111111111111111111111111111111",
            "repo": {"full_name": "forge-admin/agent-forge-core"}
        }
    })
}

fn context() -> MergePreflightContextV1 {
    MergePreflightContextV1 {
        pull_request_number: 23,
        execution_identity: "fixture-agent".into(),
        expected: expected(),
    }
}

fn preflight(
    current: &PullRequestSnapshotV1,
    proof: Option<&SignedProof>,
    context: &MergePreflightContextV1,
) -> Result<(), MergePreflightError> {
    validate_merge_preflight(&scope(), current, proof, KEY, context)
}

#[test]
fn fresh_open_pull_request_and_passing_proof_are_ready_for_later_review() {
    assert_eq!(
        preflight(&snapshot(), Some(&proof(Outcome::Pass)), &context()),
        Ok(())
    );
}

#[test]
fn proof_for_an_old_base_is_refused() {
    let mut current = snapshot();
    current.base_sha = "3".repeat(40);
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::StaleBinding("base"))
    );
}

#[test]
fn proof_for_an_old_candidate_is_refused() {
    let mut current = snapshot();
    current.head_sha = "4".repeat(40);
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::StaleBinding("candidate"))
    );
}

#[test]
fn closed_pull_request_is_refused() {
    let mut current = snapshot();
    current.state = "closed".into();
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::PullRequestNotOpen)
    );
}

#[test]
fn already_merged_pull_request_is_refused() {
    let mut current = snapshot();
    current.merged = true;
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::AlreadyMerged)
    );
}

#[test]
fn draft_pull_request_is_refused() {
    let mut current = snapshot();
    current.draft = true;
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::DraftPullRequest)
    );
}

#[test]
fn unknown_or_negative_mergeability_is_refused() {
    let mut current = snapshot();
    current.mergeable = None;
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::MergeabilityUnknown)
    );

    let mut current = snapshot();
    current.mergeable = Some(false);
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::NotMergeable)
    );
}

#[test]
fn wrong_repository_or_base_branch_are_refused() {
    let mut current = snapshot();
    current.repository = "other-repo".into();
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::WrongScope)
    );

    let mut current = snapshot();
    current.base_branch = "release".into();
    assert_eq!(
        preflight(&current, Some(&proof(Outcome::Pass)), &context()),
        Err(MergePreflightError::WrongScope)
    );
}

#[test]
fn wrong_pull_request_number_is_refused() {
    let mut context = context();
    context.pull_request_number = 24;
    assert_eq!(
        preflight(&snapshot(), Some(&proof(Outcome::Pass)), &context),
        Err(MergePreflightError::WrongPullRequest)
    );
}

#[test]
fn verifier_cannot_also_be_the_execution_identity() {
    let mut context = context();
    context.expected.verifier_id = "fixture-agent".into();
    assert_eq!(
        preflight(&snapshot(), Some(&proof(Outcome::Pass)), &context),
        Err(MergePreflightError::VerifierNotIndependent)
    );
}

#[test]
fn missing_or_nonpassing_proof_never_becomes_ready() {
    assert_eq!(
        preflight(&snapshot(), None, &context()),
        Err(MergePreflightError::Proof(GateError::MissingProof))
    );
    assert_eq!(
        preflight(&snapshot(), Some(&proof(Outcome::Fail)), &context()),
        Err(MergePreflightError::Proof(GateError::OutcomeFailed))
    );
    assert_eq!(
        preflight(&snapshot(), Some(&proof(Outcome::Inconclusive)), &context()),
        Err(MergePreflightError::Proof(GateError::OutcomeInconclusive))
    );
}

#[test]
fn api_snapshot_parser_binds_target_base_and_allows_fork_head() {
    let mut response = forgejo_response(23);
    response["head"]["repo"]["full_name"] = serde_json::json!("contributor/agent-forge-core");
    let current = PullRequestSnapshotV1::from_forgejo_response(
        "http://127.0.0.1:3130/",
        "forge-admin",
        "agent-forge-core",
        23,
        &response,
    )
    .unwrap();
    assert_eq!(current, snapshot());

    let mut response = forgejo_response(23);
    response["base"]["repo"]["full_name"] = serde_json::json!("other-owner/other-repo");
    assert_eq!(
        PullRequestSnapshotV1::from_forgejo_response(
            "http://127.0.0.1:3130",
            "forge-admin",
            "agent-forge-core",
            23,
            &response,
        ),
        Err(MergePreflightError::WrongScope)
    );
}

#[test]
fn api_snapshot_parser_rejects_wrong_pull_request_and_html_url() {
    assert_eq!(
        PullRequestSnapshotV1::from_forgejo_response(
            "http://127.0.0.1:3130",
            "forge-admin",
            "agent-forge-core",
            23,
            &forgejo_response(24),
        ),
        Err(MergePreflightError::WrongPullRequest)
    );

    let mut response = forgejo_response(23);
    response["html_url"] =
        serde_json::json!("http://evil.invalid/forge-admin/agent-forge-core/pulls/23");
    assert_eq!(
        PullRequestSnapshotV1::from_forgejo_response(
            "http://127.0.0.1:3130",
            "forge-admin",
            "agent-forge-core",
            23,
            &response,
        ),
        Err(MergePreflightError::WrongScope)
    );
}
