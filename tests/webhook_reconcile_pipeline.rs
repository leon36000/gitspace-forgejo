use agent_forge_core::{
    EventJournal,
    webhook::{
        http::{handle_webhook_request, parse_http_request},
        reconcile::{
            ApiReadError, ForgejoCurrentState, MAX_RECONCILE_BATCH_OUTPUT_BYTES, ReconcileError,
            ReconciledLabel, ReconciledState, ResourceId, ResourceKind, ResourceRef,
            reconcile_inbox_events,
        },
    },
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

const WEBHOOK_SECRET: &[u8] = b"webhook-reconcile-pipeline-fixture-secret";
const FORGEJO_ORIGIN: &str = "http://127.0.0.1:3130";
const ISSUE_URL: &str = "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41";

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "webhook-reconcile-pipeline-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("create private fixture directory");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("restrict fixture directory permissions");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn signed_issue_request(delivery_id: &str, body: &[u8]) -> Vec<u8> {
    signed_webhook_request(delivery_id, "issues", body)
}

fn signed_webhook_request(delivery_id: &str, event: &str, body: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(WEBHOOK_SECRET).expect("valid fixture key");
    mac.update(body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let headers = format!(
        "Content-Length: {}\r\n\
         Content-Type: application/json\r\n\
         X-Forgejo-Delivery: {delivery_id}\r\n\
         X-Gitea-Delivery: {delivery_id}\r\n\
         X-Forgejo-Event: {event}\r\n\
         X-Gitea-Event: {event}\r\n\
         X-Forgejo-Signature: {signature}\r\n\
         X-Gitea-Signature: {signature}\r\n",
        body.len()
    );
    let mut request =
        format!("POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}\r\n").into_bytes();
    request.extend_from_slice(body);
    request
}

struct FixtureForgejoCurrentState {
    response: Value,
    reads: Mutex<Vec<ResourceRef>>,
}

impl FixtureForgejoCurrentState {
    fn issue(number: u64, current_state: &str) -> Self {
        Self {
            response: json!({
                "number": number,
                "state": current_state,
                "updated_at": "2026-10-22T12:30:00Z",
                "html_url": ISSUE_URL
            }),
            reads: Mutex::new(Vec::new()),
        }
    }
}

impl ForgejoCurrentState for FixtureForgejoCurrentState {
    fn read_current(&self, resource: &ResourceRef) -> Result<Value, ApiReadError> {
        self.reads.lock().unwrap().push(resource.clone());
        Ok(self.response.clone())
    }

    fn expected_html_url(&self, resource: &ResourceRef) -> String {
        match (&resource.kind, &resource.id) {
            (ResourceKind::Issue, ResourceId::Number(number)) => format!(
                "{FORGEJO_ORIGIN}/{}/{}/issues/{number}",
                resource.owner, resource.repository
            ),
            (ResourceKind::PullRequest, ResourceId::Number(number)) => format!(
                "{FORGEJO_ORIGIN}/{}/{}/pulls/{number}",
                resource.owner, resource.repository
            ),
            (ResourceKind::Branch, ResourceId::Branch(branch)) => format!(
                "{FORGEJO_ORIGIN}/{}/{}/src/branch/{}",
                resource.owner,
                resource.repository,
                branch.replace('/', "%2F")
            ),
            (ResourceKind::WorkflowRun, ResourceId::WorkflowRun { index_in_repo, .. }) => format!(
                "{FORGEJO_ORIGIN}/{}/{}/actions/runs/{index_in_repo}",
                resource.owner, resource.repository
            ),
            (ResourceKind::WorkflowJob, ResourceId::WorkflowJob { index_in_repo, .. }) => format!(
                "{FORGEJO_ORIGIN}/{}/{}/actions/runs/{index_in_repo}",
                resource.owner, resource.repository
            ),
            _ => String::new(),
        }
    }
}

#[test]
fn signed_webhook_journal_replay_reconciles_stale_payload_to_fixture_state() {
    let scratch = ScratchDir::new();
    let journal = EventJournal::open(scratch.path().join("inbox.jsonl"));
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 41, "state": "closed"}
    }))
    .unwrap();

    let first_request = parse_http_request(&signed_issue_request("delivery-41-a", &body)).unwrap();
    let first = handle_webhook_request(&first_request, WEBHOOK_SECRET, &journal);
    assert_eq!(first.status, 202);
    assert_eq!(first.body["duplicate"], false);
    assert_eq!(first.body["reconcile_required"], true);

    let duplicate = handle_webhook_request(&first_request, WEBHOOK_SECRET, &journal);
    assert_eq!(duplicate.status, 202);
    assert_eq!(duplicate.body["duplicate"], true);

    let replayed_request =
        parse_http_request(&signed_issue_request("delivery-41-b", &body)).unwrap();
    let replayed = handle_webhook_request(&replayed_request, WEBHOOK_SECRET, &journal);
    assert_eq!(replayed.status, 202);
    assert_eq!(replayed.body["duplicate"], false);

    let conflicting_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 41, "state": "closed"}
    }))
    .unwrap();
    let conflict_request =
        parse_http_request(&signed_issue_request("delivery-41-a", &conflicting_body)).unwrap();
    let conflict = handle_webhook_request(&conflict_request, WEBHOOK_SECRET, &journal);
    assert_eq!(conflict.status, 409);
    assert_eq!(conflict.body["error"]["code"], "delivery_conflict");
    assert_eq!(conflict.body["error"]["retryable"], false);

    let inbox = journal
        .replay()
        .expect("verify and replay the journal chain");
    assert_eq!(
        inbox.len(),
        2,
        "same delivery is idempotent; new UUID is retained"
    );
    assert_eq!(
        inbox[0].idempotency_key,
        "forgejo-delivery-v1:delivery-41-a"
    );
    assert_eq!(
        inbox[1].idempotency_key,
        "forgejo-delivery-v1:delivery-41-b"
    );
    let fixture = FixtureForgejoCurrentState::issue(41, "open");
    let states = reconcile_inbox_events(&inbox, &fixture).expect("bounded batch is accepted");
    assert_eq!(states.len(), 2);
    let reconciled: Vec<ReconciledState> = states
        .into_iter()
        .map(|state| state.expect("fixture response is a canonical issue snapshot"))
        .collect();

    for (state, delivery_id) in reconciled.iter().zip(["delivery-41-a", "delivery-41-b"]) {
        assert_eq!(state.delivery_id, delivery_id);
        assert_eq!(state.resource.kind, ResourceKind::Issue);
        assert_eq!(state.resource.id, ResourceId::Number(41));
        assert_eq!(
            state.state, "open",
            "Forgejo current state overrides stale payload"
        );
        assert_eq!(state.updated_at, "2026-10-22T12:30:00Z");
        assert_eq!(state.html_url, ISSUE_URL);
        assert_eq!(state.head_sha, None);
        assert_eq!(state.base_sha, None);
        assert_eq!(state.merged, None);
    }

    assert_eq!(
        *fixture.reads.lock().unwrap(),
        vec![ResourceRef {
            kind: ResourceKind::Issue,
            owner: "forge-admin".into(),
            repository: "agent-forge-core".into(),
            id: ResourceId::Number(41),
        }],
        "new delivery UUIDs for one issue share one current-state read in a batch"
    );
}

#[test]
fn rejected_issue_action_can_retry_same_delivery_then_reconciles_current_api_state() {
    let scratch = ScratchDir::new();
    let journal = EventJournal::open(scratch.path().join("inbox.jsonl"));
    let delivery_id = "issue-action-corrected-41";

    let invalid_body = serde_json::to_vec(&json!({
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 41, "state": "open"}
    }))
    .expect("serialize signed issue fixture without action");
    let invalid_request =
        parse_http_request(&signed_issue_request(delivery_id, &invalid_body)).unwrap();
    let rejected = handle_webhook_request(&invalid_request, WEBHOOK_SECRET, &journal);
    assert_eq!(rejected.status, 400);
    assert_eq!(rejected.body["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected.body["error"]["retryable"], false);
    assert_eq!(
        rejected.body["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(
        rejected.body["error"]["action"]
            .as_str()
            .is_some_and(|action| action.contains("redeliver with the same delivery ID"))
    );
    assert!(
        journal
            .replay()
            .expect("replay after missing-action refusal")
            .is_empty()
    );

    let corrected_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 41, "state": "open"}
    }))
    .expect("serialize corrected issue event");
    let corrected_request =
        parse_http_request(&signed_issue_request(delivery_id, &corrected_body)).unwrap();
    let accepted = handle_webhook_request(&corrected_request, WEBHOOK_SECRET, &journal);
    assert_eq!(accepted.status, 202);
    assert_eq!(accepted.body["duplicate"], false);
    assert_eq!(accepted.body["sequence"], 1);

    let duplicate = handle_webhook_request(&corrected_request, WEBHOOK_SECRET, &journal);
    assert_eq!(duplicate.status, 202);
    assert_eq!(duplicate.body["duplicate"], true);
    assert_eq!(duplicate.body["sequence"], 1);

    let inbox = journal
        .replay()
        .expect("replay corrected and duplicate issue delivery");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].payload["payload"]["action"], "opened");

    let fixture = FixtureForgejoCurrentState::issue(41, "closed");
    assert!(fixture.reads.lock().unwrap().is_empty());
    let states = reconcile_inbox_events(&inbox, &fixture).expect("single delivery is in budget");
    let reconciled = states
        .into_iter()
        .next()
        .expect("one selected issue delivery")
        .expect("Forgejo returned a canonical issue snapshot");
    assert_eq!(reconciled.delivery_id, delivery_id);
    assert_eq!(reconciled.resource.kind, ResourceKind::Issue);
    assert_eq!(reconciled.resource.id, ResourceId::Number(41));
    assert_eq!(reconciled.state, "closed");
    assert_eq!(reconciled.html_url, ISSUE_URL);
    assert_eq!(
        fixture.reads.lock().unwrap().as_slice(),
        &[ResourceRef {
            kind: ResourceKind::Issue,
            owner: "forge-admin".to_owned(),
            repository: "agent-forge-core".to_owned(),
            id: ResourceId::Number(41),
        }]
    );
}

#[test]
fn late_stale_notification_is_reconciled_to_current_forgejo_state() {
    let scratch = ScratchDir::new();
    let journal = EventJournal::open(scratch.path().join("inbox.jsonl"));
    let newer_body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {
            "number": 41,
            "state": "closed",
            "updated_at": "2026-10-25T14:00:00Z"
        }
    }))
    .unwrap();
    let newer_request =
        parse_http_request(&signed_issue_request("delivery-41-newer", &newer_body)).unwrap();
    let accepted_newer = handle_webhook_request(&newer_request, WEBHOOK_SECRET, &journal);
    assert_eq!(accepted_newer.status, 202);
    assert_eq!(accepted_newer.body["duplicate"], false);

    let duplicate_newer = handle_webhook_request(&newer_request, WEBHOOK_SECRET, &journal);
    assert_eq!(duplicate_newer.status, 202);
    assert_eq!(duplicate_newer.body["duplicate"], true);

    // Forgejo can redeliver an older notification after a newer one. Its new
    // delivery ID is distinct, but its body must not become canonical state.
    let older_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {
            "number": 41,
            "state": "open",
            "updated_at": "2026-10-25T13:00:00Z"
        }
    }))
    .unwrap();
    let late_older_request = parse_http_request(&signed_issue_request(
        "delivery-41-older-arrived-late",
        &older_body,
    ))
    .unwrap();
    let accepted_late_older = handle_webhook_request(&late_older_request, WEBHOOK_SECRET, &journal);
    assert_eq!(accepted_late_older.status, 202);
    assert_eq!(accepted_late_older.body["duplicate"], false);

    let inbox = journal
        .replay()
        .expect("verify and replay the journal chain");
    assert_eq!(
        inbox.len(),
        2,
        "same delivery is idempotent; a distinct late delivery is retained"
    );
    assert_eq!(
        inbox
            .iter()
            .map(|event| event.idempotency_key.as_str())
            .collect::<Vec<_>>(),
        [
            "forgejo-delivery-v1:delivery-41-newer",
            "forgejo-delivery-v1:delivery-41-older-arrived-late"
        ]
    );

    let fixture = FixtureForgejoCurrentState {
        response: json!({
            "number": 41,
            "state": "closed",
            "updated_at": "2026-10-25T14:00:00Z",
            "html_url": ISSUE_URL
        }),
        reads: Mutex::new(Vec::new()),
    };
    let states = reconcile_inbox_events(&inbox, &fixture).expect("bounded batch is accepted");
    let reconciled: Vec<ReconciledState> = states
        .into_iter()
        .map(|state| state.expect("Forgejo returned a canonical issue snapshot"))
        .collect();

    assert_eq!(reconciled.len(), 2);
    assert_eq!(reconciled[0].delivery_id, "delivery-41-newer");
    assert_eq!(reconciled[1].delivery_id, "delivery-41-older-arrived-late");
    for state in &reconciled {
        assert_eq!(state.state, "closed");
        assert_eq!(state.updated_at, "2026-10-25T14:00:00Z");
        assert_eq!(state.html_url, ISSUE_URL);
    }
    assert_eq!(fixture.reads.lock().unwrap().len(), 1);
}

#[test]
fn out_of_order_signed_label_events_use_one_canonical_api_label_snapshot() {
    let scratch = ScratchDir::new();
    let journal = EventJournal::open(scratch.path().join("inbox.jsonl"));

    let newer_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {
            "number": 41,
            "state": "open",
            "updated_at": "2026-10-25T14:00:00Z",
            "labels": [{"id": 99, "name": "stale/newer-webhook"}]
        }
    }))
    .unwrap();
    let newer_request =
        parse_http_request(&signed_issue_request("delivery-label-newer", &newer_body)).unwrap();
    let accepted_newer = handle_webhook_request(&newer_request, WEBHOOK_SECRET, &journal);
    assert_eq!(accepted_newer.status, 202);
    assert_eq!(accepted_newer.body["sequence"], 1);

    let late_older_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {
            "number": 41,
            "state": "open",
            "updated_at": "2026-10-25T13:00:00Z",
            "labels": [{"id": 3, "name": "stale/older-webhook"}]
        }
    }))
    .unwrap();
    let late_older_request = parse_http_request(&signed_issue_request(
        "delivery-label-older-arrived-late",
        &late_older_body,
    ))
    .unwrap();
    let accepted_late_older = handle_webhook_request(&late_older_request, WEBHOOK_SECRET, &journal);
    assert_eq!(accepted_late_older.status, 202);
    assert_eq!(accepted_late_older.body["sequence"], 2);

    let inbox = journal.replay().expect("replay signed issue label events");
    assert_eq!(inbox.len(), 2);
    let fixture = FixtureForgejoCurrentState {
        response: json!({
            "number": 41,
            "state": "open",
            "updated_at": "2026-10-25T14:30:00Z",
            "html_url": ISSUE_URL,
            "labels": [{"id": 12, "name": "canonical/current"}]
        }),
        reads: Mutex::new(Vec::new()),
    };

    let results = reconcile_inbox_events(&inbox, &fixture).expect("reconcile bounded event batch");
    assert_eq!(results.len(), 2);
    for (result, expected_delivery) in results
        .iter()
        .zip(["delivery-label-newer", "delivery-label-older-arrived-late"])
    {
        let reconciled = result.as_ref().expect("current API snapshot is valid");
        assert_eq!(reconciled.delivery_id, expected_delivery);
        assert_eq!(reconciled.state, "open");
        assert_eq!(
            reconciled.labels,
            Some(vec![ReconciledLabel {
                id: 12,
                name: "canonical/current".to_owned(),
            }])
        );
    }
    assert_eq!(
        *fixture.reads.lock().unwrap(),
        vec![ResourceRef {
            kind: ResourceKind::Issue,
            owner: "forge-admin".to_owned(),
            repository: "agent-forge-core".to_owned(),
            id: ResourceId::Number(41),
        }],
        "same issue notifications share one canonical API label read"
    );
}

#[test]
fn signed_branch_create_is_ingested_then_reconciled_to_current_branch_state() {
    let scratch = ScratchDir::new();
    let journal = EventJournal::open(scratch.path().join("inbox.jsonl"));
    let payload_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let current_sha = "cccccccccccccccccccccccccccccccccccccccc";
    let branch_name = "feature/agent-proof";
    let body = serde_json::to_vec(&json!({
        "ref": branch_name,
        "ref_type": "branch",
        "sha": payload_sha,
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize signed branch create payload");
    let request = parse_http_request(&signed_webhook_request(
        "branch-create-pipeline-41",
        "create",
        &body,
    ))
    .expect("parse signed branch create request");

    let accepted = handle_webhook_request(&request, WEBHOOK_SECRET, &journal);
    assert_eq!(accepted.status, 202);
    assert_eq!(accepted.body["reconcile_required"], true);
    assert_eq!(accepted.body["duplicate"], false);

    let inbox = journal.replay().expect("verify and replay journal chain");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "forgejo.webhook.v1.create");
    assert_eq!(inbox[0].payload["payload"]["sha"], payload_sha);

    let current_branch_url =
        format!("{FORGEJO_ORIGIN}/forge-admin/agent-forge-core/src/branch/feature%2Fagent-proof");
    let fixture = FixtureForgejoCurrentState {
        response: json!({
            "name": branch_name,
            "commit": {
                "id": current_sha,
                "timestamp": "2026-10-26T12:30:00Z"
            }
        }),
        reads: Mutex::new(Vec::new()),
    };
    let states = reconcile_inbox_events(&inbox, &fixture).expect("accept bounded event batch");
    assert_eq!(states.len(), 1);
    let state = states
        .into_iter()
        .next()
        .unwrap()
        .expect("canonical branch snapshot");
    assert_eq!(state.delivery_id, "branch-create-pipeline-41");
    assert_eq!(state.resource.kind, ResourceKind::Branch);
    assert_eq!(state.resource.id, ResourceId::Branch(branch_name.into()));
    assert_eq!(state.state, "present");
    assert_eq!(state.head_sha.as_deref(), Some(current_sha));
    assert_ne!(state.head_sha.as_deref(), Some(payload_sha));
    assert_eq!(state.html_url, current_branch_url);
    assert_eq!(
        *fixture.reads.lock().unwrap(),
        vec![ResourceRef {
            kind: ResourceKind::Branch,
            owner: "forge-admin".into(),
            repository: "agent-forge-core".into(),
            id: ResourceId::Branch(branch_name.into()),
        }]
    );
}

struct FixtureLargeCommentCurrentState {
    body: String,
    reads: Mutex<Vec<ResourceRef>>,
}

impl ForgejoCurrentState for FixtureLargeCommentCurrentState {
    fn read_current(&self, resource: &ResourceRef) -> Result<Value, ApiReadError> {
        self.reads.lock().unwrap().push(resource.clone());
        match resource.kind {
            ResourceKind::Issue => Ok(json!({
                "number": 41,
                "state": "open",
                "updated_at": "2026-10-27T12:30:00Z",
                "html_url": ISSUE_URL,
                "repository": {
                    "full_name": "forge-admin/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": "forge-admin"
                }
            })),
            ResourceKind::IssueComment => Ok(json!({
                "id": 9001,
                "issue_url": ISSUE_URL,
                "pull_request_url": null,
                "html_url": format!("{ISSUE_URL}#issuecomment-9001"),
                "updated_at": "2026-10-27T12:31:00Z",
                "body": self.body
            })),
            _ => Err(ApiReadError::InvalidResponse),
        }
    }

    fn expected_html_url(&self, resource: &ResourceRef) -> String {
        match (&resource.kind, &resource.id) {
            (ResourceKind::Issue, ResourceId::Number(41)) => ISSUE_URL.to_owned(),
            _ => String::new(),
        }
    }
}

#[test]
fn repeated_comment_deliveries_fail_closed_when_projected_batch_output_exceeds_budget() {
    const DELIVERY_COUNT: usize = 100;
    const COMMENT_BODY_BYTES: usize = 128 * 1024;

    let scratch = ScratchDir::new();
    let journal = EventJournal::open(scratch.path().join("inbox.jsonl"));
    let mut delivery_ids = Vec::with_capacity(DELIVERY_COUNT);
    for index in 0..DELIVERY_COUNT {
        let delivery_id = format!("large-comment-output-{index}");
        let body = serde_json::to_vec(&json!({
            "action": "created",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "issue": {"number": 41},
            "comment": {"id": 9001},
            "is_pull": false
        }))
        .unwrap();
        let request = parse_http_request(&signed_webhook_request(
            &delivery_id,
            "issue_comment",
            &body,
        ))
        .expect("parse bounded signed comment delivery");
        let accepted = handle_webhook_request(&request, WEBHOOK_SECRET, &journal);
        assert_eq!(accepted.status, 202);
        assert_eq!(accepted.body["duplicate"], false);
        delivery_ids.push(delivery_id);
    }
    let events = journal.replay().expect("read append-only webhook journal");
    assert_eq!(events.len(), DELIVERY_COUNT);

    let fixture = FixtureLargeCommentCurrentState {
        body: "x".repeat(COMMENT_BODY_BYTES),
        reads: Mutex::new(Vec::new()),
    };
    let error = match reconcile_inbox_events(&events, &fixture) {
        Err(error) => error,
        Ok(states) => panic!(
            "the batch must refuse repeated comment output instead of materializing {} states",
            states.len()
        ),
    };

    assert!(matches!(
        error,
        ReconcileError::BatchOutputBudgetExceeded {
            maximum_output_bytes: MAX_RECONCILE_BATCH_OUTPUT_BYTES
        }
    ));
    assert_eq!(fixture.reads.lock().unwrap().len(), 2);
}
