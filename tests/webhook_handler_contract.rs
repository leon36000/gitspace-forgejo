use agent_forge_core::{
    EventJournal,
    webhook::{
        MAX_WEBHOOK_BODY_BYTES, SUPPORTED_EVENTS, WEBHOOK_EVENT_CONTRACTS_V1,
        WEBHOOK_EVENT_CONTRACTS_V2, WEBHOOK_EVENT_CONTRACTS_V3, WEBHOOK_EVENT_CONTRACTS_V4,
        WEBHOOK_EVENT_CONTRACTS_V5, WEBHOOK_EVENT_CONTRACTS_V6, WEBHOOK_EVENT_CONTRACTS_V7,
        WEBHOOK_EVENT_CONTRACTS_V8, WEBHOOK_EVENT_CONTRACTS_V9,
        http::{handle_webhook_request, parse_http_request},
        read_journaled_webhook_event,
        reconcile::{ResourceId, ResourceKind, event_resource_ref},
    },
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

fn temp_journal() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "forgejo-webhook-handler-{}-{stamp}.jsonl",
        std::process::id()
    ))
}

fn signed_request(id: &str, event: &str, payload: &Value) -> Vec<u8> {
    let secret = b"test-secret";
    let body = serde_json::to_vec(payload).unwrap();
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());
    format!(
        "POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Forgejo-Delivery: {id}\r\nX-Gitea-Delivery: {id}\r\nX-Forgejo-Event: {event}\r\nX-Gitea-Event: {event}\r\nX-Forgejo-Signature: {signature}\r\nX-Gitea-Signature: {signature}\r\n\r\n{}",
        body.len(),
        String::from_utf8(body).unwrap()
    )
    .into_bytes()
}

#[test]
fn capabilities_publish_machine_readable_contracts_for_every_supported_event() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let request =
        parse_http_request(b"GET /forgejo/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();

    let response = handle_webhook_request(&request, b"test-secret", &journal);

    assert_eq!(response.status, 200);
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V1).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v2"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V2).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v2"]["workflow_statuses"]
            .as_array()
            .unwrap()
            .len(),
        14
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v3"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V3).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v3"]["workflow_actions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v4"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V4).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v4"]["workflow_event_actions"]
            .as_array()
            .unwrap()
            .len(),
        14
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v5"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V5).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v5"]["event_actions"],
        json!([
            {"event":"issues","payload_field":"action","allowed_actions":["opened","closed","reopened","edited"]},
            {"event":"issue_comment","payload_field":"action","allowed_actions":["created","edited","deleted"]}
        ])
    );
    assert_eq!(response.body["schema_version"], 9);
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v6"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V6).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v6"]["event_actions"],
        json!([
            {"event":"issues","payload_field":"action","allowed_actions":["opened","closed","reopened","edited","assigned","unassigned","label_updated","label_cleared","milestoned","demilestoned"]},
            {"event":"issue_comment","payload_field":"action","allowed_actions":["created","edited","deleted"]}
        ])
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v7"]["schema_version"], 7,
        "pull_request action discovery is additive and leaves v6 available"
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v7"]["event_actions"],
        json!([
            {"event":"issues","payload_field":"action","allowed_actions":["opened","closed","reopened","edited","assigned","unassigned","label_updated","label_cleared","milestoned","demilestoned"]},
            {"event":"issue_comment","payload_field":"action","allowed_actions":["created","edited","deleted"]},
            {"event":"pull_request","payload_field":"action","allowed_actions":["opened","closed","reopened","edited","assigned","unassigned","label_updated","label_cleared","milestoned","demilestoned","synchronized","review_requested","review_request_removed"]}
        ])
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v7"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V7).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v8"]["schema_version"], 8,
        "v8 introduces the required Forgejo issue_comment discriminator"
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v8"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V8).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v9"]["schema_version"], 9,
        "v9 adds native release support while v8 remains available"
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v9"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V9).unwrap()
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts_v9"]["event_actions"][3],
        json!({"event":"release","payload_field":"action","allowed_actions":["published","updated","deleted"]})
    );
    let issue_comment_v7 = response.body["webhook_ingest"]["event_contracts_v7"]["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|contract| contract["event"] == "issue_comment")
        .unwrap();
    assert!(
        !issue_comment_v7["required_payload_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "is_pull")
    );
    let issue_comment_v8 = response.body["webhook_ingest"]["event_contracts_v8"]["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|contract| contract["event"] == "issue_comment")
        .unwrap();
    assert!(
        issue_comment_v8["required_payload_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "is_pull (boolean)")
    );
    assert_eq!(response.body["schema_version"], 9);
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts"]["webhook_payload_is_authoritative"],
        false
    );
    let advertised = response.body["webhook_ingest"]["supported_events"]
        .as_array()
        .expect("the receiver advertises its supported event names");
    let contracts = response.body["webhook_ingest"]["event_contracts_v9"]["contracts"]
        .as_array()
        .expect("the receiver publishes the latest structured event contracts");
    assert_eq!(
        advertised
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>(),
        Some(SUPPORTED_EVENTS.to_vec())
    );
    assert_eq!(
        contracts
            .iter()
            .map(|contract| contract["event"].as_str())
            .collect::<Option<Vec<_>>>(),
        Some(SUPPORTED_EVENTS.to_vec()),
        "contract order and coverage must match the event allowlist"
    );

    let contract = |event: &str| {
        contracts
            .iter()
            .find(|contract| contract["event"] == event)
            .unwrap_or_else(|| panic!("missing machine-readable contract for {event}"))
    };
    let expected_contracts: [(&str, &str, &[&str], &str); 24] = [
        (
            "issues",
            "issue",
            &["repository.full_name", "issue.number", "action"],
            "GET /api/v1/repos/{owner}/{repository}/issues/{number}",
        ),
        (
            "issue_comment",
            "issue_or_pull_request",
            &[
                "repository.full_name",
                "issue.number",
                "action",
                "comment (object)",
                "is_pull (boolean)",
            ],
            "GET /api/v1/repos/{owner}/{repository}/issues/{number} or /pulls/{number}, selected by signed pull_request, then GET /issues/comments/{comment_id}",
        ),
        (
            "pull_request",
            "pull_request",
            &["repository.full_name", "pull_request.number"],
            "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        ),
        (
            "push",
            "branch",
            &["repository.full_name", "ref", "before", "after"],
            "GET /api/v1/repos/{owner}/{repository}/branches/{encoded_branch}",
        ),
        (
            "create",
            "branch",
            &["repository.full_name", "ref_type", "ref", "sha"],
            "GET /api/v1/repos/{owner}/{repository}/branches/{encoded_branch}",
        ),
        (
            "delete",
            "branch",
            &["repository.full_name", "ref_type", "ref"],
            "GET /api/v1/repos/{owner}/{repository}/branches/{encoded_branch}",
        ),
        (
            "pull_request_approved",
            "pull_request",
            &[
                "repository.full_name",
                "pull_request.number",
                "action=reviewed",
                "review.type=pull_request_review_approved",
            ],
            "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        ),
        (
            "pull_request_rejected",
            "pull_request",
            &[
                "repository.full_name",
                "pull_request.number",
                "action=reviewed",
                "review.type=pull_request_review_rejected",
            ],
            "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        ),
        (
            "pull_request_comment",
            "pull_request",
            &[
                "repository.full_name",
                "pull_request.number",
                "action=reviewed",
                "review.type=pull_request_review_comment",
            ],
            "GET /api/v1/repos/{owner}/{repository}/pulls/{number}",
        ),
        (
            "workflow_run_blocked",
            "workflow_run",
            &[
                "action",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
                "run.commit_sha",
                "run.updated",
                "run.status=blocked",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_run_cancelled",
            "workflow_run",
            &[
                "action",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
                "run.commit_sha",
                "run.updated",
                "run.status=cancelled",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_run_failure",
            "workflow_run",
            &[
                "action",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
                "run.commit_sha",
                "run.updated",
                "run.status=failure",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_run_running",
            "workflow_run",
            &[
                "action",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
                "run.commit_sha",
                "run.updated",
                "run.status=running",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_run_skipped",
            "workflow_run",
            &[
                "action",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
                "run.commit_sha",
                "run.updated",
                "run.status=skipped",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_run_success",
            "workflow_run",
            &[
                "action",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
                "run.commit_sha",
                "run.updated",
                "run.status=success",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_run_waiting",
            "workflow_run",
            &[
                "action",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
                "run.commit_sha",
                "run.updated",
                "run.status=waiting",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_job_blocked",
            "workflow_job",
            &[
                "action",
                "job.id",
                "job.run_id",
                "job.status",
                "job.html_url",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_job_cancelled",
            "workflow_job",
            &[
                "action",
                "job.id",
                "job.run_id",
                "job.status",
                "job.html_url",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_job_failure",
            "workflow_job",
            &[
                "action",
                "job.id",
                "job.run_id",
                "job.status",
                "job.html_url",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_job_running",
            "workflow_job",
            &[
                "action",
                "job.id",
                "job.run_id",
                "job.status",
                "job.html_url",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_job_skipped",
            "workflow_job",
            &[
                "action",
                "job.id",
                "job.run_id",
                "job.status",
                "job.html_url",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_job_success",
            "workflow_job",
            &[
                "action",
                "job.id",
                "job.run_id",
                "job.status",
                "job.html_url",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "workflow_job_waiting",
            "workflow_job",
            &[
                "action",
                "job.id",
                "job.run_id",
                "job.status",
                "job.html_url",
                "run.id",
                "run.index_in_repo",
                "run.repository.full_name",
            ],
            "GET /api/v1/repos/{owner}/{repository}/actions/jobs/{job_id}, then GET /api/v1/repos/{owner}/{repository}/actions/runs/{run_id}",
        ),
        (
            "release",
            "release",
            &["repository.full_name", "release.id", "action"],
            "GET /api/v1/repos/{owner}/{repository}/releases/{release_id}",
        ),
    ];
    for (event, resource_kind, required_fields, canonical_read) in expected_contracts {
        let value = contract(event);
        assert_eq!(value["resource_kind"], resource_kind, "event {event}");
        assert_eq!(
            value["required_payload_fields"],
            json!(required_fields),
            "event {event}"
        );
        assert_eq!(value["canonical_read"], canonical_read, "event {event}");
        assert_eq!(
            value["not_found_behavior"], "inconclusive_resource_not_found",
            "event {event}"
        );
        assert!(
            value["payload_constraints"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "event {event} must document its payload constraints"
        );
    }
    assert_eq!(
        contract("push")["payload_constraints"],
        json!([
            "ref uses refs/heads/<branch>",
            "before and after are 40- or 64-character hexadecimal object IDs",
            "the current Forgejo branch read, not payload.after, is canonical"
        ])
    );
    assert!(contract("workflow_job_failure")["payload_constraints"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item
            == "job.html_url supplies only the job index binding; the current job and run API URLs are canonical, and the retry attempt may advance")));
    assert_eq!(
        contract("delete")["payload_constraints"],
        json!([
            "ref_type is branch",
            "ref is a short branch name",
            "sha is create-only and forbidden for delete; this cross-checks payload shape against the unsigned event header",
            "a branch lookup HTTP 404 remains inconclusive because private visibility can be masked"
        ])
    );
    assert_eq!(
        contract("pull_request")["payload_constraints"],
        json!([
            "top-level issue and comment fields are rejected; review may be absent or null, but a non-null review field is rejected",
            "the current Forgejo pull request read, not the webhook payload, is canonical"
        ])
    );
    assert_eq!(
        contract("pull_request_approved")["required_payload_fields"],
        json!([
            "repository.full_name",
            "pull_request.number",
            "action=reviewed",
            "review.type=pull_request_review_approved"
        ])
    );
    assert_eq!(
        contract("release")["payload_constraints"],
        json!([
            "action is one of published, updated, or deleted",
            "release.id is a positive repository-scoped identifier",
            "the current Forgejo release API read is canonical; webhook release notes and assets are not projected"
        ])
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts"]["schema_version"],
        1
    );
    assert_eq!(
        response.body["webhook_ingest"]["event_contracts"]["webhook_payload_is_authoritative"],
        false
    );
    assert_eq!(journal.replay().unwrap().len(), 0);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn valid_delivery_is_accepted_and_identical_retry_is_idempotent() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request(
        "delivery-1",
        "issues",
        &json!({
            "action": "opened",
            "issue": {"number": 1},
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
    );
    let parsed = parse_http_request(&input).unwrap();

    let first = handle_webhook_request(&parsed, b"test-secret", &journal);
    let retry = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(first.status, 202);
    assert_eq!(first.body["duplicate"], false);
    assert_eq!(first.body["reconcile_required"], true);
    assert_eq!(retry.status, 202);
    assert_eq!(retry.body["duplicate"], true);
    assert_eq!(journal.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn issues_ingestion_requires_a_supported_forgejo_action_without_journaling_invalid_payloads() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "opened",
        "issue": {"number": 41},
        "repository": {"full_name": "forge-admin/agent-forge-core"}
    });

    let mut missing_action = payload.clone();
    missing_action.as_object_mut().unwrap().remove("action");
    let missing = parse_http_request(&signed_request(
        "issues-missing-action",
        "issues",
        &missing_action,
    ))
    .unwrap();
    let response = handle_webhook_request(&missing, b"test-secret", &journal);
    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert_eq!(
        response.body["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(journal.replay().unwrap().is_empty());

    let mut non_string_action = payload.clone();
    non_string_action["action"] = json!(7);
    let non_string = parse_http_request(&signed_request(
        "issues-non-string-action",
        "issues",
        &non_string_action,
    ))
    .unwrap();
    let response = handle_webhook_request(&non_string, b"test-secret", &journal);
    assert_eq!(response.status, 400);
    assert_eq!(
        response.body["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(journal.replay().unwrap().is_empty());

    let mut action_from_another_event = payload.clone();
    action_from_another_event["action"] = json!("synchronized");
    let unsupported = parse_http_request(&signed_request(
        "issues-wrong-event-action",
        "issues",
        &action_from_another_event,
    ))
    .unwrap();
    let response = handle_webhook_request(&unsupported, b"test-secret", &journal);
    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert_eq!(
        response.body["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(journal.replay().unwrap().is_empty());

    let mut conflicting_number = payload.clone();
    conflicting_number["number"] = json!(42);
    let conflicting = parse_http_request(&signed_request(
        "issues-conflicting-root-number",
        "issues",
        &conflicting_number,
    ))
    .unwrap();
    let response = handle_webhook_request(&conflicting, b"test-secret", &journal);
    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert!(
        response.body["error"]["action"]
            .as_str()
            .unwrap()
            .contains("not journaled")
    );
    assert!(journal.replay().unwrap().is_empty());

    for (index, action) in [
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
    ]
    .into_iter()
    .enumerate()
    {
        let mut accepted_payload = payload.clone();
        accepted_payload["action"] = json!(action);
        let request = parse_http_request(&signed_request(
            &format!("issues-supported-action-{index}"),
            "issues",
            &accepted_payload,
        ))
        .unwrap();
        let response = handle_webhook_request(&request, b"test-secret", &journal);
        assert_eq!(response.status, 202, "Forgejo issues action {action}");
    }
    assert_eq!(journal.replay().unwrap().len(), 10);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn pull_request_ingestion_requires_supported_forgejo_v17_actions_without_journaling_invalid_payloads()
 {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "opened",
        "pull_request": {"number": 77},
        "repository": {"full_name": "forge-admin/agent-forge-core"}
    });

    let mut missing_action = payload.clone();
    missing_action.as_object_mut().unwrap().remove("action");
    let missing = parse_http_request(&signed_request(
        "pull-request-missing-action",
        "pull_request",
        &missing_action,
    ))
    .unwrap();
    let response = handle_webhook_request(&missing, b"test-secret", &journal);
    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert_eq!(
        response.body["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(journal.replay().unwrap().is_empty());

    let mut non_string_action = payload.clone();
    non_string_action["action"] = json!(7);
    let non_string = parse_http_request(&signed_request(
        "pull-request-non-string-action",
        "pull_request",
        &non_string_action,
    ))
    .unwrap();
    let response = handle_webhook_request(&non_string, b"test-secret", &journal);
    assert_eq!(response.status, 400);
    assert_eq!(
        response.body["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(journal.replay().unwrap().is_empty());

    for (index, action) in ["reviewed", "merged", "synchronize"]
        .into_iter()
        .enumerate()
    {
        let mut rejected_payload = payload.clone();
        rejected_payload["action"] = json!(action);
        let request = parse_http_request(&signed_request(
            &format!("pull-request-wrong-event-action-{index}"),
            "pull_request",
            &rejected_payload,
        ))
        .unwrap();
        let response = handle_webhook_request(&request, b"test-secret", &journal);
        assert_eq!(response.status, 400, "pull_request action {action}");
        assert_eq!(
            response.body["error"]["details"]["field_path"],
            "payload.action"
        );
        assert!(journal.replay().unwrap().is_empty());
    }

    for (index, action) in [
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
    ]
    .into_iter()
    .enumerate()
    {
        let mut accepted_payload = payload.clone();
        accepted_payload["action"] = json!(action);
        let request = parse_http_request(&signed_request(
            &format!("pull-request-supported-action-{index}"),
            "pull_request",
            &accepted_payload,
        ))
        .unwrap();
        let response = handle_webhook_request(&request, b"test-secret", &journal);
        assert_eq!(
            response.status, 202,
            "Forgejo v17 pull_request action {action}"
        );
    }
    assert_eq!(journal.replay().unwrap().len(), 13);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn concurrent_changed_payloads_with_one_delivery_id_accept_once_and_conflict_once() {
    let path = temp_journal();
    let journal = Arc::new(EventJournal::open(&path));
    for attempt in 0..16 {
        let delivery_id = format!("delivery-conflicting-race-{attempt}");
        let barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();

        for action in ["opened", "closed"] {
            let input = signed_request(
                &delivery_id,
                "issues",
                &json!({
                    "action": action,
                    "issue": {"number": 9},
                    "repository": {"full_name": "forge-admin/agent-forge-core"}
                }),
            );
            let parsed = parse_http_request(&input).unwrap();
            let journal = Arc::clone(&journal);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                handle_webhook_request(&parsed, b"test-secret", &journal)
            }));
        }

        let responses = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            responses
                .iter()
                .filter(|response| response.status == 202)
                .count(),
            1,
            "exactly one signed body may claim the delivery ID"
        );
        assert_eq!(
            responses
                .iter()
                .filter(|response| response.status == 409)
                .count(),
            1,
            "the competing body must be rejected as an ID conflict"
        );
        let accepted = responses
            .iter()
            .find(|response| response.status == 202)
            .unwrap();
        assert_eq!(accepted.body["duplicate"], false);
        let conflict = responses
            .iter()
            .find(|response| response.status == 409)
            .unwrap();
        assert_eq!(conflict.body["error"]["code"], "delivery_conflict");

        let events = journal.replay().unwrap();
        assert_eq!(
            events.len(),
            attempt + 1,
            "only the current winner is appended"
        );
        assert_eq!(
            events[attempt].idempotency_key,
            format!("forgejo-delivery-v1:{delivery_id}")
        );
        assert!(matches!(
            events[attempt].payload["payload"]["action"].as_str(),
            Some("opened" | "closed")
        ));
    }

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn invalid_signature_is_unauthorized_and_not_journaled() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request("delivery-2", "issues", &json!({"action":"opened"}));
    let mut source = String::from_utf8(input).unwrap();
    let forged = "00".repeat(32);
    for name in ["X-Forgejo-Signature", "X-Gitea-Signature"] {
        let prefix = format!("{name}: ");
        let start = source.find(&prefix).unwrap() + prefix.len();
        let end = source[start..].find("\r\n").unwrap() + start;
        source.replace_range(start..end, &forged);
    }
    let parsed = parse_http_request(source.as_bytes()).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 401);
    assert_eq!(response.body["error"]["code"], "invalid_signature");
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn unsupported_event_is_rejected_without_journal_side_effect() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request("delivery-3", "repository", &json!({}));
    let parsed = parse_http_request(&input).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 422);
    assert_eq!(response.body["error"]["code"], "unsupported_event");
    let action = response.body["error"]["action"]
        .as_str()
        .expect("unsupported event errors include corrective guidance");
    let supported_event_list = SUPPORTED_EVENTS.join(", ");
    assert!(
        action.contains(&supported_event_list),
        "unsupported event guidance must advertise the complete allowlist: {action}"
    );
    assert!(
        action.contains("webhook_ingest.event_contracts")
            && action.contains("GET /forgejo/capabilities"),
        "unsupported event guidance must direct operators to its machine-readable contract: {action}"
    );
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_branch_delete_delivery_is_bound_to_its_branch() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request(
        "branch-delete-delivery-1",
        "delete",
        &json!({
            "ref": "feature/agent-proof",
            "ref_type": "branch",
            "pusher_type": "user",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "sender": {"login": "fixture-owner"}
        }),
    );
    let parsed = parse_http_request(&input).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);
    let events = journal.replay().unwrap();

    assert_eq!(response.status, 202);
    assert_eq!(response.body["reconcile_required"], true);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.delete");
    assert_eq!(
        event_resource_ref(&events[0]).unwrap().id,
        agent_forge_core::webhook::reconcile::ResourceId::Branch("feature/agent-proof".into())
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_workflow_run_delivery_is_admitted_and_bound_to_the_run() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request(
        "workflow-run-delivery-1",
        "workflow_run_success",
        &json!({
            "action": "run_completed",
            "run": {
                "id": 7001,
                "index_in_repo": 41,
                "status": "success",
                "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "updated": "2026-10-26T11:00:00Z",
                "repository": {
                    "full_name": "forge-admin/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "forge-admin"}
                }
            }
        }),
    );
    let parsed = parse_http_request(&input).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);
    let events = journal.replay().unwrap();

    assert_eq!(response.status, 202);
    assert_eq!(response.body["reconcile_required"], true);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.workflow_run_success");
    let resource = event_resource_ref(&events[0]).unwrap();
    assert_eq!(resource.kind, ResourceKind::WorkflowRun);
    assert_eq!(resource.owner, "forge-admin");
    assert_eq!(resource.repository, "agent-forge-core");
    assert!(matches!(
        resource.id,
        agent_forge_core::webhook::reconcile::ResourceId::WorkflowRun {
            run_id: 7001,
            index_in_repo: 41
        }
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn workflow_run_status_mismatch_refusal_points_to_event_contract() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "failure",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": {
                "full_name": "forge-admin/agent-forge-core",
                "name": "agent-forge-core",
                "owner": {"login": "forge-admin"}
            }
        }
    });
    let input = signed_request(
        "workflow-run-status-mismatch",
        "workflow_run_success",
        &payload,
    );
    let parsed = parse_http_request(&input).expect("parse signed workflow-run request");

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert_eq!(response.body["error"]["retryable"], false);
    let action = response.body["error"]["action"]
        .as_str()
        .expect("status mismatch refusal contains recovery guidance");
    assert!(action.contains("GET /forgejo/capabilities"));
    assert!(action.contains("event contract"));
    assert!(action.contains("not journaled"));
    assert!(journal.replay().expect("replay rejected inbox").is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_workflow_job_delivery_is_admitted_as_a_distinct_resource() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request(
        "workflow-job-delivery-1",
        "workflow_job_failure",
        &json!({
            "action": "job_completed",
            "job": {
                "id": 9001,
                "run_id": 7001,
                "status": "failure",
                "name": "integration-tests",
                "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/actions/runs/41/jobs/1/attempt/1"
            },
            "run": {
                "id": 7001,
                "index_in_repo": 41,
                "status": "running",
                "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "updated": "2026-10-26T11:00:00Z",
                "repository": {
                    "full_name": "forge-admin/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "forge-admin"}
                }
            }
        }),
    );
    let parsed = parse_http_request(&input).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);
    let events = journal.replay().unwrap();

    assert_eq!(response.status, 202);
    assert_eq!(response.body["reconcile_required"], true);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.workflow_job_failure");
    let resource = event_resource_ref(&events[0]).unwrap();
    assert_eq!(resource.owner, "forge-admin");
    assert_eq!(resource.repository, "agent-forge-core");
    assert_eq!(resource.kind, ResourceKind::WorkflowJob);
    assert_eq!(
        resource.id,
        ResourceId::WorkflowJob {
            job_id: 9001,
            run_id: 7001,
            index_in_repo: 41,
            job_index: 1
        }
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn workflow_job_header_status_and_parent_run_binding_are_checked_before_append() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let base = json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/actions/runs/41/jobs/1/attempt/1"
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "running",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }
    });
    let mut cases = vec![(
        "job-header-status-substitution",
        "workflow_job_success",
        base.clone(),
    )];
    let mut mismatched_parent = base;
    mismatched_parent["job"]["run_id"] = json!(7002);
    cases.push((
        "job-parent-run-mismatch",
        "workflow_job_failure",
        mismatched_parent,
    ));
    let malformed_job_url = json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/actions/runs/41/jobs/01/attempt/1"
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }
    });
    cases.push((
        "job-url-has-noncanonical-index",
        "workflow_job_failure",
        malformed_job_url,
    ));

    for (delivery_id, event, payload) in cases {
        let input = signed_request(delivery_id, event, &payload);
        let parsed = parse_http_request(&input).unwrap();
        let response = handle_webhook_request(&parsed, b"test-secret", &journal);

        assert_eq!(response.status, 400, "delivery {delivery_id}");
        assert_eq!(response.body["error"]["code"], "event_payload_invalid");
        if delivery_id == "job-header-status-substitution" {
            let action = response.body["error"]["action"]
                .as_str()
                .expect("invalid status refusal contains recovery guidance");
            assert!(
                action.contains("GET /forgejo/capabilities")
                    && action.contains("event contract")
                    && action.contains("not journaled"),
                "workflow status refusal should point to its machine-readable contract: {action}"
            );
        }
    }
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_branch_create_delivery_is_bound_to_its_branch() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request(
        "branch-create-delivery-1",
        "create",
        &json!({
            "ref": "feature/agent-proof",
            "ref_type": "branch",
            "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "sender": {"login": "fixture-owner"}
        }),
    );
    let parsed = parse_http_request(&input).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);
    let events = journal.replay().unwrap();

    assert_eq!(response.status, 202);
    assert_eq!(response.body["reconcile_required"], true);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.create");
    assert_eq!(
        event_resource_ref(&events[0]).unwrap().id,
        agent_forge_core::webhook::reconcile::ResourceId::Branch("feature/agent-proof".into())
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn unsigned_delete_header_cannot_reclassify_a_signed_create_payload() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request(
        "branch-create-relabelled-delete-1",
        "delete",
        &json!({
            "ref": "feature/agent-proof",
            "ref_type": "branch",
            "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "sender": {"login": "fixture-owner"}
        }),
    );
    let parsed = parse_http_request(&input).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert_eq!(
        response.body["error"]["details"]["field_path"],
        "payload.sha"
    );
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_branch_create_refuses_tags_and_malformed_bindings_without_journaling() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let invalid_payloads = [
        json!({
            "ref": "v1.0.0",
            "ref_type": "tag",
            "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": "feature/../agent-proof",
            "ref_type": "branch",
            "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": "feature/agent-proof",
            "ref_type": "branch",
            "sha": "malformed",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": "feature/agent-proof",
            "ref_type": "branch",
            "sha": "0000000000000000000000000000000000000000",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": "feature/agent-proof",
            "ref_type": "branch",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
    ];

    for (index, payload) in invalid_payloads.iter().enumerate() {
        let request = signed_request(&format!("branch-create-invalid-{index}"), "create", payload);
        let parsed = parse_http_request(&request).unwrap();
        let response = handle_webhook_request(&parsed, b"test-secret", &journal);
        if index == 0 {
            assert_eq!(response.status, 422, "unsupported tag create: {response:?}");
            assert_eq!(response.body["error"]["code"], "unsupported_event");
            assert_eq!(response.body["error"]["retryable"], false);
            assert_eq!(
                response.body["error"]["details"]["field_path"],
                "payload.ref_type"
            );
            assert_eq!(
                response.body["error"]["action"],
                "this adapter accepts branch refs only; exclude tag create/delete deliveries at a trusted sender-side filter, or disable those event subscriptions until explicit tag support is added; consult GET /forgejo/capabilities; the rejected body was not journaled"
            );
            continue;
        }
        assert_eq!(response.status, 400, "payload {index}: {response:?}");
        assert_eq!(response.body["error"]["code"], "event_payload_invalid");
        if index >= 2 {
            assert_eq!(
                response.body["error"]["details"]["field_path"], "payload.sha",
                "payload {index}: {response:?}"
            );
        }
    }
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_branch_delete_refuses_tags_and_malformed_bindings_without_journaling() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let invalid_payloads = [
        json!({
            "ref": "refs/tags/v1.0.0",
            "ref_type": "tag",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": "feature/../agent-proof",
            "ref_type": "branch",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": "feature/agent-proof",
            "ref_type": "branch",
            "repository": {"full_name": "forge-admin"}
        }),
        json!({
            "ref": "feature/agent-proof",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref_type": "branch",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": false,
            "ref_type": "branch",
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
        json!({
            "ref": "feature/agent-proof",
            "ref_type": 1,
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
    ];

    for (index, payload) in invalid_payloads.iter().enumerate() {
        let request = signed_request(&format!("branch-delete-invalid-{index}"), "delete", payload);
        let parsed = parse_http_request(&request).unwrap();
        let response = handle_webhook_request(&parsed, b"test-secret", &journal);
        if index == 0 {
            assert_eq!(response.status, 422, "unsupported tag delete: {response:?}");
            assert_eq!(response.body["error"]["code"], "unsupported_event");
            assert_eq!(response.body["error"]["retryable"], false);
            assert_eq!(
                response.body["error"]["details"]["field_path"],
                "payload.ref_type"
            );
            assert_eq!(
                response.body["error"]["action"],
                "this adapter accepts branch refs only; exclude tag create/delete deliveries at a trusted sender-side filter, or disable those event subscriptions until explicit tag support is added; consult GET /forgejo/capabilities; the rejected body was not journaled"
            );
            continue;
        }
        assert_eq!(response.status, 400, "payload {index}: {response:?}");
        assert_eq!(response.body["error"]["code"], "event_payload_invalid");
        if index == 1 || index == 4 || index == 5 {
            assert_eq!(
                response.body["error"]["details"]["field_path"], "payload.ref",
                "payload {index}: malformed branch ref should identify its field"
            );
        }
        if index == 3 || index == 6 {
            assert_eq!(
                response.body["error"]["details"]["field_path"], "payload.ref_type",
                "payload {index}: missing ref_type should identify its field"
            );
        }
    }
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_approved_review_event_is_accepted_and_bound_to_its_pull_request() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "reviewed",
        "number": 14,
        "pull_request": {
            "number": 14,
            "state": "open",
            "merged": false,
            "head": {"sha": "e7a0672a04f51c3f430a57780a2de5fb2fdf0958"},
            "base": {"sha": "f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0"}
        },
        "requested_reviewer": {"id": 42, "login": "verifier-bot"},
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "sender": {"id": 42, "login": "verifier-bot"},
        "review": {"type": "pull_request_review_approved", "content": "approved"}
    });
    let input = signed_request("review-delivery-14", "pull_request_approved", &payload);
    let parsed = parse_http_request(&input).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 202);
    assert_eq!(response.body["accepted"], true);
    assert_eq!(response.body["reconcile_required"], true);
    let event = read_journaled_webhook_event(&journal, "review-delivery-14").unwrap();
    assert_eq!(
        event_resource_ref(event.event()).unwrap().kind,
        ResourceKind::PullRequest
    );
    assert_eq!(journal.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn review_notifications_require_the_reviewed_action_before_journaling() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let cases = [
        ("pull_request_approved", "pull_request_review_approved"),
        ("pull_request_rejected", "pull_request_review_rejected"),
        ("pull_request_comment", "pull_request_review_comment"),
    ];

    for (event_index, (event, review_type)) in cases.into_iter().enumerate() {
        for (action_index, action) in [None, Some(json!("edited")), Some(json!(42))]
            .into_iter()
            .enumerate()
        {
            let mut payload = json!({
                "action": "reviewed",
                "number": 14,
                "pull_request": {
                    "number": 14,
                    "state": "open",
                    "merged": false,
                    "head": {"sha": "e7a0672a04f51c3f430a57780a2de5fb2fdf0958"},
                    "base": {"sha": "f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0"}
                },
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "review": {"type": review_type}
            });
            match action {
                None => {
                    payload
                        .as_object_mut()
                        .expect("review payload is an object")
                        .remove("action");
                }
                Some(action) => payload["action"] = action,
            }
            let delivery_id = format!("review-action-{event_index}-{action_index}");
            let request = signed_request(&delivery_id, event, &payload);
            let parsed = parse_http_request(&request).unwrap();

            let response = handle_webhook_request(&parsed, b"test-secret", &journal);

            assert_eq!(response.status, 400, "{event} action case {action_index}");
            assert_eq!(
                response.body["error"]["code"], "event_payload_invalid",
                "{event} action case {action_index}: {response:?}"
            );
            assert_eq!(
                response.body["error"]["details"]["field_path"], "payload.action",
                "{event} action case {action_index}: {response:?}"
            );
        }
    }

    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_synchronized_pull_request_event_is_accepted_under_pull_request_header() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "synchronized",
        "number": 22,
        "pull_request": {
            "number": 22,
            "state": "open",
            "merged": false,
            "head": {"sha": "e7a0672a04f51c3f430a57780a2de5fb2fdf0958"},
            "base": {"sha": "f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0"}
        },
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "sender": {"id": 42, "login": "agent"}
    });
    let request = signed_request("pr-sync-22", "pull_request", &payload);
    let parsed = parse_http_request(&request).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 202);
    assert_eq!(response.body["accepted"], true);
    let event = read_journaled_webhook_event(&journal, "pr-sync-22").unwrap();
    assert_eq!(event.event().kind, "forgejo.webhook.v1.pull_request");
    assert_eq!(
        event_resource_ref(event.event()).unwrap().kind,
        ResourceKind::PullRequest
    );
    assert_eq!(journal.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_pull_request_event_accepts_null_review_pointer() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "closed",
        "number": 22,
        "pull_request": {
            "number": 22,
            "state": "closed",
            "merged": true,
            "head": {"sha": "e7a0672a04f51c3f430a57780a2de5fb2fdf0958"},
            "base": {"sha": "f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0"},
            "merge_commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "requested_reviewer": null,
        "commit_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "review": null
    });
    let request = signed_request("pr-closed-null-review-22", "pull_request", &payload);
    let parsed = parse_http_request(&request).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 202);
    assert_eq!(response.body["accepted"], true);
    let event = read_journaled_webhook_event(&journal, "pr-closed-null-review-22").unwrap();
    assert_eq!(event.event().kind, "forgejo.webhook.v1.pull_request");
    assert_eq!(journal.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn pull_request_event_with_non_null_review_member_is_rejected_before_journaling() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);

    for (index, review) in [
        Value::Bool(false),
        Value::String("pull_request_review_approved".to_owned()),
        json!([]),
        json!({}),
        json!({"type": "future_review_type"}),
        json!({"type": 7}),
    ]
    .into_iter()
    .enumerate()
    {
        let payload = json!({
            "action": "synchronized",
            "number": 22,
            "pull_request": {
                "number": 22,
                "state": "open",
                "merged": false,
                "head": {"sha": "e7a0672a04f51c3f430a57780a2de5fb2fdf0958"},
                "base": {"sha": "f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0"}
            },
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "review": review
        });
        let request = signed_request(
            &format!("pr-review-field-{index}"),
            "pull_request",
            &payload,
        );
        let parsed = parse_http_request(&request).unwrap();

        let response = handle_webhook_request(&parsed, b"test-secret", &journal);

        assert_eq!(response.status, 400, "review {index}: {response:?}");
        assert_eq!(response.body["error"]["code"], "event_payload_invalid");
        assert_eq!(
            response.body["error"]["details"]["field_path"], "payload.review",
            "review {index}: {response:?}"
        );
    }

    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_internal_pull_request_sync_task_name_is_not_a_wire_event_header() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "opened",
        "pull_request": {"number": 22},
        "repository": {"full_name": "forge-admin/agent-forge-core"}
    });
    let request = signed_request("pr-sync-wrong-header-22", "pull_request_sync", &payload);
    let parsed = parse_http_request(&request).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 422);
    assert_eq!(response.body["error"]["code"], "unsupported_event");
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_review_subtypes_are_journaled_as_pull_request_notifications() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    for (index, (event, review_type)) in [
        ("pull_request_approved", "pull_request_review_approved"),
        ("pull_request_rejected", "pull_request_review_rejected"),
        ("pull_request_comment", "pull_request_review_comment"),
    ]
    .into_iter()
    .enumerate()
    {
        let delivery_id = format!("review-subtype-{index}");
        let payload = json!({
            "action": "reviewed",
            "pull_request": {"number": 14},
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "review": {"type": review_type}
        });
        let request = signed_request(&delivery_id, event, &payload);
        let parsed = parse_http_request(&request).unwrap();
        let response = handle_webhook_request(&parsed, b"test-secret", &journal);

        assert_eq!(response.status, 202, "event {event}");
        let journaled = read_journaled_webhook_event(&journal, &delivery_id).unwrap();
        assert_eq!(
            event_resource_ref(journaled.event()).unwrap().kind,
            ResourceKind::PullRequest,
            "event {event}"
        );
    }
    assert_eq!(journal.replay().unwrap().len(), 3);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_issue_comment_wire_event_binds_issue_and_pull_request_comments() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let cases = [
        (
            "issue-comment-20",
            json!({
                "action": "created",
                "issue": {"number": 20},
                "comment": {"id": 120, "body": "untrusted issue comment"},
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "is_pull": false
            }),
            ResourceKind::IssueComment,
        ),
        (
            "pr-comment-21",
            json!({
                "action": "created",
                "issue": {"number": 21},
                "pull_request": {"number": 21},
                "comment": {"id": 121, "body": "untrusted PR conversation comment"},
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "is_pull": true
            }),
            ResourceKind::PullRequestComment,
        ),
    ];

    for (delivery_id, payload, expected_kind) in cases {
        let request = signed_request(delivery_id, "issue_comment", &payload);
        let parsed = parse_http_request(&request).unwrap();
        let response = handle_webhook_request(&parsed, b"test-secret", &journal);
        assert_eq!(response.status, 202, "delivery {delivery_id}: {response:?}");
        let event = read_journaled_webhook_event(&journal, delivery_id).unwrap();
        let resource = event_resource_ref(event.event()).unwrap();
        assert_eq!(resource.kind, expected_kind, "delivery {delivery_id}");
        assert_eq!(
            resource.number(),
            Some(payload["issue"]["number"].as_u64().unwrap())
        );
    }

    assert_eq!(journal.replay().unwrap().len(), 2);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn issue_comment_refuses_inconsistent_issue_pull_request_bindings() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let invalid_payloads = [
        json!({
            "action": "created",
            "issue": {"number": 20},
            "pull_request": {"number": 21},
            "comment": {"id": 122},
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "is_pull": true
        }),
        json!({
            "action": "created",
            "issue": {"number": 20},
            "pull_request": null,
            "comment": {"id": 123},
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "is_pull": false
        }),
        json!({
            "action": "created",
            "issue": {"number": 20},
            "comment": {"id": 125},
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "is_pull": true
        }),
        json!({
            "action": "created",
            "issue": {"number": 20},
            "comment": {"id": 124},
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "is_pull": "true"
        }),
        json!({
            "action": "created",
            "issue": {"number": 20},
            "comment": {"id": 126},
            "repository": {"full_name": "forge-admin/agent-forge-core"}
        }),
    ];

    for (index, payload) in invalid_payloads.iter().enumerate() {
        let delivery_id = format!("invalid-issue-comment-{index}");
        let request = signed_request(&delivery_id, "issue_comment", payload);
        let parsed = parse_http_request(&request).unwrap();
        let response = handle_webhook_request(&parsed, b"test-secret", &journal);
        assert_eq!(response.status, 400, "delivery {delivery_id}: {response:?}");
        assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    }
    let workflow_payload = json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": {
                "full_name": "forge-admin/agent-forge-core",
                "name": "agent-forge-core",
                "owner": {"login": "forge-admin"}
            }
        }
    });
    let request = signed_request(
        "header-substitution-workflow-run",
        "workflow_run_failure",
        &workflow_payload,
    );
    let parsed = parse_http_request(&request).unwrap();
    let response = handle_webhook_request(&parsed, b"test-secret", &journal);
    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_v17_pull_request_comment_event_requires_a_review_payload() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "created",
        "issue": {"number": 20},
        "pull_request": {"number": 20},
        "comment": {"id": 124, "body": "conversation comment"},
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "is_pull": true
    });
    let request = signed_request(
        "wrong-pr-comment-wire-shape",
        "pull_request_comment",
        &payload,
    );
    let parsed = parse_http_request(&request).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn unsigned_event_header_substitution_is_refused_when_payload_shape_disagrees() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "created",
        "issue": {"number": 20},
        "pull_request": {"number": 20},
        "comment": {"id": 125, "body": "signed conversation comment"},
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "is_pull": true
    });

    for (index, event_name) in ["issues", "pull_request"].into_iter().enumerate() {
        // The signature covers the body, not the event header. This request is
        // still body-authentic but must not be reclassified as another event.
        let delivery_id = format!("header-substitution-{index}");
        let request = signed_request(&delivery_id, event_name, &payload);
        let parsed = parse_http_request(&request).unwrap();
        let response = handle_webhook_request(&parsed, b"test-secret", &journal);
        assert_eq!(response.status, 400, "event {event_name}: {response:?}");
        assert_eq!(response.body["error"]["code"], "event_payload_invalid");
    }
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn forgejo_review_type_value_is_not_mistaken_for_an_http_event_header() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let payload = json!({
        "action": "reviewed",
        "pull_request": {"number": 20},
        "review": {"type": "pull_request_review_approved"},
        "repository": {"full_name": "forge-admin/agent-forge-core"}
    });
    let request = signed_request(
        "review-type-is-not-header",
        "pull_request_review_approved",
        &payload,
    );
    let parsed = parse_http_request(&request).unwrap();

    let response = handle_webhook_request(&parsed, b"test-secret", &journal);

    assert_eq!(response.status, 422);
    assert_eq!(response.body["error"]["code"], "unsupported_event");
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn wrong_path_method_and_content_type_are_rejected() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let input = signed_request("delivery-4", "issues", &json!({"action":"opened"}));
    let source = String::from_utf8(input).unwrap();
    let wrong_path = parse_http_request(
        source
            .replace("POST /forgejo/webhook", "POST /admin")
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        handle_webhook_request(&wrong_path, b"test-secret", &journal).status,
        404
    );
    let wrong_method = parse_http_request(
        source
            .replace("POST /forgejo/webhook", "GET /forgejo/webhook")
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        handle_webhook_request(&wrong_method, b"test-secret", &journal).status,
        405
    );
    let wrong_type = parse_http_request(
        source
            .replace("Content-Type: application/json\r\n", "")
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        handle_webhook_request(&wrong_type, b"test-secret", &journal).status,
        415
    );
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn body_over_limit_is_rejected_by_parser_before_allocation() {
    let input = format!(
        "POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
        MAX_WEBHOOK_BODY_BYTES + 1
    );
    assert!(matches!(
        parse_http_request(input.as_bytes()),
        Err(agent_forge_core::webhook::http::HttpRequestError::BodyTooLarge)
    ));
}
