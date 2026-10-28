use agent_forge_core::Policy;
use agent_forge_core::forgejo::{
    MAX_MERGE_APPROVAL_POLICY_BYTES, MAX_MERGE_APPROVAL_REVIEW_ENTRIES,
    MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES, MAX_MERGE_APPROVAL_REVIEW_PULL_REQUEST_URL_BYTES,
    MAX_MERGE_APPROVAL_REVIEW_TOTAL_BODY_BYTES, MergeApprovalError, PullRequestReviewPageBodyV1,
    PullRequestReviewPageV1, PullRequestReviewScopeV1, PullRequestReviewV1,
    PullRequestReviewsSnapshotV1, ReviewUserV1, collect_pull_request_review_json_pages,
    collect_pull_request_review_pages, validate_merge_approvals,
};
use serde_json::json;
use std::cell::Cell;
use std::io::{self, Read};
use std::rc::Rc;

struct PaddedJsonArrayReader {
    remaining: usize,
    sent: usize,
    total_read: Rc<Cell<usize>>,
}

impl PaddedJsonArrayReader {
    fn new(length: usize, total_read: Rc<Cell<usize>>) -> Self {
        Self {
            remaining: length,
            sent: 0,
            total_read,
        }
    }
}

impl Read for PaddedJsonArrayReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.remaining);
        if count == 0 {
            return Ok(0);
        }
        for (index, byte) in output[..count].iter_mut().enumerate() {
            *byte = match self.sent + index {
                0 => b'[',
                1 => b']',
                _ => b' ',
            };
        }
        self.sent += count;
        self.remaining -= count;
        self.total_read.set(self.total_read.get() + count);
        Ok(count)
    }
}

struct FailedReader;

impl Read for FailedReader {
    fn read(&mut self, _output: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("fixture response read failure"))
    }
}

fn candidate() -> String {
    "1".repeat(40)
}

fn review(
    id: u64,
    reviewer_id: u64,
    state: &str,
    commit_id: &str,
    stale: bool,
    official: bool,
    dismissed: bool,
) -> PullRequestReviewV1 {
    PullRequestReviewV1 {
        id,
        user: Some(ReviewUserV1 { id: reviewer_id }),
        state: state.to_owned(),
        commit_id: commit_id.to_owned(),
        pull_request_url: scope(23).pull_request_url(),
        stale,
        official,
        dismissed,
    }
}

fn snapshot(reviews: Vec<PullRequestReviewV1>) -> PullRequestReviewsSnapshotV1 {
    PullRequestReviewsSnapshotV1 {
        scope: scope(23),
        total_count: reviews.len() as u64,
        reviews,
    }
}

fn scope(number: u64) -> PullRequestReviewScopeV1 {
    PullRequestReviewScopeV1::new("https://forgejo.example", "acme", "project", number)
        .expect("valid fixture pull request scope")
}

fn policy_bytes(required_approvals: u32, authorized_reviewer_ids: &[u64]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "revision": 4,
        "verifier_id": "independent-verifier",
        "environment": "isolated fixture",
        "acceptance_command": ["python3", "acceptance.py"],
        "commands": [["cargo", "test"]],
        "protected_criteria": ["policy/**", "requirements/**"],
        "timeout_seconds": 90,
        "merge_approval_policy": {
            "schema_version": 1,
            "required_approvals": required_approvals,
            "authorized_reviewer_ids": authorized_reviewer_ids
        }
    }))
    .expect("serialize fixture policy")
}

fn validate(
    snapshot: &PullRequestReviewsSnapshotV1,
    candidate_commit: &str,
    execution_reviewer_id: u64,
    policy_bytes: &[u8],
) -> Result<agent_forge_core::forgejo::MergeApprovalDecisionV1, MergeApprovalError> {
    let policy_sha256 = agent_forge_core::sha256(policy_bytes);
    validate_merge_approvals(
        snapshot,
        &scope(23),
        candidate_commit,
        execution_reviewer_id,
        &policy_sha256,
        policy_bytes,
    )
}

#[test]
fn current_official_approval_is_bound_to_the_exact_candidate_and_decision() {
    let current = candidate();
    let snapshot = snapshot(vec![review(
        7, 42, "APPROVED", &current, false, true, false,
    )]);

    let policy = policy_bytes(1, &[42, 43]);
    let decision = validate(&snapshot, &current, 9, &policy).expect("approval is current");

    assert_eq!(decision.scope, scope(23));
    assert_eq!(decision.candidate_commit, current);
    assert_eq!(decision.approved_review_ids, vec![7]);
    assert_eq!(decision.reviewer_ids, vec![42]);
}

#[test]
fn decision_fingerprint_binds_the_complete_review_snapshot_independent_of_order() {
    let current = candidate();
    let first = review(7, 42, "APPROVED", &current, false, true, false);
    let second = review(8, 43, "COMMENT", &current, false, true, false);
    let policy = policy_bytes(1, &[42]);

    let forward = validate(
        &snapshot(vec![first.clone(), second.clone()]),
        &current,
        9,
        &policy,
    )
    .expect("current approval");
    let reverse = validate(
        &snapshot(vec![second.clone(), first.clone()]),
        &current,
        9,
        &policy,
    )
    .expect("same snapshot in a different API order");
    let dismissed = validate(
        &snapshot(vec![
            first,
            review(8, 43, "COMMENT", &current, false, true, true),
        ]),
        &current,
        9,
        &policy,
    )
    .expect("current approval remains sufficient");

    assert_eq!(forward.review_snapshot_sha256.len(), 64);
    assert_eq!(
        forward.review_snapshot_sha256,
        reverse.review_snapshot_sha256
    );
    assert_ne!(
        forward.review_snapshot_sha256,
        dismissed.review_snapshot_sha256
    );

    let golden = validate(
        &snapshot(vec![review(
            7, 42, "APPROVED", &current, false, true, false,
        )]),
        &current,
        9,
        &policy,
    )
    .expect("golden canonical snapshot");
    assert_eq!(
        golden.review_snapshot_sha256,
        "83a3f6713e1e53d0ee44a5deb3cb7f3a80223436db19880f0d06f0a4d31b5b89"
    );
}

#[test]
fn review_json_page_read_stops_at_one_byte_beyond_page_budget() {
    let total_read = Rc::new(Cell::new(0));
    let response = PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: 1,
        total_count: 0,
        body: PaddedJsonArrayReader::new(
            MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES + 64,
            total_read.clone(),
        ),
    };

    assert_eq!(
        collect_pull_request_review_json_pages(&scope(23), [response]),
        Err(MergeApprovalError::ReviewPageBodyTooLarge)
    );
    assert_eq!(
        total_read.get(),
        MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES + 1
    );
}

#[test]
fn review_json_page_accepts_exact_page_byte_limit() {
    let total_read = Rc::new(Cell::new(0));
    let response = PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: 1,
        total_count: 0,
        body: PaddedJsonArrayReader::new(
            MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES,
            total_read.clone(),
        ),
    };

    let snapshot = collect_pull_request_review_json_pages(&scope(23), [response])
        .expect("a response exactly at the page budget is accepted");
    assert!(snapshot.reviews.is_empty());
    assert_eq!(total_read.get(), MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES);
}

#[test]
fn review_json_pages_share_a_cumulative_byte_budget() {
    let total_read = Rc::new(Cell::new(0));
    let page_size = MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES;
    let page_count = MAX_MERGE_APPROVAL_REVIEW_TOTAL_BODY_BYTES / page_size + 1;
    let pages = (1..=page_count).map(|page| PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: page as u32,
        total_count: MAX_MERGE_APPROVAL_REVIEW_ENTRIES as u64,
        body: PaddedJsonArrayReader::new(page_size, total_read.clone()),
    });

    assert_eq!(
        collect_pull_request_review_json_pages(&scope(23), pages),
        Err(MergeApprovalError::ReviewSnapshotBodyTooLarge)
    );
    assert_eq!(
        total_read.get(),
        MAX_MERGE_APPROVAL_REVIEW_TOTAL_BODY_BYTES + 1
    );
}

#[test]
fn review_json_pages_accept_exact_cumulative_byte_limit() {
    let current = candidate();
    let page_size = MAX_MERGE_APPROVAL_REVIEW_PAGE_BODY_BYTES;
    let page_count = MAX_MERGE_APPROVAL_REVIEW_TOTAL_BODY_BYTES / page_size;
    let pages = (1..=page_count).map(|review_id| {
        let mut body = serde_json::to_vec(&vec![review(
            review_id as u64,
            42,
            "COMMENT",
            &current,
            false,
            true,
            false,
        )])
        .expect("serialize bounded review fixture");
        assert!(body.len() < page_size);
        body.resize(page_size, b' ');
        PullRequestReviewPageBodyV1 {
            scope: scope(23),
            page: review_id as u32,
            total_count: page_count as u64,
            body: io::Cursor::new(body),
        }
    });

    let snapshot = collect_pull_request_review_json_pages(&scope(23), pages)
        .expect("an aggregate response exactly at its byte budget is accepted");
    assert_eq!(snapshot.reviews.len(), page_count);
    assert_eq!(
        page_count * page_size,
        MAX_MERGE_APPROVAL_REVIEW_TOTAL_BODY_BYTES
    );
}

#[test]
fn review_json_pages_decode_minimal_fields_with_bounded_reader() {
    let response = PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: 1,
        total_count: 1,
        body: io::Cursor::new(
            br#"[{"id":1,"user":{"id":42},"state":"APPROVED","commit_id":"1111111111111111111111111111111111111111","pull_request_url":"https://forgejo.example/acme/project/pulls/23","stale":false,"official":true,"dismissed":false,"body":"ignored response field"}]"#,
        ),
    };

    let snapshot = collect_pull_request_review_json_pages(&scope(23), [response])
        .expect("bounded review response decodes into a scoped page");
    assert_eq!(snapshot.reviews.len(), 1);
    assert_eq!(
        snapshot.reviews[0].user.as_ref().map(|user| user.id),
        Some(42)
    );
    assert_eq!(snapshot.reviews[0].state, "APPROVED");
}

#[test]
fn review_from_another_pull_request_is_rejected_by_its_url() {
    let response = PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: 1,
        total_count: 1,
        body: io::Cursor::new(
            br#"[{"id":1,"user":{"id":42},"state":"APPROVED","commit_id":"1111111111111111111111111111111111111111","pull_request_url":"https://forgejo.example/acme/project/pulls/24","stale":false,"official":true,"dismissed":false}]"#,
        ),
    };

    assert_eq!(
        collect_pull_request_review_json_pages(&scope(23), [response]),
        Err(MergeApprovalError::InvalidReviewSet)
    );
}

#[test]
fn direct_snapshot_from_another_pull_request_is_rejected_by_its_url() {
    let current = candidate();
    let policy = policy_bytes(1, &[42]);
    let mut foreign_review = review(7, 42, "APPROVED", &current, false, true, false);
    foreign_review.pull_request_url = scope(24).pull_request_url();

    assert_eq!(
        validate(&snapshot(vec![foreign_review]), &current, 9, &policy),
        Err(MergeApprovalError::InvalidReviewSet)
    );
}

#[test]
fn pull_request_url_size_limit_is_enforced_at_both_entries() {
    fn scope_with_url_length(url_length: usize) -> PullRequestReviewScopeV1 {
        let base = "https://forgejo.example";
        let suffix = "/acme/project/pulls/23";
        let prefix_length = base.len() + 1 + suffix.len();
        let path_length = url_length - prefix_length;
        let origin = format!("{base}/{}", "x".repeat(path_length));
        PullRequestReviewScopeV1::new(&origin, "acme", "project", 23).expect("valid test origin")
    }

    let current = candidate();
    let policy = policy_bytes(1, &[42]);
    for (length, accepted) in [
        (MAX_MERGE_APPROVAL_REVIEW_PULL_REQUEST_URL_BYTES, true),
        (MAX_MERGE_APPROVAL_REVIEW_PULL_REQUEST_URL_BYTES + 1, false),
    ] {
        let expected_scope = scope_with_url_length(length);
        let mut entry = review(7, 42, "APPROVED", &current, false, true, false);
        entry.pull_request_url = expected_scope.pull_request_url();
        assert_eq!(entry.pull_request_url.len(), length);

        let reviews = vec![entry.clone()];
        let collected = collect_pull_request_review_pages(
            &expected_scope,
            vec![PullRequestReviewPageV1 {
                scope: expected_scope.clone(),
                page: 1,
                total_count: 1,
                reviews,
            }],
        );
        let policy_sha256 = agent_forge_core::sha256(&policy);
        let direct = validate_merge_approvals(
            &PullRequestReviewsSnapshotV1 {
                scope: expected_scope.clone(),
                total_count: 1,
                reviews: vec![entry],
            },
            &expected_scope,
            &current,
            9,
            &policy_sha256,
            &policy,
        );

        if accepted {
            assert!(collected.is_ok());
            assert!(direct.is_ok());
        } else {
            assert_eq!(collected, Err(MergeApprovalError::InvalidReviewSet));
            assert_eq!(direct, Err(MergeApprovalError::InvalidReviewSet));
        }
    }
}

#[test]
fn pull_request_url_preserves_the_forgejo_deployment_prefix() {
    let scoped = PullRequestReviewScopeV1::new(
        "https://forgejo.example/forgejo/",
        "team-unit",
        "repo_v2",
        24,
    )
    .expect("valid Forgejo deployment scope");

    assert_eq!(
        scoped.pull_request_url(),
        "https://forgejo.example/forgejo/team-unit/repo_v2/pulls/24"
    );
}

#[test]
fn pull_request_scope_rejects_non_path_safe_repository_segments() {
    assert!(
        PullRequestReviewScopeV1::new("https://forgejo.example", "team+unit", "repo_v2", 24,)
            .is_err()
    );
    assert!(
        PullRequestReviewScopeV1::new("https://forgejo.example", "team-unit", "repo#v2", 24,)
            .is_err()
    );
}

#[test]
fn review_json_page_errors_distinguish_invalid_json_from_reader_failure() {
    let invalid_json = PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: 1,
        total_count: 0,
        body: io::Cursor::new(b"not-json"),
    };
    assert_eq!(
        collect_pull_request_review_json_pages(&scope(23), [invalid_json]),
        Err(MergeApprovalError::InvalidReviewResponse)
    );

    let failed_read = PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: 1,
        total_count: 0,
        body: FailedReader,
    };
    assert_eq!(
        collect_pull_request_review_json_pages(&scope(23), [failed_read]),
        Err(MergeApprovalError::ReviewResponseReadFailed)
    );
}

#[test]
fn review_json_page_rejects_duplicate_unknown_members() {
    let pull_request_url = scope(23).pull_request_url();
    let body = format!(
        r#"[{{"id":7,"pull_request_url":"{pull_request_url}","stale":false,"official":true,"dismissed":false,"extension":1,"extension":2}}]"#
    );
    let response = PullRequestReviewPageBodyV1 {
        scope: scope(23),
        page: 1,
        total_count: 1,
        body: io::Cursor::new(body),
    };

    assert_eq!(
        collect_pull_request_review_json_pages(&scope(23), [response]),
        Err(MergeApprovalError::InvalidReviewResponse)
    );
}

#[test]
fn wrong_review_scope_is_rejected_without_reading_its_body() {
    let total_read = Rc::new(Cell::new(0));
    let response = PullRequestReviewPageBodyV1 {
        scope: scope(24),
        page: 1,
        total_count: 0,
        body: PaddedJsonArrayReader::new(32, total_read.clone()),
    };

    assert_eq!(
        collect_pull_request_review_json_pages(&scope(23), [response]),
        Err(MergeApprovalError::WrongScope)
    );
    assert_eq!(total_read.get(), 0);
}

#[test]
fn review_state_and_commit_id_accept_64_bytes_and_refuse_65_before_fingerprinting() {
    let current = candidate();
    let policy = policy_bytes(1, &[42]);
    for (at_limit_review, oversized_review) in [
        (
            review(8, 43, &"X".repeat(64), &current, false, true, false),
            review(8, 43, &"X".repeat(65), &current, false, true, false),
        ),
        (
            review(8, 43, "COMMENT", &"a".repeat(64), false, true, false),
            review(8, 43, "COMMENT", &"a".repeat(65), false, true, false),
        ),
    ] {
        assert!(
            validate(
                &snapshot(vec![
                    review(7, 42, "APPROVED", &current, false, true, false),
                    at_limit_review.clone(),
                ]),
                &current,
                9,
                &policy
            )
            .is_ok()
        );
        assert!(
            collect_pull_request_review_pages(
                &scope(23),
                vec![PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 1,
                    total_count: 1,
                    reviews: vec![at_limit_review],
                }]
            )
            .is_ok()
        );

        let snapshot = snapshot(vec![
            review(7, 42, "APPROVED", &current, false, true, false),
            oversized_review.clone(),
        ]);

        assert_eq!(
            validate(&snapshot, &current, 9, &policy),
            Err(MergeApprovalError::InvalidReviewSet)
        );
        assert_eq!(
            collect_pull_request_review_pages(
                &scope(23),
                vec![PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 1,
                    total_count: 1,
                    reviews: vec![oversized_review],
                }]
            ),
            Err(MergeApprovalError::InvalidReviewSet)
        );
    }
}

#[test]
fn stale_dismissed_nonofficial_and_wrong_head_approvals_do_not_count() {
    let current = candidate();
    let old = "2".repeat(40);
    let snapshot = snapshot(vec![
        review(1, 42, "APPROVED", &current, true, true, false),
        review(2, 43, "APPROVED", &current, false, true, true),
        review(3, 42, "APPROVED", &current, false, false, false),
        review(4, 43, "APPROVED", &old, false, true, false),
        review(5, 42, "REQUEST_CHANGES", &current, false, true, false),
    ]);

    assert_eq!(
        validate(&snapshot, &current, 9, &policy_bytes(1, &[42, 43])),
        Err(MergeApprovalError::InsufficientApprovals {
            required: 1,
            found: 0,
        })
    );
}

#[test]
fn executor_review_is_not_counted_as_an_independent_approval() {
    let current = candidate();
    let snapshot = snapshot(vec![review(7, 9, "APPROVED", &current, false, true, false)]);

    assert_eq!(
        validate(&snapshot, &current, 9, &policy_bytes(1, &[42, 43])),
        Err(MergeApprovalError::InsufficientApprovals {
            required: 1,
            found: 0,
        })
    );
}

#[test]
fn reviewer_must_be_in_the_host_policy_allowlist() {
    let current = candidate();
    let snapshot = snapshot(vec![review(
        7, 99, "APPROVED", &current, false, true, false,
    )]);

    assert_eq!(
        validate(&snapshot, &current, 9, &policy_bytes(1, &[42, 43])),
        Err(MergeApprovalError::InsufficientApprovals {
            required: 1,
            found: 0,
        })
    );
}

#[test]
fn incomplete_review_pages_are_inconclusive() {
    let current = candidate();
    let mut snapshot = snapshot(vec![review(
        7, 42, "APPROVED", &current, false, true, false,
    )]);
    snapshot.total_count = 2;

    assert_eq!(
        validate(&snapshot, &current, 9, &policy_bytes(1, &[42, 43])),
        Err(MergeApprovalError::IncompleteReviewList)
    );
}

#[test]
fn duplicate_review_ids_are_rejected() {
    let current = candidate();
    let snapshot = snapshot(vec![
        review(7, 42, "APPROVED", &current, false, true, false),
        review(7, 43, "APPROVED", &current, false, true, false),
    ]);

    assert_eq!(
        validate(&snapshot, &current, 9, &policy_bytes(1, &[42, 43])),
        Err(MergeApprovalError::InvalidReviewSet)
    );
}

#[test]
fn reviews_from_a_different_repository_scope_cannot_authorize() {
    let current = candidate();
    let mut snapshot = snapshot(vec![review(
        7, 42, "APPROVED", &current, false, true, false,
    )]);
    snapshot.scope =
        PullRequestReviewScopeV1::new("https://forgejo.example", "other-org", "other-project", 23)
            .unwrap();
    let policy = policy_bytes(1, &[42]);

    assert_eq!(
        validate_merge_approvals(
            &snapshot,
            &scope(23),
            &current,
            9,
            &agent_forge_core::sha256(&policy),
            &policy,
        ),
        Err(MergeApprovalError::WrongScope)
    );
}

#[test]
fn policy_rejects_zero_or_duplicate_reviewer_ids() {
    let current = candidate();
    let snapshot = snapshot(vec![review(
        7, 42, "APPROVED", &current, false, true, false,
    )]);
    let mut invalid = serde_json::from_slice::<serde_json::Value>(&policy_bytes(1, &[42, 43]))
        .expect("valid fixture policy");
    invalid["merge_approval_policy"]["authorized_reviewer_ids"] = json!([42, 42]);

    assert_eq!(
        validate(
            &snapshot,
            &current,
            9,
            &serde_json::to_vec(&invalid).unwrap()
        ),
        Err(MergeApprovalError::InvalidPolicy)
    );

    invalid["merge_approval_policy"]["authorized_reviewer_ids"] = json!([0]);
    assert_eq!(
        validate(
            &snapshot,
            &current,
            9,
            &serde_json::to_vec(&invalid).unwrap()
        ),
        Err(MergeApprovalError::InvalidPolicy)
    );

    invalid = serde_json::from_slice(&policy_bytes(1, &[42, 43])).unwrap();
    invalid["merge_approval_policy"]["schema_version"] = json!(2);
    assert_eq!(
        validate(
            &snapshot,
            &current,
            9,
            &serde_json::to_vec(&invalid).unwrap()
        ),
        Err(MergeApprovalError::InvalidPolicy)
    );

    invalid = serde_json::from_slice(&policy_bytes(1, &[42, 43])).unwrap();
    invalid["merge_approval_policy"]["can_merge"] = json!(true);
    assert_eq!(
        validate(
            &snapshot,
            &current,
            9,
            &serde_json::to_vec(&invalid).unwrap()
        ),
        Err(MergeApprovalError::InvalidPolicy)
    );
}

#[test]
fn distinct_current_reviewers_count_once_and_return_in_stable_order() {
    let current = candidate();
    let snapshot = snapshot(vec![
        review(8, 43, "APPROVED", &current, false, true, false),
        review(7, 42, "APPROVED", &current, false, true, false),
    ]);
    let policy = policy_bytes(2, &[42, 43]);

    let decision = validate(&snapshot, &current, 9, &policy).expect("two reviewers");
    assert_eq!(decision.approved_review_ids, vec![7, 8]);
    assert_eq!(decision.reviewer_ids, vec![42, 43]);
}

#[test]
fn forgejo_api_review_json_keeps_identity_and_freshness_fields() {
    let raw = json!({
        "id": 7,
        "user": {"id": 42, "login": "reviewer", "html_url": "http://fixture/user/reviewer"},
        "state": "APPROVED",
        "commit_id": candidate(),
        "pull_request_url": "https://forgejo.example/acme/project/pulls/23",
        "stale": false,
        "official": true,
        "dismissed": false,
        "body": "approved",
        "comments_count": 0,
        "submitted_at": "2026-10-22T22:00:00Z",
        "unknown_future_field": "ignored by the forward-compatible reader"
    });

    let parsed: PullRequestReviewV1 = serde_json::from_value(raw).expect("Forgejo review shape");
    assert_eq!(parsed.user.unwrap().id, 42);
    assert_eq!(parsed.state, "APPROVED");
    assert!(parsed.official);
    assert!(!parsed.stale);
    assert!(!parsed.dismissed);
}

#[test]
fn missing_scope_or_freshness_fields_are_rejected_during_deserialization() {
    for field in ["pull_request_url", "stale", "dismissed"] {
        let mut raw = json!({
            "id": 7,
            "user": {"id": 42},
            "state": "APPROVED",
            "commit_id": candidate(),
            "pull_request_url": "https://forgejo.example/acme/project/pulls/23",
            "stale": false,
            "official": true,
            "dismissed": false
        });
        raw.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<PullRequestReviewV1>(raw).is_err());
    }
}

#[test]
fn complete_forgejo_review_pages_are_collected_in_order() {
    let current = candidate();
    let collected = collect_pull_request_review_pages(
        &scope(23),
        vec![
            PullRequestReviewPageV1 {
                scope: scope(23),
                page: 1,
                total_count: 2,
                reviews: vec![review(7, 42, "APPROVED", &current, false, true, false)],
            },
            PullRequestReviewPageV1 {
                scope: scope(23),
                page: 2,
                total_count: 2,
                reviews: vec![review(8, 43, "APPROVED", &current, false, true, false)],
            },
        ],
    )
    .expect("complete paginated Forgejo response");

    assert_eq!(collected.total_count, 2);
    assert_eq!(
        collected
            .reviews
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![7, 8]
    );
}

#[test]
fn review_page_gaps_changed_counts_and_duplicate_records_are_refused() {
    let current = candidate();
    let first = review(7, 42, "APPROVED", &current, false, true, false);
    let second = review(8, 43, "APPROVED", &current, false, true, false);

    assert_eq!(
        collect_pull_request_review_pages(
            &scope(23),
            vec![
                PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 1,
                    total_count: 2,
                    reviews: vec![first.clone()],
                },
                PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 3,
                    total_count: 2,
                    reviews: vec![second.clone()],
                },
            ],
        ),
        Err(MergeApprovalError::IncompleteReviewList)
    );
    assert_eq!(
        collect_pull_request_review_pages(
            &scope(23),
            vec![
                PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 1,
                    total_count: 2,
                    reviews: vec![first.clone()],
                },
                PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 2,
                    total_count: 3,
                    reviews: vec![second],
                },
            ],
        ),
        Err(MergeApprovalError::IncompleteReviewList)
    );
    assert_eq!(
        collect_pull_request_review_pages(
            &scope(23),
            vec![
                PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 1,
                    total_count: 2,
                    reviews: vec![first.clone()],
                },
                PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 2,
                    total_count: 2,
                    reviews: vec![first],
                },
            ],
        ),
        Err(MergeApprovalError::InvalidReviewSet)
    );
}

#[test]
fn pages_mixed_across_pull_requests_or_repositories_are_refused() {
    let current = candidate();
    let other_repository =
        PullRequestReviewScopeV1::new("https://forgejo.example", "acme", "other-project", 23)
            .unwrap();

    assert_eq!(
        collect_pull_request_review_pages(
            &scope(23),
            vec![
                PullRequestReviewPageV1 {
                    scope: scope(23),
                    page: 1,
                    total_count: 2,
                    reviews: vec![review(7, 42, "APPROVED", &current, false, true, false)],
                },
                PullRequestReviewPageV1 {
                    scope: other_repository,
                    page: 2,
                    total_count: 2,
                    reviews: vec![review(8, 43, "APPROVED", &current, false, true, false)],
                },
            ],
        ),
        Err(MergeApprovalError::WrongScope)
    );
}

#[test]
fn legacy_policy_without_an_approval_allowlist_deserializes_but_cannot_authorize() {
    let parsed: Policy =
        serde_json::from_str(include_str!("../policy/policy.json")).expect("legacy policy");
    assert_eq!(parsed.merge_approval_policy, None);

    let snapshot = snapshot(vec![review(
        7,
        42,
        "APPROVED",
        &candidate(),
        false,
        true,
        false,
    )]);
    let policy_bytes = include_str!("../policy/policy.json").as_bytes();
    let policy_sha256 = agent_forge_core::sha256(policy_bytes);
    assert_eq!(
        validate_merge_approvals(
            &snapshot,
            &scope(23),
            &candidate(),
            9,
            &policy_sha256,
            policy_bytes
        ),
        Err(MergeApprovalError::NoApprovalPolicy)
    );
}

#[test]
fn versioned_approval_policy_is_parsed_from_the_canonical_policy_field() {
    let policy_bytes = policy_bytes(1, &[42]);
    let parsed: Policy = serde_json::from_slice(&policy_bytes).expect("versioned canonical policy");
    assert_eq!(
        parsed
            .merge_approval_policy
            .as_ref()
            .unwrap()
            .authorized_reviewer_ids,
        vec![42]
    );
    let decision = validate(
        &snapshot(vec![review(
            7,
            42,
            "APPROVED",
            &candidate(),
            false,
            true,
            false,
        )]),
        &candidate(),
        9,
        &policy_bytes,
    )
    .expect("policy-bound reviewer");
    assert_eq!(decision.reviewer_ids, vec![42]);
}

#[test]
fn changed_policy_bytes_invalidate_the_previous_binding() {
    let current = candidate();
    let snapshot = snapshot(vec![review(
        7, 42, "APPROVED", &current, false, true, false,
    )]);
    let signed_policy = policy_bytes(1, &[42]);
    let signed_policy_sha256 = agent_forge_core::sha256(&signed_policy);
    let changed_policy = policy_bytes(2, &[42, 43]);

    assert_eq!(
        validate_merge_approvals(
            &snapshot,
            &scope(23),
            &current,
            9,
            &signed_policy_sha256,
            &changed_policy,
        ),
        Err(MergeApprovalError::StalePolicy)
    );
}

#[test]
fn overlarge_policy_or_review_snapshot_fails_closed() {
    let current = candidate();
    let review_snapshot = snapshot(vec![review(
        7, 42, "APPROVED", &current, false, true, false,
    )]);
    let oversized_policy = vec![b' '; MAX_MERGE_APPROVAL_POLICY_BYTES + 1];
    let oversized_policy_sha256 = agent_forge_core::sha256(&oversized_policy);
    assert_eq!(
        validate_merge_approvals(
            &review_snapshot,
            &scope(23),
            &current,
            9,
            &oversized_policy_sha256,
            &oversized_policy,
        ),
        Err(MergeApprovalError::InvalidPolicy)
    );

    let policy = policy_bytes(1, &[42]);
    let mut overlarge_reviews = review_snapshot;
    overlarge_reviews.total_count = MAX_MERGE_APPROVAL_REVIEW_ENTRIES as u64 + 1;
    assert_eq!(
        validate(&overlarge_reviews, &current, 9, &policy),
        Err(MergeApprovalError::TooManyReviews)
    );
}
