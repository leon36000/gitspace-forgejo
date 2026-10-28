use agent_forge_core::{
    AppendResult, EventJournal,
    webhook::{WebhookError, ingest_forgejo_delivery},
};
use hmac::{Hmac, Mac};
use serde_json::json;
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
        "forgejo-webhook-contract-{}-{stamp}.jsonl",
        std::process::id()
    ))
}

fn signature(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn body(action: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": action,
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 9}
    }))
    .unwrap()
}

#[test]
fn identical_delivery_is_duplicate_after_journal_reopen() {
    let path = temp_journal();
    let bytes = body("opened");
    let sig = signature(b"test-secret", &bytes);

    let first = EventJournal::open(&path);
    assert!(matches!(
        ingest_forgejo_delivery(&first, b"test-secret", "delivery-1", "issues", &sig, &bytes),
        Ok(AppendResult::Appended(_))
    ));
    drop(first);

    let resumed = EventJournal::open(&path);
    assert!(matches!(
        ingest_forgejo_delivery(
            &resumed,
            b"test-secret",
            "delivery-1",
            "issues",
            &sig,
            &bytes
        ),
        Ok(AppendResult::Duplicate(_))
    ));
    assert_eq!(resumed.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn invalid_signature_is_rejected_without_journal_side_effect() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let bytes = body("opened");

    let result = ingest_forgejo_delivery(
        &journal,
        b"test-secret",
        "delivery-2",
        "issues",
        &"00".repeat(32),
        &bytes,
    );

    assert!(matches!(result, Err(WebhookError::InvalidSignature)));
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn signed_repository_identity_conflicts_are_rejected_without_journal_side_effect() {
    for (delivery_id, name, login) in [
        ("repository-name-conflict", "other-repo", "forge-admin"),
        (
            "repository-owner-conflict",
            "agent-forge-core",
            "other-owner",
        ),
    ] {
        let path = temp_journal();
        let journal = EventJournal::open(&path);
        let bytes = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {
                "full_name": "forge-admin/agent-forge-core",
                "name": name,
                "owner": {"login": login}
            },
            "issue": {"number": 9}
        }))
        .unwrap();

        let result = ingest_forgejo_delivery(
            &journal,
            b"test-secret",
            delivery_id,
            "issues",
            &signature(b"test-secret", &bytes),
            &bytes,
        );

        assert!(
            matches!(
                result,
                Err(WebhookError::InvalidEventPayloadField(
                    "repository.full_name"
                ))
            ),
            "contradictory repository.{name}/{login} metadata must be refused at repository.full_name"
        );
        assert!(journal.replay().unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
    }
}

#[test]
fn consistent_repository_identity_metadata_is_accepted() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let bytes = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {
            "full_name": "forge-admin/agent-forge-core",
            "name": "agent-forge-core",
            "owner": {"login": "forge-admin"}
        },
        "issue": {"number": 9}
    }))
    .unwrap();

    assert!(matches!(
        ingest_forgejo_delivery(
            &journal,
            b"test-secret",
            "repository-identity-consistent",
            "issues",
            &signature(b"test-secret", &bytes),
            &bytes,
        ),
        Ok(AppendResult::Appended(_))
    ));
    assert_eq!(journal.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn signed_issue_payload_with_non_object_review_member_is_rejected_without_journal_side_effect() {
    for (delivery_id, review) in [
        ("issue-review-null", serde_json::Value::Null),
        ("issue-review-bool", serde_json::Value::Bool(false)),
        (
            "issue-review-string",
            serde_json::Value::String("pull_request_review_approved".to_owned()),
        ),
        ("issue-review-array", json!([])),
    ] {
        let path = temp_journal();
        let journal = EventJournal::open(&path);
        let bytes = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "issue": {"number": 9},
            "review": review
        }))
        .unwrap();

        let result = ingest_forgejo_delivery(
            &journal,
            b"test-secret",
            delivery_id,
            "issues",
            &signature(b"test-secret", &bytes),
            &bytes,
        );

        assert!(
            matches!(
                result,
                Err(WebhookError::InvalidEventPayloadField("review"))
            ),
            "unexpected review field {delivery_id} must be refused"
        );
        assert!(journal.replay().unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
    }
}

#[test]
fn signed_payload_with_duplicate_json_member_is_rejected_without_journal_side_effect() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let bytes = br#"{"action":"opened","repository":{"full_name":"forge-admin/agent-forge-core","full_name":"attacker/agent-forge-core"},"issue":{"number":9}}"#;

    let result = ingest_forgejo_delivery(
        &journal,
        b"test-secret",
        "duplicate-member-1",
        "issues",
        &signature(b"test-secret", bytes),
        bytes,
    );

    assert!(matches!(result, Err(WebhookError::InvalidJsonAt { .. })));
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn signed_payload_with_escaped_duplicate_json_member_is_rejected_without_journal_side_effect() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let bytes = br#"{"action":"opened","repository":{"full_name":"forge-admin/agent-forge-core","full\u005fname":"attacker/agent-forge-core"},"issue":{"number":9}}"#;

    let result = ingest_forgejo_delivery(
        &journal,
        b"test-secret",
        "duplicate-member-escaped-1",
        "issues",
        &signature(b"test-secret", bytes),
        bytes,
    );

    assert!(matches!(result, Err(WebhookError::InvalidJsonAt { .. })));
    assert!(journal.replay().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn same_delivery_id_with_changed_payload_is_conflict() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let first = body("opened");
    let changed = body("closed");

    ingest_forgejo_delivery(
        &journal,
        b"test-secret",
        "delivery-3",
        "issues",
        &signature(b"test-secret", &first),
        &first,
    )
    .unwrap();
    let result = ingest_forgejo_delivery(
        &journal,
        b"test-secret",
        "delivery-3",
        "issues",
        &signature(b"test-secret", &changed),
        &changed,
    );

    assert!(matches!(result, Err(WebhookError::IdempotencyConflict)));
    assert_eq!(journal.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn late_older_notification_is_appended_and_does_not_rewrite_history() {
    let path = temp_journal();
    let journal = EventJournal::open(&path);
    let newer = body("closed");
    let older = body("opened");

    ingest_forgejo_delivery(
        &journal,
        b"test-secret",
        "delivery-newer",
        "issues",
        &signature(b"test-secret", &newer),
        &newer,
    )
    .unwrap();
    ingest_forgejo_delivery(
        &journal,
        b"test-secret",
        "delivery-older",
        "issues",
        &signature(b"test-secret", &older),
        &older,
    )
    .unwrap();

    let events = journal.replay().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["payload"]["action"], "closed");
    assert_eq!(events[1].payload["payload"]["action"], "opened");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}

#[test]
fn concurrent_retries_append_exactly_once() {
    let path = temp_journal();
    let journal = Arc::new(EventJournal::open(&path));
    let bytes = body("opened");
    let sig = signature(b"test-secret", &bytes);
    let barrier = Arc::new(Barrier::new(8));
    let mut workers = Vec::new();

    for _ in 0..8 {
        let journal = Arc::clone(&journal);
        let barrier = Arc::clone(&barrier);
        let bytes = bytes.clone();
        let sig = sig.clone();
        workers.push(thread::spawn(move || {
            barrier.wait();
            ingest_forgejo_delivery(
                &journal,
                b"test-secret",
                "delivery-8",
                "issues",
                &sig,
                &bytes,
            )
        }));
    }

    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    let appended = results
        .iter()
        .filter(|result| matches!(result, Ok(AppendResult::Appended(_))))
        .count();
    let duplicates = results
        .iter()
        .filter(|result| matches!(result, Ok(AppendResult::Duplicate(_))))
        .count();
    assert_eq!(appended, 1);
    assert_eq!(duplicates, 7);
    assert_eq!(journal.replay().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
}
