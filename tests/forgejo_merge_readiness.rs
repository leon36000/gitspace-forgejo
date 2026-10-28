use agent_forge_core::forgejo::{
    BaseBranchScope, MergeApprovalError, MergeApprovalPolicyV1, MergePreflightContextV1,
    MergePreflightError, MergeReadinessDecisionV1, MergeReadinessError,
    PullRequestReviewBaseBindingV1, PullRequestReviewBaseBindingsSnapshotV1,
    PullRequestReviewScopeV1, PullRequestReviewV1, PullRequestReviewsSnapshotV1, ReviewUserV1,
    evaluate_merge_readiness,
};
use agent_forge_core::{ExpectedBinding, Outcome, Policy, ProofEvidence, SignedProof, sign_proof};

const KEY: &[u8] = b"fixture verifier key";
const EXECUTION_REVIEWER_ID: u64 = 9;

fn expected(policy_sha256: String) -> ExpectedBinding {
    ExpectedBinding {
        requirement_sha256: "a".repeat(64),
        policy_sha256,
        candidate_commit: "1".repeat(40),
        base_commit: "2".repeat(40),
        merge_commit: None,
        verifier_id: "independent-verifier".into(),
    }
}

fn proof(expected: &ExpectedBinding) -> SignedProof {
    sign_proof(
        ProofEvidence {
            schema_version: 1,
            requirement_sha256: expected.requirement_sha256.clone(),
            policy_sha256: expected.policy_sha256.clone(),
            candidate_commit: expected.candidate_commit.clone(),
            base_commit: expected.base_commit.clone(),
            merge_commit: None,
            verifier_id: expected.verifier_id.clone(),
            environment: "isolated fixture".into(),
            commands: vec![vec!["cargo".into(), "test".into()]],
            artifact_sha256: vec!["c".repeat(64)],
            outcome: Outcome::Pass,
            detail: "verified fixture".into(),
        },
        KEY,
    )
    .unwrap()
}

fn policy_bytes(verifier_id: &str) -> Vec<u8> {
    serde_json::to_vec(&Policy {
        schema_version: 1,
        revision: 4,
        verifier_id: verifier_id.into(),
        environment: "isolated fixture".into(),
        acceptance_command: vec!["cargo".into(), "test".into()],
        commands: vec![vec!["cargo".into(), "test".into()]],
        protected_criteria: vec!["requirements/**".into(), "policy/**".into()],
        timeout_seconds: 90,
        merge_approval_policy: Some(MergeApprovalPolicyV1 {
            schema_version: 1,
            required_approvals: 1,
            authorized_reviewer_ids: vec![42, 43],
        }),
    })
    .unwrap()
}

fn branch_scope() -> BaseBranchScope {
    BaseBranchScope::new("https://forgejo.example/", "acme", "project", "main").unwrap()
}

fn review_scope(number: u64) -> PullRequestReviewScopeV1 {
    PullRequestReviewScopeV1::new("https://forgejo.example", "acme", "project", number).unwrap()
}

fn context(policy_bytes: &[u8]) -> MergePreflightContextV1 {
    MergePreflightContextV1 {
        pull_request_number: 23,
        execution_identity: "fixture-agent".into(),
        expected: expected(agent_forge_core::sha256(policy_bytes)),
    }
}

fn pull_request() -> agent_forge_core::forgejo::PullRequestSnapshotV1 {
    agent_forge_core::forgejo::PullRequestSnapshotV1 {
        origin: "https://forgejo.example".into(),
        owner: "acme".into(),
        repository: "project".into(),
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

fn reviews(number: u64, commit_id: &str) -> PullRequestReviewsSnapshotV1 {
    PullRequestReviewsSnapshotV1 {
        scope: review_scope(number),
        total_count: 1,
        reviews: vec![PullRequestReviewV1 {
            id: 17,
            user: Some(ReviewUserV1 { id: 42 }),
            state: "APPROVED".into(),
            commit_id: commit_id.into(),
            pull_request_url: review_scope(23).pull_request_url(),
            stale: false,
            official: true,
            dismissed: false,
        }],
    }
}

fn evaluate(
    current: &agent_forge_core::forgejo::PullRequestSnapshotV1,
    review_snapshot: &PullRequestReviewsSnapshotV1,
    context: &MergePreflightContextV1,
    proof: &SignedProof,
    policy_bytes: &[u8],
) -> Result<MergeReadinessDecisionV1, MergeReadinessError> {
    let review_base_bindings = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: review_snapshot.scope.clone(),
        bindings: review_snapshot
            .reviews
            .iter()
            .filter(|review| {
                review.state == "APPROVED"
                    && review.official
                    && !review.stale
                    && !review.dismissed
                    && review.commit_id == current.head_sha
            })
            .map(|review| PullRequestReviewBaseBindingV1 {
                review_id: review.id,
                base_branch: current.base_branch.clone(),
                base_commit: current.base_sha.clone(),
            })
            .collect(),
    };
    evaluate_with_review_base_bindings(
        current,
        review_snapshot,
        &review_base_bindings,
        context,
        proof,
        policy_bytes,
    )
}

fn evaluate_with_review_base_bindings(
    current: &agent_forge_core::forgejo::PullRequestSnapshotV1,
    review_snapshot: &PullRequestReviewsSnapshotV1,
    review_base_bindings: &PullRequestReviewBaseBindingsSnapshotV1,
    context: &MergePreflightContextV1,
    proof: &SignedProof,
    policy_bytes: &[u8],
) -> Result<MergeReadinessDecisionV1, MergeReadinessError> {
    evaluate_merge_readiness(
        &branch_scope(),
        current,
        review_snapshot,
        review_base_bindings,
        Some(proof),
        KEY,
        context,
        EXECUTION_REVIEWER_ID,
        policy_bytes,
    )
}

#[test]
fn readiness_binds_proof_current_pull_request_and_approvals_into_one_decision() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();
    let review_snapshot = reviews(23, &current.head_sha);

    let decision = evaluate(&current, &review_snapshot, &context, &proof, &policy_bytes)
        .expect("fresh proof and current approval make this PR ready");

    assert_eq!(decision.pull_request_number, 23);
    assert_eq!(decision.origin, "https://forgejo.example");
    assert_eq!(decision.owner, "acme");
    assert_eq!(decision.repository, "project");
    assert_eq!(decision.base_branch, current.base_branch);
    assert_eq!(decision.base_commit, current.base_sha);
    assert_eq!(decision.candidate_commit, current.head_sha);
    assert_eq!(
        decision.requirement_sha256,
        context.expected.requirement_sha256
    );
    assert_eq!(decision.policy_sha256, context.expected.policy_sha256);
    assert_eq!(decision.verifier_id, "independent-verifier");
    assert_eq!(decision.approved_review_ids, vec![17]);
    assert_eq!(decision.reviewer_ids, vec![42]);
    assert_eq!(decision.review_snapshot_sha256.len(), 64);
    assert_eq!(decision.review_base_bindings_sha256.len(), 64);
}

#[test]
fn approval_without_a_trusted_base_binding_is_not_ready() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();
    let review_snapshot = reviews(23, &current.head_sha);
    let unbound = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: review_scope(23),
        bindings: Vec::new(),
    };

    assert_eq!(
        evaluate_with_review_base_bindings(
            &current,
            &review_snapshot,
            &unbound,
            &context,
            &proof,
            &policy_bytes,
        ),
        Err(MergeReadinessError::Approval(
            MergeApprovalError::MissingReviewBaseBinding
        ))
    );
}

#[test]
fn approval_bound_to_a_different_base_ref_or_commit_is_not_ready() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();
    let review_snapshot = reviews(23, &current.head_sha);

    for (base_branch, base_commit) in [
        ("main", "3".repeat(40)),
        ("release", current.base_sha.clone()),
    ] {
        let bindings = PullRequestReviewBaseBindingsSnapshotV1 {
            scope: review_scope(23),
            bindings: vec![PullRequestReviewBaseBindingV1 {
                review_id: 17,
                base_branch: base_branch.into(),
                base_commit,
            }],
        };

        assert_eq!(
            evaluate_with_review_base_bindings(
                &current,
                &review_snapshot,
                &bindings,
                &context,
                &proof,
                &policy_bytes,
            ),
            Err(MergeReadinessError::Approval(
                MergeApprovalError::StaleReviewBaseBinding
            ))
        );
    }
}

#[test]
fn base_binding_from_another_pull_request_is_not_ready() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();
    let review_snapshot = reviews(23, &current.head_sha);
    let bindings = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: review_scope(24),
        bindings: vec![PullRequestReviewBaseBindingV1 {
            review_id: 17,
            base_branch: current.base_branch.clone(),
            base_commit: current.base_sha.clone(),
        }],
    };

    assert_eq!(
        evaluate_with_review_base_bindings(
            &current,
            &review_snapshot,
            &bindings,
            &context,
            &proof,
            &policy_bytes,
        ),
        Err(MergeReadinessError::Approval(
            MergeApprovalError::WrongPullRequest
        ))
    );
}

#[test]
fn duplicate_base_bindings_are_not_ready() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();
    let review_snapshot = reviews(23, &current.head_sha);
    let binding = PullRequestReviewBaseBindingV1 {
        review_id: 17,
        base_branch: current.base_branch.clone(),
        base_commit: current.base_sha.clone(),
    };
    let bindings = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: review_scope(23),
        bindings: vec![binding.clone(), binding],
    };

    assert_eq!(
        evaluate_with_review_base_bindings(
            &current,
            &review_snapshot,
            &bindings,
            &context,
            &proof,
            &policy_bytes,
        ),
        Err(MergeReadinessError::Approval(
            MergeApprovalError::InvalidReviewBaseBindings
        ))
    );
}

#[test]
fn base_binding_fingerprint_is_order_independent_and_changes_with_the_base() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let initial_proof = proof(&context.expected);
    let current = pull_request();
    let mut review_snapshot = reviews(23, &current.head_sha);
    review_snapshot.total_count = 2;
    let mut second_review = review_snapshot.reviews[0].clone();
    second_review.id = 18;
    second_review.user = Some(ReviewUserV1 { id: 43 });
    review_snapshot.reviews.push(second_review);

    let binding_for = |review_id, base_commit: &str| PullRequestReviewBaseBindingV1 {
        review_id,
        base_branch: current.base_branch.clone(),
        base_commit: base_commit.into(),
    };
    let bindings = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: review_scope(23),
        bindings: vec![
            binding_for(17, &current.base_sha),
            binding_for(18, &current.base_sha),
        ],
    };
    let reversed = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: bindings.scope.clone(),
        bindings: bindings.bindings.iter().cloned().rev().collect(),
    };

    let ordered_decision = evaluate_with_review_base_bindings(
        &current,
        &review_snapshot,
        &bindings,
        &context,
        &initial_proof,
        &policy_bytes,
    )
    .expect("both accepted reviews have a current base binding");
    let reversed_decision = evaluate_with_review_base_bindings(
        &current,
        &review_snapshot,
        &reversed,
        &context,
        &initial_proof,
        &policy_bytes,
    )
    .expect("binding order cannot change the readiness result");

    assert_eq!(
        ordered_decision.review_base_bindings_sha256,
        reversed_decision.review_base_bindings_sha256
    );
    assert_eq!(
        ordered_decision.review_base_bindings_sha256,
        "405838373b3ee0f03e008bc25b03791f0fc0a54d69880fbe5ba49b12abb0aad7"
    );

    let mut changed_current = current.clone();
    changed_current.base_sha = "3".repeat(40);
    let mut changed_context = context.clone();
    changed_context.expected.base_commit = changed_current.base_sha.clone();
    let changed_proof = proof(&changed_context.expected);
    let changed_bindings = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: review_scope(23),
        bindings: vec![
            binding_for(17, &changed_current.base_sha),
            binding_for(18, &changed_current.base_sha),
        ],
    };
    let changed_decision = evaluate_with_review_base_bindings(
        &changed_current,
        &review_snapshot,
        &changed_bindings,
        &changed_context,
        &changed_proof,
        &policy_bytes,
    )
    .expect("a proof and approvals bound to the changed base are current");

    assert_ne!(
        ordered_decision.review_base_bindings_sha256,
        changed_decision.review_base_bindings_sha256
    );
}

#[test]
fn fresh_approval_cannot_make_a_proof_for_an_old_head_ready() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let mut current = pull_request();
    current.head_sha = "3".repeat(40);

    assert_eq!(
        evaluate(
            &current,
            &reviews(23, &"3".repeat(40)),
            &context,
            &proof,
            &policy_bytes,
        ),
        Err(MergeReadinessError::Preflight(
            MergePreflightError::StaleBinding("candidate")
        ))
    );
}

#[test]
fn approval_for_another_pull_request_cannot_make_current_pr_ready() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();

    assert_eq!(
        evaluate(
            &current,
            &reviews(24, &current.head_sha),
            &context,
            &proof,
            &policy_bytes,
        ),
        Err(MergeReadinessError::Approval(
            agent_forge_core::forgejo::MergeApprovalError::WrongPullRequest
        ))
    );
}

#[test]
fn approval_for_an_old_reviewed_head_is_not_counted() {
    let policy_bytes = policy_bytes("independent-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();

    assert_eq!(
        evaluate(
            &current,
            &reviews(23, &"4".repeat(40)),
            &context,
            &proof,
            &policy_bytes,
        ),
        Err(MergeReadinessError::Approval(
            agent_forge_core::forgejo::MergeApprovalError::InsufficientApprovals {
                required: 1,
                found: 0,
            }
        ))
    );
}

#[test]
fn policy_and_passing_proof_must_name_the_same_verifier() {
    let policy_bytes = policy_bytes("another-verifier");
    let context = context(&policy_bytes);
    let proof = proof(&context.expected);
    let current = pull_request();

    assert_eq!(
        evaluate(
            &current,
            &reviews(23, &current.head_sha),
            &context,
            &proof,
            &policy_bytes,
        ),
        Err(MergeReadinessError::Approval(
            agent_forge_core::forgejo::MergeApprovalError::VerifierMismatch
        ))
    );
}
