//! Process checks for `forgejo-mission-local --delivery-id`.
//!
//! These tests launch the compiled CLI. The binary is fixed to the loopback
//! Forgejo origin and to this checkout's `.runtime` inbox, so they only reach
//! refusals that happen before any Forgejo request. Inbox scenarios that need
//! a populated journal run in the binary's in-process fixture tests instead.

use agent_forge_core::{
    EventJournal,
    webhook::{ingest_forgejo_delivery_attested, read_journaled_webhook_event},
};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

const BINDING: [&str; 10] = [
    "--issue",
    "41",
    "--requirement",
    "REQ-001",
    "--revision",
    "1",
    "--allocation-id",
    "admission-41",
    "--workspace-id",
    "local-demo",
];

fn run_cli(extra: &[&str]) -> (Option<i32>, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-mission-local"))
        .args(BINDING)
        .args(extra)
        .output()
        .expect("launch forgejo-mission-local");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    let value = serde_json::from_str(stdout.trim()).expect("one JSON document on stdout");
    (output.status.code(), value)
}

#[test]
fn launched_help_documents_delivery_mode_and_the_fixed_inbox() {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-mission-local"))
        .arg("--help")
        .output()
        .expect("launch forgejo-mission-local");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(output.status.success());
    assert!(
        value["usage"]
            .as_str()
            .unwrap()
            .contains("--delivery-id ID --requirement-sha256 HEX")
    );
    assert_eq!(value["inbox"], ".runtime/forgejo-webhook-inbox.jsonl");
    assert_eq!(
        value["supported_agent_execution_request_schema_versions"],
        json!([1, 2, 3, 4])
    );
    assert_eq!(value["current_issue_context"]["legacy_schema_version"], 1);
    assert_eq!(value["current_issue_context"]["labels_schema_version"], 2);
    assert_eq!(
        value["current_issue_context"]["milestone_schema_version"],
        3
    );
    assert_eq!(
        value["current_issue_context"]["labels_source"],
        "current_forgejo_issue_api"
    );
    assert_eq!(
        value["current_issue_context"]["labels_are_untrusted_user_content"],
        true
    );
    assert_eq!(
        value["current_issue_context"]["milestone_source"],
        "current_forgejo_issue_api"
    );
    assert_eq!(
        value["current_issue_context"]["missing_milestone_means_unknown"],
        true
    );
    assert_eq!(
        value["current_issue_context"]["null_milestone_means_unassigned"],
        true
    );
    assert!(
        value["direct_admission"]
            .as_str()
            .unwrap()
            .contains("caller is not authenticated")
    );
}

#[test]
fn launched_cli_refuses_delivery_mode_without_a_digest_or_with_an_unsafe_id() {
    let digest = "0".repeat(64);
    for (extra, code) in [
        (
            vec!["--delivery-id", "delivery-41-a"],
            "mission_arguments_missing",
        ),
        (
            vec!["--delivery-id", "../inbox", "--requirement-sha256", &digest],
            "mission_arguments_invalid",
        ),
    ] {
        let (status, value) = run_cli(&extra);
        assert_eq!(status, Some(1));
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["error"]["code"], code);
    }
}

#[test]
#[ignore = "run via isolated bwrap runner with scratch .runtime"]
fn launched_cli_requires_attestation_policy_before_reading_host_credential() {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    let missing_credential = runtime.join("credential-must-not-be-read");
    let digest = "0".repeat(64);
    let (status, value) = run_cli(&[
        "--delivery-id",
        "delivery-41-attestation-policy",
        "--requirement-sha256",
        &digest,
        "--host-credential-file",
        missing_credential.to_str().unwrap(),
    ]);

    assert_eq!(status, Some(1));
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(
        value["error"]["code"],
        "host_journal_attestation_policy_missing"
    );
    assert!(!missing_credential.exists());
    assert!(!runtime.join("mission-execution.jsonl").exists());
}

#[test]
#[ignore = "run via isolated bwrap runner with scratch .runtime"]
fn launched_cli_rejects_untrusted_attestation_before_reading_host_credential() {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    let inbox_path = runtime.join("forgejo-webhook-inbox.jsonl");
    let credential = runtime.join("credential-must-not-be-read");
    let delivery_id = "delivery-41-untrusted-attestation";
    let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
    let wrong_key = SigningKey::from_bytes(&[43_u8; 32]);
    let body = serde_json::to_vec(&json!({
        "action":"edited",
        "repository":{"full_name":"forge-admin/agent-forge-core"},
        "issue":{"number":41,"state":"open"}
    }))
    .unwrap();
    let mut mac = HmacSha256::new_from_slice(b"synthetic-process-e2e-secret-only").unwrap();
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let inbox = EventJournal::open(&inbox_path);
    ingest_forgejo_delivery_attested(
        &inbox,
        b"synthetic-process-e2e-secret-only",
        &signing_key,
        delivery_id,
        "issues",
        &signature,
        &body,
    )
    .expect("append an HMAC- and Ed25519-verified synthetic delivery");
    drop(inbox);

    let (status, value) = run_cli(&[
        "--delivery-id",
        delivery_id,
        "--requirement-sha256",
        &"0".repeat(64),
        "--trusted-webhook-attestation-public-key-hex",
        &hex::encode(wrong_key.verifying_key().as_bytes()),
        "--host-credential-file",
        credential.to_str().unwrap(),
    ]);

    assert_eq!(status, Some(1));
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(
        value["error"]["code"],
        "host_journal_attestation_untrusted_key"
    );
    assert!(!credential.exists());
    assert!(!runtime.join("mission-execution.jsonl").exists());
}

#[test]
#[ignore = "run via isolated bwrap runner with scratch .runtime"]
fn launched_cli_rejects_semantically_valid_rehashed_tampering_before_credential() {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    let inbox_path = runtime.join("forgejo-webhook-inbox.jsonl");
    let credential = runtime.join("credential-must-not-be-read");
    let delivery_id = "delivery-41-semantic-attestation-tamper";
    let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
    let body = serde_json::to_vec(&json!({
        "action":"edited",
        "repository":{"full_name":"forge-admin/agent-forge-core"},
        "issue":{"number":41,"state":"open","title":"Original title"}
    }))
    .expect("serialize semantically valid issue event");
    let secret = b"synthetic-process-e2e-secret-only";
    let mut mac = HmacSha256::new_from_slice(secret).expect("valid synthetic HMAC secret");
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let inbox = EventJournal::open(&inbox_path);
    ingest_forgejo_delivery_attested(
        &inbox,
        secret,
        &signing_key,
        delivery_id,
        "issues",
        &signature,
        &body,
    )
    .expect("append HMAC-verified, receiver-attested source event");
    drop(inbox);

    let mut row: Value = serde_json::from_slice(
        fs::read(&inbox_path)
            .expect("read receiver journal")
            .strip_suffix(b"\n")
            .expect("journal row has a terminating newline"),
    )
    .expect("parse attested journal row");
    row["payload"]["payload"]["issue"]["title"] = json!("Tampered title");
    let mut hash_fields = json!({
        "sequence": row["sequence"].clone(),
        "event_id": row["event_id"].clone(),
        "idempotency_key": row["idempotency_key"].clone(),
        "kind": row["kind"].clone(),
        "payload": row["payload"].clone(),
        "previous_hash": row["previous_hash"].clone(),
    });
    hash_fields
        .as_object_mut()
        .expect("event hash is an object")
        .insert(
            "receiver_attestation".to_owned(),
            row["receiver_attestation"].clone(),
        );
    row["hash"] = json!(hex::encode(Sha256::digest(
        serde_json::to_vec(&hash_fields).expect("serialize hash input")
    )));
    let mut rewritten = serde_json::to_vec(&row).expect("serialize rewritten journal row");
    rewritten.push(b'\n');
    fs::write(&inbox_path, &rewritten).expect("write rehashed but invalidly attested row");

    let inbox = EventJournal::open(&inbox_path);
    let legacy = read_journaled_webhook_event(&inbox, delivery_id)
        .expect("hash-chain and event-contract reader accepts the changed title");
    assert_eq!(
        legacy.event().payload["payload"]["issue"]["title"],
        "Tampered title"
    );
    assert_eq!(
        inbox.replay().expect("rewritten hash chain is valid").len(),
        1
    );

    let trusted_key = hex::encode(signing_key.verifying_key().as_bytes());
    let requirement_digest = "0".repeat(64);
    let (status, value) = run_cli(&[
        "--delivery-id",
        delivery_id,
        "--requirement-sha256",
        &requirement_digest,
        "--trusted-webhook-attestation-public-key-hex",
        &trusted_key,
        "--host-credential-file",
        credential.to_str().expect("credential path is UTF-8"),
    ]);

    assert_eq!(status, Some(1));
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "host_journal_attestation_invalid");
    assert!(
        !credential.exists(),
        "credential remains unread and unopened"
    );
    assert!(!runtime.join("mission-execution.jsonl").exists());
    assert_eq!(
        fs::read(&inbox_path).expect("read journal after process refusal"),
        rewritten,
        "the refusal does not repair or rewrite the selected journal row"
    );
}

#[test]
#[ignore = "run via isolated bwrap runner with scratch .runtime"]
fn launched_cli_reports_torn_attested_journal_before_credential_without_repair() {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    let inbox_path = runtime.join("forgejo-webhook-inbox.jsonl");
    let credential = runtime.join("credential-must-not-be-read");
    let handoff = runtime.join("mission-host-admission.json");
    let delivery_id = "delivery-41-torn-attested-journal";
    let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
    let body = serde_json::to_vec(&json!({
        "action":"opened",
        "repository":{"full_name":"forge-admin/agent-forge-core"},
        "issue":{"number":41,"state":"open"}
    }))
    .expect("serialize valid synthetic issue delivery");
    let secret = b"synthetic-process-e2e-secret-only";
    let mut mac = HmacSha256::new_from_slice(secret).expect("valid synthetic HMAC secret");
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let inbox = EventJournal::open(&inbox_path);
    ingest_forgejo_delivery_attested(
        &inbox,
        secret,
        &signing_key,
        delivery_id,
        "issues",
        &signature,
        &body,
    )
    .expect("append HMAC-verified and receiver-attested delivery");
    drop(inbox);

    let mut torn = OpenOptions::new()
        .append(true)
        .open(&inbox_path)
        .expect("open the private inbox for a synthetic interrupted append");
    torn.write_all(b"{\"sequence\":")
        .expect("append only an incomplete second JSONL record");
    torn.sync_all().expect("persist the incomplete tail");
    drop(torn);
    let inbox_after_torn_tail = fs::read(&inbox_path).expect("snapshot torn inbox bytes");
    assert!(inbox_after_torn_tail.ends_with(b"{\"sequence\":"));

    let public_key = hex::encode(signing_key.verifying_key().as_bytes());
    let requirement_digest = "0".repeat(64);
    let (status, value) = run_cli(&[
        "--delivery-id",
        delivery_id,
        "--requirement-sha256",
        &requirement_digest,
        "--trusted-webhook-attestation-public-key-hex",
        &public_key,
        "--host-credential-file",
        credential.to_str().expect("credential path is UTF-8"),
        "--host-admission-file",
        handoff.to_str().expect("handoff path is UTF-8"),
    ]);

    assert_eq!(status, Some(1));
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "webhook_inbox_corrupt");
    assert_eq!(value["error"]["details"]["line"], 2);
    assert_eq!(value["error"]["details"]["canonical_state_created"], false);
    assert!(
        value["error"]["action"]
            .as_str()
            .unwrap()
            .contains("preserve a copy of the inbox"),
        "the diagnostic must tell the operator how to recover without repairing evidence"
    );
    assert!(
        !credential.exists(),
        "the missing credential path must not be opened after journal corruption"
    );
    assert!(
        !runtime.join("mission-execution.jsonl").exists(),
        "corrupt inbox must not allocate a mission"
    );
    assert!(
        !handoff.exists(),
        "corrupt inbox must not write a host handoff"
    );
    assert_eq!(
        fs::read(&inbox_path).expect("read inbox after refusal"),
        inbox_after_torn_tail,
        "the admission reader must not repair or truncate the interrupted append"
    );
}

#[test]
#[ignore = "run via isolated bwrap runner with scratch .runtime"]
fn launched_cli_refuses_an_unknown_delivery_without_creating_runtime_state() {
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let inbox_existed = fs::symlink_metadata(&inbox).is_ok();
    let journal_before = fs::read(&mission_journal).ok();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let delivery_id = format!("process-unknown-{}-{nanos}", std::process::id());
    let public_key = hex::encode(
        SigningKey::from_bytes(&[42_u8; 32])
            .verifying_key()
            .to_bytes(),
    );

    let (status, value) = run_cli(&[
        "--delivery-id",
        &delivery_id,
        "--requirement-sha256",
        &"0".repeat(64),
        "--trusted-webhook-attestation-public-key-hex",
        &public_key,
    ]);

    assert_eq!(status, Some(1));
    assert_eq!(value["verdict"], "inconclusive");
    let expected = if inbox_existed {
        "webhook_delivery_not_found"
    } else {
        "webhook_inbox_missing"
    };
    assert_eq!(value["error"]["code"], expected);
    assert_eq!(fs::symlink_metadata(&inbox).is_ok(), inbox_existed);
    assert_eq!(fs::read(&mission_journal).ok(), journal_before);
}
