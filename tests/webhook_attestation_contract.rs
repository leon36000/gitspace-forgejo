use agent_forge_core::{
    EventJournal, JournalAttestationError, JournalEvent, verify_journal_event_attestation,
    webhook::{
        WebhookError,
        http::{handle_webhook_request_attested, parse_http_request},
        ingest_forgejo_delivery, read_journaled_webhook_event_verified,
    },
};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

const HMAC_SECRET: &[u8] = b"attestation-contract-fixture-secret";

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "webhook-attestation-contract-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create scratch directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("restrict scratch directory");
        Self(path)
    }

    fn journal(&self) -> EventJournal {
        EventJournal::open_webhook_inbox(self.0.join("inbox.jsonl"))
    }

    fn journal_path(&self) -> PathBuf {
        self.0.join("inbox.jsonl")
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn issue_body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 9}
    }))
    .unwrap()
}

fn signature(body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(HMAC_SECRET).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn signed_http_request(delivery_id: &str, body: &[u8]) -> Vec<u8> {
    format!(
        "POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Forgejo-Delivery: {delivery_id}\r\nX-Forgejo-Event: issues\r\nX-Forgejo-Signature: {}\r\n\r\n{}",
        body.len(),
        signature(body),
        String::from_utf8(body.to_vec()).unwrap()
    )
    .into_bytes()
}

#[test]
fn actual_http_handler_persists_receiver_attestation_verified_by_public_key_only() {
    let scratch = ScratchDir::new();
    let journal = scratch.journal();
    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let request_bytes = signed_http_request("attested-delivery-1", &issue_body());
    let request = parse_http_request(&request_bytes).expect("parse signed webhook request");

    let response = handle_webhook_request_attested(&request, HMAC_SECRET, &journal, &signing_key);

    assert_eq!(response.status, 202);
    assert_eq!(journal.replay_read_only().unwrap().len(), 1);
    let event =
        read_journaled_webhook_event_verified(&journal, "attested-delivery-1", &[verifying_key])
            .expect("the separate public key verifies the persisted line");
    assert_eq!(event.event().sequence, 1);
    assert!(event.event().receiver_attestation.is_some());

    let wrong_key = SigningKey::from_bytes(&[8u8; 32]).verifying_key();
    assert!(matches!(
        verify_journal_event_attestation(event.event(), &[wrong_key]),
        Err(JournalAttestationError::UntrustedKey)
    ));
    let mut altered_payload = event.event().clone();
    altered_payload.payload["payload"]["issue"]["number"] = json!(10);
    assert!(matches!(
        verify_journal_event_attestation(event.event(), &[signing_key.verifying_key()]),
        Ok(())
    ));
    assert!(matches!(
        verify_journal_event_attestation(&altered_payload, &[signing_key.verifying_key()]),
        Err(JournalAttestationError::InvalidSignature)
    ));
    let mut altered_sequence = event.event().clone();
    altered_sequence.sequence += 1;
    assert!(matches!(
        verify_journal_event_attestation(&altered_sequence, &[signing_key.verifying_key()]),
        Err(JournalAttestationError::InvalidSignature)
    ));
    let mut altered_previous_hash = event.event().clone();
    altered_previous_hash.previous_hash = "1".repeat(64);
    assert!(matches!(
        verify_journal_event_attestation(&altered_previous_hash, &[signing_key.verifying_key()]),
        Err(JournalAttestationError::InvalidSignature)
    ));
}

#[test]
fn legacy_webhook_is_rejected_by_the_strict_public_key_reader_and_bad_hmac_is_not_appended() {
    let scratch = ScratchDir::new();
    let journal = scratch.journal();
    let body = issue_body();
    let signing_key = SigningKey::from_bytes(&[9u8; 32]);

    ingest_forgejo_delivery(
        &journal,
        HMAC_SECRET,
        "legacy-delivery-1",
        "issues",
        &signature(&body),
        &body,
    )
    .expect("legacy compatibility path accepts the valid HMAC");
    assert!(matches!(
        read_journaled_webhook_event_verified(
            &journal,
            "legacy-delivery-1",
            &[signing_key.verifying_key()]
        ),
        Err(WebhookError::Attestation(JournalAttestationError::Missing))
    ));

    let result = agent_forge_core::webhook::ingest_forgejo_delivery_attested(
        &journal,
        HMAC_SECRET,
        &signing_key,
        "bad-hmac-delivery-1",
        "issues",
        &"00".repeat(32),
        &body,
    );
    assert!(matches!(result, Err(WebhookError::InvalidSignature)));
    assert_eq!(journal.replay_read_only().unwrap().len(), 1);
}

#[test]
fn tampered_persisted_attestation_is_rejected_even_when_the_unkeyed_chain_is_recomputed() {
    let scratch = ScratchDir::new();
    let journal = scratch.journal();
    let signing_key = SigningKey::from_bytes(&[10u8; 32]);
    let body = issue_body();
    agent_forge_core::webhook::ingest_forgejo_delivery_attested(
        &journal,
        HMAC_SECRET,
        &signing_key,
        "tampered-delivery-1",
        "issues",
        &signature(&body),
        &body,
    )
    .unwrap();

    let path = scratch.journal_path();
    let mut row: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let old_signature = row["receiver_attestation"]["signature_hex"]
        .as_str()
        .unwrap()
        .to_owned();
    let first = if old_signature.starts_with('0') {
        '1'
    } else {
        '0'
    };
    row["receiver_attestation"]["signature_hex"] =
        Value::String(format!("{first}{}", &old_signature[1..]));
    row["hash"] = Value::String(recompute_attested_event_hash(&row));
    fs::write(
        &path,
        [serde_json::to_vec(&row).unwrap(), b"\n".to_vec()].concat(),
    )
    .unwrap();

    assert_eq!(journal.replay_read_only().unwrap().len(), 1);
    assert!(matches!(
        read_journaled_webhook_event_verified(
            &journal,
            "tampered-delivery-1",
            &[signing_key.verifying_key()]
        ),
        Err(WebhookError::Attestation(
            JournalAttestationError::InvalidSignature
        ))
    ));
}

fn recompute_attested_event_hash(row: &Value) -> String {
    let hashed = json!({
        "sequence": row["sequence"],
        "event_id": row["event_id"],
        "idempotency_key": row["idempotency_key"],
        "kind": row["kind"],
        "payload": row["payload"],
        "previous_hash": row["previous_hash"],
        "receiver_attestation": row["receiver_attestation"],
    });
    hex::encode(Sha256::digest(serde_json::to_vec(&hashed).unwrap()))
}

#[allow(dead_code)]
fn _event_type_anchor(_: JournalEvent) {}
