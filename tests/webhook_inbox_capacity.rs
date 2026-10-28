use agent_forge_core::webhook::{
    http::{handle_webhook_request, parse_http_request},
    ingest_forgejo_delivery, read_journaled_webhook_event,
};
use agent_forge_core::{EventJournal, webhook::MAX_WEBHOOK_BODY_BYTES};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    net::TcpListener,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;
const MAX_INBOX_BYTES: u64 = 64 * 1024 * 1024;

fn scratch_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "forgejo-webhook-capacity-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("create private test directory");
    path
}

fn signed_request(delivery_id: &str, body: &[u8]) -> Vec<u8> {
    assert!(body.len() <= MAX_WEBHOOK_BODY_BYTES);
    let secret = b"capacity-test-secret";
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    let signature = hex::encode(mac.finalize().into_bytes());
    format!(
        "POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Forgejo-Delivery: {delivery_id}\r\nX-Forgejo-Event: issues\r\nX-Forgejo-Signature: {signature}\r\n\r\n{}",
        body.len(),
        String::from_utf8(body.to_vec()).unwrap()
    )
    .into_bytes()
}

fn valid_issue_body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 41, "state": "open"}
    }))
    .expect("serialize valid issue webhook payload")
}

#[test]
fn capabilities_disclose_the_total_inbox_limit_and_non_truncating_policy() {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .arg("--capabilities")
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation capability discovery");

    assert!(
        output.status.success(),
        "capability discovery failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("capability JSON");
    let operation = value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["id"] == "reconcile_webhook_deliveries")
        .expect("reconciliation capability");
    assert_eq!(
        operation["inbox_journal_capacity"]["maximum_bytes"],
        MAX_INBOX_BYTES
    );
    assert_eq!(
        operation["inbox_journal_capacity"]["overflow_policy"],
        "reject_without_truncation"
    );
}

#[test]
fn reconcile_refuses_an_oversized_inbox_without_changing_it() {
    let root = scratch_dir();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("make inbox fixture directory private");
    let path = root.join("inbox.jsonl");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create private sparse inbox");
    file.set_len(MAX_INBOX_BYTES + 1)
        .expect("make inbox exceed configured size");
    file.sync_all().expect("sync oversized inbox fixture");

    let journal = EventJournal::open(&path);
    let replay = read_journaled_webhook_event(&journal, "delivery-not-present");
    assert!(
        format!("{replay:?}").contains("InboxTooLarge"),
        "expected the shared inbox reader to refuse oversized history, got {replay:?}"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&path)
        .arg("delivery-not-present")
        .stdin(Stdio::null())
        .output()
        .expect("run actual reconciliation CLI");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("inbox_capacity_exceeded"),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), MAX_INBOX_BYTES + 1);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn receiver_refuses_an_oversized_inbox_before_binding() {
    let root = scratch_dir();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("make receiver fixture private");
    let secret_path = root.join("secret");
    let mut secret = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&secret_path)
        .expect("create private webhook secret");
    secret.write_all(b"test-secret").unwrap();
    secret.sync_all().unwrap();
    let path = root.join("inbox.jsonl");
    let inbox = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .expect("create private sparse inbox");
    inbox
        .set_len(MAX_INBOX_BYTES + 1)
        .expect("make inbox exceed configured size");
    inbox.sync_all().unwrap();
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
        .env_clear()
        .arg(address.to_string())
        .arg(&secret_path)
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual webhook receiver");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("webhook inbox exceeds"),
        "unexpected startup error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), MAX_INBOX_BYTES + 1);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn full_inbox_returns_actionable_http_error_without_creating_an_event() {
    let root = scratch_dir();
    let path = root.join("inbox.jsonl");
    let journal = EventJournal::open_with_max_bytes(&path, 0);
    let body = valid_issue_body();
    let request =
        parse_http_request(&signed_request("delivery-full", &body)).expect("parse signed request");

    let response = handle_webhook_request(&request, b"capacity-test-secret", &journal);

    assert_eq!(response.status, 507);
    assert_eq!(response.body["error"]["code"], "inbox_capacity_exceeded");
    assert_eq!(response.body["error"]["retryable"], false);
    assert!(
        response.body["error"]["action"]
            .as_str()
            .unwrap()
            .contains("backup")
    );
    assert!(
        !path.exists(),
        "capacity refusal must not create the inbox file"
    );
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn replay_does_not_repair_an_unterminated_tail_past_the_configured_limit() {
    let root = scratch_dir();
    let path = root.join("inbox.jsonl");
    let journal = EventJournal::open(&path);
    journal
        .append("record", "task", serde_json::json!({"value": 1}))
        .expect("append journal record");
    let current_length = fs::metadata(&path).unwrap().len() - 1;
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(current_length)
        .expect("remove final newline to model interrupted append");
    let bounded = EventJournal::open_with_max_bytes(&path, current_length);

    let replay = bounded.replay();

    assert!(
        format!("{replay:?}").contains("InboxTooLarge"),
        "expected replay to refuse an over-limit newline repair, got {replay:?}"
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), current_length);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn duplicate_at_exact_limit_is_accepted_but_a_new_delivery_is_not_appended() {
    let root = scratch_dir();
    let path = root.join("inbox.jsonl");
    let first = EventJournal::open(&path);
    let body = valid_issue_body();
    let signature = {
        let mut mac = HmacSha256::new_from_slice(b"capacity-test-secret").unwrap();
        mac.update(&body);
        hex::encode(mac.finalize().into_bytes())
    };

    assert!(
        ingest_forgejo_delivery(
            &first,
            b"capacity-test-secret",
            "delivery-at-cap",
            "issues",
            &signature,
            &body,
        )
        .is_ok()
    );
    let original = fs::read(&path).expect("read initial delivery");
    let bounded = EventJournal::open_with_max_bytes(&path, original.len() as u64);

    let duplicate = ingest_forgejo_delivery(
        &bounded,
        b"capacity-test-secret",
        "delivery-at-cap",
        "issues",
        &signature,
        &body,
    );
    assert!(
        duplicate.is_ok(),
        "same delivery should remain idempotent at capacity"
    );
    let rejected = ingest_forgejo_delivery(
        &bounded,
        b"capacity-test-secret",
        "delivery-new-at-cap",
        "issues",
        &signature,
        &body,
    );
    assert!(
        format!("{rejected:?}").contains("InboxTooLarge"),
        "expected an explicit inbox limit error, got {rejected:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private test fixture");
}
