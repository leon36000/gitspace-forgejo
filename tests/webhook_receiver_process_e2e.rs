#![cfg(unix)]

use agent_forge_core::webhook::read_journaled_webhook_event_verified;
use agent_forge_core::{
    EventJournal, sha256,
    webhook::reconcile::{
        ApiReadError, ForgejoCurrentState, MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
        MAX_RECONCILE_BATCH_OUTPUT_BYTES, ReconciledState, ResourceId, ResourceKind, ResourceRef,
        reconcile_inbox_events,
    },
    webhook::{
        MAX_WEBHOOK_BODY_BYTES, SUPPORTED_EVENT_CONTRACTS_V7, SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
        SUPPORTED_WORKFLOW_STATUS_CONTRACTS, WEBHOOK_EVENT_CONTRACTS_V4,
        WEBHOOK_EVENT_CONTRACTS_V5, WEBHOOK_EVENT_CONTRACTS_V6, WEBHOOK_EVENT_CONTRACTS_V7,
        WEBHOOK_EVENT_CONTRACTS_V8, WEBHOOK_EVENT_CONTRACTS_V9,
    },
};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

const WEBHOOK_SECRET: &[u8] = b"receiver-process-e2e-fixture-secret";
const OWNER: &str = "forge-admin";
const REPOSITORY: &str = "agent-forge-core";

fn assert_reconcile_schema_version(value: &Value) {
    let is_batch =
        value.get("results").is_some() || value.get("requested_delivery_count").is_some();
    // Schema 16/20 added the current pull-request draft flag; 17/21 adds
    // current issue/PR assignees; 18/22 adds current work-item milestones;
    // 19/23 adds the bounded current-release projection.
    let expected = if is_batch { 23 } else { 19 };
    assert_eq!(value["schema_version"], expected);
    assert_eq!(
        value["source_event_provenance_contract"],
        json!({
            "receiver_hmac_check": "performed_before_append_by_current_ingest_path",
            "per_delivery_ingestion_attestation_persisted": "optional_ed25519_receiver_signature",
            "receiver_attestation_verification": "requires_external_trusted_key_policy; required_before_credential_for_in_scope_delivery",
            "raw_signature_persisted": false,
            "replay_hmac_verification": "not_performed",
            "journal_replay_contract": "validates_hash_chain_and_envelope; optional_receiver_attestation_requires_external_trust_policy",
            "per_delivery_attestation_observation": "journal_source.receiver_attestation.status reports only receiver signature verification under the supplied external key policy",
            "per_delivery_journal_identity": ["journal_source.sequence", "journal_source.event_sha256"],
            "batch_error_source_observation": "selected_event_sources binds selected delivery IDs to journal rows for structured batch refusals after source selection, including host credential-pipe read failures, when per-delivery results are omitted",
            "signature_verification_does_not_establish": ["upstream_truth", "software_correctness", "local_writer_authenticity"],
            "journal_hash_chain_authenticates_local_writer": false,
            "source_authenticity_claim_for_selected_delivery": "not_made",
            "trusted_boundary": "local_filesystem_writer",
            "current_state_read_means": "canonical_api_read_only"
        })
    );
}

#[test]
fn reconcile_process_contract_assertion_rejects_incomplete_provenance() {
    let incomplete = json!({
        "schema_version": 19,
        "source_event_provenance_contract": {
            "replay_hmac_verification": "not_performed",
            "source_authenticity_claim_for_selected_delivery": "not_made"
        }
    });
    assert!(
        std::panic::catch_unwind(|| assert_reconcile_schema_version(&incomplete)).is_err(),
        "process contract assertion must reject a response that omits the other provenance limits"
    );
}

const ISSUE_NUMBER: u64 = 41;
const RECEIVER_READINESS_PROBE: &[u8] =
    b"GET /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n";

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "webhook-receiver-process-e2e-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create private test directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("restrict test directory permissions");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Receiver(Child);

impl Drop for Receiver {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[cfg(target_os = "linux")]
struct ProcessGroupGuard(libc::pid_t);

#[cfg(target_os = "linux")]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        // The direct-parent cancellation regression deliberately demonstrates
        // what happens to descendants. Keep the test hermetic even on red.
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}

#[cfg(target_os = "linux")]
fn wait_until_process_exited_or_zombie(pid: libc::pid_t, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        };
        let Some((_, fields)) = stat.rsplit_once(") ") else {
            return false;
        };
        if fields.starts_with('Z') {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn write_secret(path: &std::path::Path) {
    use std::fs::OpenOptions;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create fixture secret with owner-only mode");
    file.write_all(WEBHOOK_SECRET)
        .expect("write fixture secret");
    file.sync_all().expect("flush fixture secret");
}

fn write_synthetic_webhook_batch(
    path: &std::path::Path,
    deliveries: &[(&str, &str, Value)],
) -> Vec<String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create synthetic legacy webhook inbox");
    let mut previous_hash = "0".repeat(64);
    let mut hashes = Vec::with_capacity(deliveries.len());
    for (index, (delivery_id, event_name, event_payload)) in deliveries.iter().enumerate() {
        let mut row = json!({
            "sequence": index + 1,
            "event_id": format!("fixture-event-{}", index + 1),
            "idempotency_key": format!("forgejo-delivery-v1:{delivery_id}"),
            "kind": format!("forgejo.webhook.v1.{event_name}"),
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "delivery_id": delivery_id,
                "event": event_name,
                "payload": event_payload,
            },
            "previous_hash": previous_hash,
        });
        let hash = sha256(&serde_json::to_vec(&row).expect("serialize synthetic hash material"));
        row["hash"] = Value::String(hash.clone());
        let mut line = serde_json::to_vec(&row).expect("serialize synthetic webhook journal row");
        line.push(b'\n');
        file.write_all(&line).expect("write synthetic journal row");
        previous_hash = hash.clone();
        hashes.push(hash);
    }
    file.sync_all().expect("sync synthetic webhook inbox");
    hashes
}

fn write_signing_seed(path: &std::path::Path, seed: &[u8; 32]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create receiver signing seed with owner-only mode");
    file.write_all(seed).expect("write receiver signing seed");
    file.sync_all().expect("flush receiver signing seed");
}

fn start_ready_attested_receiver(
    secret: &std::path::Path,
    journal: &std::path::Path,
) -> (Receiver, SocketAddr, SigningKey) {
    let seed = [0x71; 32];
    let signing_seed_path = journal.with_file_name("receiver-signing.seed");
    write_signing_seed(&signing_seed_path, &seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let (receiver, address) =
        start_ready_receiver_with_attestation_key(secret, journal, &signing_seed_path);
    (receiver, address, signing_key)
}

fn trusted_attestation_key_args(signing_key: &SigningKey) -> [String; 2] {
    [
        "--trusted-webhook-attestation-public-key-hex".to_owned(),
        hex::encode(signing_key.verifying_key().as_bytes()),
    ]
}

fn request(delivery_id: &str, body: &[u8]) -> Vec<u8> {
    request_event(delivery_id, "issues", body)
}

fn request_event(delivery_id: &str, event: &str, body: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(WEBHOOK_SECRET).expect("valid fixture key");
    mac.update(body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let headers = format!(
        "POST /forgejo/webhook HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Content-Length: {}\r\n\
         Content-Type: application/json\r\n\
         X-Forgejo-Delivery: {delivery_id}\r\n\
         X-Forgejo-Event: {event}\r\n\
         X-Forgejo-Signature: {signature}\r\n",
        body.len()
    );
    let mut bytes = format!("{headers}\r\n").into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn send(address: SocketAddr, request: &[u8]) -> (u16, Value) {
    send_with_timeout(address, request, Duration::from_secs(3))
}

fn send_with_timeout(address: SocketAddr, request: &[u8], read_timeout: Duration) -> (u16, Value) {
    let (status, _, body) = send_with_headers_and_timeout(address, request, read_timeout);
    (status, body)
}

fn send_with_headers_and_timeout(
    address: SocketAddr,
    request: &[u8],
    read_timeout: Duration,
) -> (u16, String, Value) {
    let mut stream = TcpStream::connect(address).expect("connect to receiver subprocess");
    stream
        .set_read_timeout(Some(read_timeout))
        .expect("set bounded client timeout");
    stream
        .write_all(request)
        .expect("send complete signed webhook");
    stream
        .shutdown(Shutdown::Write)
        .expect("half close request body");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read complete receiver response");
    let response = String::from_utf8(response).expect("HTTP response is UTF-8");
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .expect("HTTP response has header terminator");
    let status = headers
        .split_whitespace()
        .nth(1)
        .expect("HTTP status exists")
        .parse()
        .expect("HTTP status is numeric");
    (
        status,
        headers.to_owned(),
        serde_json::from_str(body).expect("response body is JSON"),
    )
}

fn collect_http_response(mut stream: TcpStream) -> (u16, String, Value) {
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read complete HTTP response");
    let response = String::from_utf8(response).expect("HTTP response is UTF-8");
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .expect("HTTP response has header terminator");
    let status = headers
        .split_whitespace()
        .nth(1)
        .expect("HTTP status exists")
        .parse()
        .expect("HTTP status is numeric");
    (
        status,
        headers.to_owned(),
        serde_json::from_str(body).expect("response body is JSON"),
    )
}

fn response_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().skip(1).find_map(|line| {
        let (header_name, value) = line.split_once(':')?;
        header_name
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

fn start_receiver(
    address: SocketAddr,
    secret: &std::path::Path,
    journal: &std::path::Path,
) -> Receiver {
    Receiver(
        Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
            .env_clear()
            .arg(address.to_string())
            .arg(secret)
            .arg(journal)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the actual webhook receiver binary"),
    )
}

fn start_receiver_with_attestation_key(
    address: SocketAddr,
    secret: &std::path::Path,
    journal: &std::path::Path,
    signing_seed: &std::path::Path,
) -> Receiver {
    Receiver(
        Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
            .env_clear()
            .arg(address.to_string())
            .arg(secret)
            .arg(journal)
            .arg(signing_seed)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start receiver with host-held attestation seed"),
    )
}

fn start_receiver_with_umask(
    address: SocketAddr,
    secret: &std::path::Path,
    journal: &std::path::Path,
    mask: libc::mode_t,
) -> Receiver {
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"));
    command
        .env_clear()
        .arg(address.to_string())
        .arg(secret)
        .arg(journal)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            libc::umask(mask);
            Ok(())
        });
    }
    Receiver(command.spawn().expect("start receiver with explicit umask"))
}

#[cfg(feature = "test-crash-injection")]
fn start_receiver_with_crash_after_append(
    address: SocketAddr,
    secret: &std::path::Path,
    journal: &std::path::Path,
    delivery_id: &str,
) -> Receiver {
    Receiver(
        Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
            .env_clear()
            .env("AGENT_FORGE_TEST_CRASH_AFTER_WEBHOOK_APPEND", delivery_id)
            .arg(address.to_string())
            .arg(secret)
            .arg(journal)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start crash-injected webhook receiver"),
    )
}

fn receiver_readiness_probe(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .is_err()
        || stream.write_all(RECEIVER_READINESS_PROBE).is_err()
        || stream.shutdown(Shutdown::Write).is_err()
    {
        return false;
    }

    let mut response = Vec::new();
    if stream.take(8 * 1024).read_to_end(&mut response).is_err() {
        return false;
    }
    let Ok(response) = std::str::from_utf8(&response) else {
        return false;
    };
    let Some((headers, body)) = response.split_once("\r\n\r\n") else {
        return false;
    };
    let status = headers
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok());
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    status == Some(405) && value["error"]["code"] == "method_not_allowed"
}

fn receiver_is_ready(receiver: &mut Receiver, address: SocketAddr) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if receiver
            .0
            .try_wait()
            .expect("check receiver startup")
            .is_some()
        {
            return false;
        }
        if receiver_readiness_probe(address) {
            return receiver
                .0
                .try_wait()
                .expect("check receiver after readiness probe")
                .is_none();
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn start_ready_receiver(
    secret: &std::path::Path,
    journal: &std::path::Path,
) -> (Receiver, SocketAddr) {
    for _ in 0..5 {
        let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = reservation.local_addr().expect("read reserved address");
        drop(reservation);

        let mut receiver = start_receiver(address, secret, journal);
        if receiver_is_ready(&mut receiver, address) {
            return (receiver, address);
        }
        drop(receiver);
    }
    panic!("receiver did not bind and pass its HTTP readiness probe after five port attempts");
}

fn start_ready_receiver_with_attestation_key(
    secret: &std::path::Path,
    journal: &std::path::Path,
    signing_seed: &std::path::Path,
) -> (Receiver, SocketAddr) {
    for _ in 0..5 {
        let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = reservation.local_addr().expect("read reserved address");
        drop(reservation);

        let mut receiver =
            start_receiver_with_attestation_key(address, secret, journal, signing_seed);
        if receiver_is_ready(&mut receiver, address) {
            return (receiver, address);
        }
        drop(receiver);
    }
    panic!(
        "attested receiver did not bind and pass its HTTP readiness probe after five port attempts"
    );
}

fn start_ready_receiver_with_umask(
    secret: &std::path::Path,
    journal: &std::path::Path,
    mask: libc::mode_t,
) -> (Receiver, SocketAddr) {
    for _ in 0..5 {
        let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = reservation.local_addr().expect("read reserved address");
        drop(reservation);

        let mut receiver = start_receiver_with_umask(address, secret, journal, mask);
        if receiver_is_ready(&mut receiver, address) {
            return (receiver, address);
        }
        drop(receiver);
    }
    panic!("receiver did not bind and pass its HTTP readiness probe after five port attempts");
}

#[cfg(feature = "test-crash-injection")]
fn start_ready_receiver_with_crash_after_append(
    secret: &std::path::Path,
    journal: &std::path::Path,
    delivery_id: &str,
) -> (Receiver, SocketAddr) {
    for _ in 0..5 {
        let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = reservation.local_addr().expect("read reserved address");
        drop(reservation);

        let mut receiver =
            start_receiver_with_crash_after_append(address, secret, journal, delivery_id);
        if receiver_is_ready(&mut receiver, address) {
            return (receiver, address);
        }
        drop(receiver);
    }
    panic!("crash-injected receiver did not bind and pass its readiness probe");
}

fn stop_receiver(receiver: &mut Receiver) -> ExitStatus {
    let result = unsafe { libc::kill(receiver.0.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(result, 0, "send graceful shutdown to receiver");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = receiver.0.try_wait().expect("wait for receiver shutdown") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = receiver.0.kill();
            let _ = receiver.0.wait();
            panic!("receiver did not shut down within five seconds");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

struct FixtureCurrentState {
    response: Value,
    origin: String,
    reads: Mutex<Vec<ResourceRef>>,
}

impl FixtureCurrentState {
    fn new(address: SocketAddr) -> Self {
        let origin = format!("http://{address}");
        Self {
            response: json!({
                "number": ISSUE_NUMBER,
                "state": "open",
                "updated_at": "2026-10-22T12:30:00Z",
                "html_url": format!("{origin}/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
            }),
            origin,
            reads: Mutex::new(Vec::new()),
        }
    }
}

impl ForgejoCurrentState for FixtureCurrentState {
    fn read_current(&self, resource: &ResourceRef) -> Result<Value, ApiReadError> {
        self.reads.lock().unwrap().push(resource.clone());
        Ok(self.response.clone())
    }

    fn expected_html_url(&self, resource: &ResourceRef) -> String {
        let (collection, identity) = match (&resource.kind, &resource.id) {
            (ResourceKind::Issue, ResourceId::Number(number)) => ("issues", number.to_string()),
            (ResourceKind::PullRequest, ResourceId::Number(number)) => {
                ("pulls", number.to_string())
            }
            (ResourceKind::Branch, ResourceId::Branch(branch)) => {
                ("src/branch", branch.replace('/', "%2F"))
            }
            (ResourceKind::WorkflowRun, ResourceId::WorkflowRun { index_in_repo, .. }) => {
                ("actions/runs", index_in_repo.to_string())
            }
            _ => return String::new(),
        };
        format!(
            "{}/{}/{}/{collection}/{identity}",
            self.origin, resource.owner, resource.repository
        )
    }
}

#[test]
fn actual_receiver_process_attests_an_accepted_webhook_with_its_host_seed() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let signing_seed_path = scratch.path().join("receiver-signing.seed");
    let journal_path = scratch.path().join("inbox.jsonl");
    let seed = [41u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    write_secret(&secret);
    write_signing_seed(&signing_seed_path, &seed);

    let (mut receiver, address) =
        start_ready_receiver_with_attestation_key(&secret, &journal_path, &signing_seed_path);
    let delivery_id = "receiver-attested-delivery-41";
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize local webhook fixture");
    let (status, response) = send(address, &request(delivery_id, &body));

    assert_eq!(status, 202);
    assert_eq!(response["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal = EventJournal::open_webhook_inbox(&journal_path);
    let event = read_journaled_webhook_event_verified(
        &journal,
        delivery_id,
        &[signing_key.verifying_key()],
    )
    .expect("the process persisted a signature verifiable without its private seed");
    assert_eq!(event.event().sequence, 1);
    assert_eq!(event.event().payload["delivery_id"], delivery_id);
    assert!(event.event().receiver_attestation.is_some());
}

#[cfg(feature = "test-crash-injection")]
#[test]
fn abrupt_receiver_crash_after_durable_append_before_ack_replays_idempotently() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let delivery_id = "crash-after-append-41";
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed webhook fixture");
    let signed_request = request(delivery_id, &body);
    let (mut receiver, address) =
        start_ready_receiver_with_crash_after_append(&secret, &journal_path, delivery_id);

    let mut stream = TcpStream::connect(address).expect("connect to crash-injected receiver");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("bound response observation");
    stream
        .write_all(&signed_request)
        .expect("deliver complete signed webhook");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish webhook request");
    let mut response = Vec::new();
    let response_read = stream.read_to_end(&mut response);

    assert!(
        response.is_empty(),
        "receiver must crash after durable append and before any HTTP acknowledgement; read={response_read:?} response={:?}",
        String::from_utf8_lossy(&response)
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let crashed = loop {
        if let Some(status) = receiver.0.try_wait().expect("observe injected crash") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "receiver did not crash at the injection point"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(crashed.signal(), Some(libc::SIGKILL));

    let bytes_after_crash =
        fs::read(&journal_path).expect("durable journal survives process crash");
    let journal_hash_after_crash = sha256(&bytes_after_crash);
    let journal = EventJournal::open(&journal_path);
    let recovered = journal.replay().expect("replay appended event after crash");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].payload["delivery_id"], delivery_id);

    let (mut restarted, restarted_address) = start_ready_receiver(&secret, &journal_path);
    let (status, duplicate) = send(restarted_address, &signed_request);
    assert_eq!(status, 202);
    assert_eq!(duplicate["accepted"], true);
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(duplicate["sequence"], recovered[0].sequence);
    assert!(stop_receiver(&mut restarted).success());

    let bytes_after_replay = fs::read(&journal_path).expect("read replayed journal");
    assert_eq!(sha256(&bytes_after_replay), journal_hash_after_crash);
    let resumed_inbox = journal.replay().expect("read recovered inbox after replay");
    assert_eq!(resumed_inbox.len(), 1);
    let fixture = FixtureCurrentState::new(restarted_address);
    let reconciled = reconcile_inbox_events(&resumed_inbox, &fixture)
        .expect("reconcile the recovered delivery against current state");
    assert_eq!(reconciled.len(), 1);
    let state = reconciled.into_iter().next().unwrap().unwrap();
    assert_eq!(state.delivery_id, delivery_id);
    assert_eq!(state.state, "open");
}

#[test]
fn actual_receiver_repairs_a_torn_webhook_tail_before_redelivery_and_reconcile() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue webhook");
    let original_request = request("delivery-tail-seed-41", &body);
    let retried_request = request("delivery-tail-retry-42", &body);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, first) = send(address, &original_request);
    assert_eq!(status, 202);
    assert_eq!(first["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());

    let complete_journal = fs::read(&journal_path).expect("read complete first journal event");
    assert!(complete_journal.ends_with(b"\n"));
    let mut interrupted_append = OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .expect("open journal to model a torn final append");
    interrupted_append
        .write_all(
            b"{\"sequence\":2,\"event_id\":\"evt-2\",\"idempotency_key\":\"forgejo-delivery-v1:delivery-tail-retry-42\",\"kind\":\"forgejo.webhook.issues\",\"payload\":",
        )
        .expect("write an incomplete final record prefix");
    interrupted_append
        .sync_all()
        .expect("persist the synthetic interrupted prefix");
    drop(interrupted_append);
    let torn_journal = fs::read(&journal_path).expect("read interrupted journal image");
    assert!(torn_journal.starts_with(&complete_journal));
    assert!(!torn_journal.ends_with(b"\n"));

    let (mut recovered_receiver, recovered_address) = start_ready_receiver(&secret, &journal_path);
    assert_eq!(
        fs::read(&journal_path).expect("read repaired journal"),
        complete_journal,
        "startup removes only the incomplete unterminated tail"
    );
    let (status, retried) = send(recovered_address, &retried_request);
    assert_eq!(status, 202);
    assert_eq!(retried["accepted"], true);
    assert_eq!(retried["duplicate"], false);
    assert_eq!(retried["sequence"], 2);
    let (status, duplicate) = send(recovered_address, &retried_request);
    assert_eq!(status, 202);
    assert_eq!(duplicate["duplicate"], true);
    assert!(stop_receiver(&mut recovered_receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay recovered webhook inbox");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["delivery_id"], "delivery-tail-seed-41");
    assert_eq!(events[1].payload["delivery_id"], "delivery-tail-retry-42");
    let fixture = FixtureCurrentState::new(recovered_address);
    let states = reconcile_inbox_events(&events, &fixture)
        .expect("reconcile both deliveries using canonical fixture state")
        .into_iter()
        .map(|state| state.expect("current issue fixture resolves each delivery"))
        .collect::<Vec<_>>();
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].delivery_id, "delivery-tail-seed-41");
    assert_eq!(states[1].delivery_id, "delivery-tail-retry-42");
}

#[test]
fn actual_receiver_process_accepts_retries_recovers_and_reconciles() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize synthetic Forgejo webhook");
    let signed_request = request("delivery-process-41", &body);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let mut bad_signature_request = signed_request.clone();
    let signature_start = bad_signature_request
        .windows(b"X-Forgejo-Signature: ".len())
        .position(|window| window == b"X-Forgejo-Signature: ")
        .expect("request contains signature header")
        + b"X-Forgejo-Signature: ".len();
    bad_signature_request[signature_start] = if bad_signature_request[signature_start] == b'0' {
        b'1'
    } else {
        b'0'
    };
    let (status, refused) = send(address, &bad_signature_request);
    assert_eq!(status, 401);
    assert_eq!(refused["error"]["code"], "invalid_signature");

    let (status, accepted) = send(address, &signed_request);
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1, "refused request did not append");
    assert_eq!(accepted["reconcile_required"], true);

    let (status, duplicate) = send(address, &signed_request);
    assert_eq!(status, 202);
    assert_eq!(duplicate["duplicate"], true);

    let conflicting_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize conflicting synthetic Forgejo webhook");
    let conflicting_request = request("delivery-process-41", &conflicting_body);
    let (status, conflict) = send(address, &conflicting_request);
    assert_eq!(status, 409);
    assert_eq!(conflict["error"]["code"], "delivery_conflict");
    assert_eq!(conflict["error"]["retryable"], false);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let inbox = journal
        .replay()
        .expect("verify recovered append-only journal");
    assert_eq!(
        inbox.len(),
        1,
        "retry is persisted once; conflict is not appended"
    );
    assert_eq!(inbox[0].payload["payload"]["issue"]["state"], "closed");

    let fixture = FixtureCurrentState::new(address);
    let reconciled = reconcile_inbox_events(&inbox, &fixture)
        .expect("reconcile one recovered delivery against current-state fixture");
    let state: ReconciledState = reconciled
        .into_iter()
        .next()
        .expect("one delivery result")
        .expect("fixture has canonical issue state");
    assert_eq!(
        state.state, "open",
        "current state supersedes stale payload"
    );
    assert_eq!(state.resource.kind, ResourceKind::Issue);
    assert_eq!(state.delivery_id, "delivery-process-41");
    assert_eq!(state.html_url, fixture.response["html_url"]);
    assert_eq!(fixture.reads.lock().unwrap().len(), 1);

    let (mut recovered_receiver, recovered_address) = start_ready_receiver(&secret, &journal_path);
    let (status, after_restart) = send(recovered_address, &signed_request);
    assert_eq!(status, 202);
    assert_eq!(after_restart["duplicate"], true);
    assert!(stop_receiver(&mut recovered_receiver).success());
    assert_eq!(journal.replay().unwrap().len(), 1);
}

#[test]
fn actual_receiver_recovers_after_all_request_workers_timeout() {
    // Keep this in sync with WORKERS in forgejo-webhook-local.rs: the partial
    // requests must occupy every request worker before the valid delivery queues.
    const REQUEST_WORKERS: usize = 4;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let mut partial_clients = Vec::with_capacity(REQUEST_WORKERS);
    for _ in 0..REQUEST_WORKERS {
        let mut client = TcpStream::connect(address).expect("connect partial webhook client");
        client
            .set_read_timeout(Some(Duration::from_secs(7)))
            .expect("bound partial-client response wait");
        client
            .write_all(
                b"POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 128\r\n\r\npartial",
            )
            .expect("send incomplete request body without closing the connection");
        partial_clients.push(client);
    }
    // Allow the listener to dispatch all four requests before submitting the
    // complete event, which then waits in the bounded accepted-connection queue.
    thread::sleep(Duration::from_millis(250));

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize complete signed webhook after slow clients");
    let valid_request = request("delivery-after-incomplete-workers-41", &body);
    let submitted_at = Instant::now();
    let (status, accepted) = send_with_timeout(address, &valid_request, Duration::from_secs(7));
    let elapsed = submitted_at.elapsed();

    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(
        elapsed >= Duration::from_secs(2),
        "valid delivery bypassed the four held worker slots after {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(6),
        "valid delivery did not resume after bounded request timeouts: {elapsed:?}"
    );

    for mut client in partial_clients {
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .expect("read timeout response from incomplete request");
        let response = String::from_utf8(response).expect("timeout response is UTF-8");
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .expect("timeout response has header terminator");
        assert!(
            headers.starts_with("HTTP/1.1 408 "),
            "received {response:?}"
        );
        let body: Value = serde_json::from_str(body).expect("timeout error is JSON");
        assert_eq!(body["error"]["code"], "request_timeout");
        assert_eq!(body["error"]["retryable"], true);
    }

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay_webhook_inbox()
        .expect("replay the receiver inbox after the timeout burst");
    assert_eq!(
        events.len(),
        1,
        "incomplete requests never reached the journal"
    );
    assert_eq!(
        events[0].payload["delivery_id"],
        "delivery-after-incomplete-workers-41"
    );
}

#[test]
fn actual_receiver_refuses_when_connection_queue_is_full_then_accepts_exact_retry_once() {
    // Keep these values aligned with WORKERS, MAX_CONNECTIONS and
    // REJECTION_QUEUE_CAPACITY in forgejo-webhook-local.rs.
    const REQUEST_WORKERS: usize = 4;
    const QUEUED_CONNECTIONS: usize = 16;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let mut partial_clients = Vec::with_capacity(REQUEST_WORKERS);
    for _ in 0..REQUEST_WORKERS {
        let mut client = TcpStream::connect(address).expect("connect partial webhook client");
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("bound partial-client response wait");
        client
            .write_all(
                b"POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 128\r\n\r\npartial",
            )
            .expect("send incomplete request body without closing the connection");
        partial_clients.push(client);
    }
    thread::sleep(Duration::from_millis(250));

    let mut queued_clients = Vec::with_capacity(QUEUED_CONNECTIONS);
    for _ in 0..QUEUED_CONNECTIONS {
        let mut client = TcpStream::connect(address).expect("connect queued readiness probe");
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("bound queued-probe response wait");
        client
            .write_all(RECEIVER_READINESS_PROBE)
            .expect("send a complete read-only request into the pending queue");
        client
            .shutdown(Shutdown::Write)
            .expect("finish the queued readiness probe");
        queued_clients.push(client);
    }
    thread::sleep(Duration::from_millis(250));

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize the signed delivery to retry after overload");
    let exact_retry = request("delivery-retry-after-server-busy-41", &body);
    let inbox_before_rejection = fs::read(&journal_path).ok();

    let (status, headers, overloaded) =
        send_with_headers_and_timeout(address, &exact_retry, Duration::from_secs(3));
    assert_eq!(status, 503);
    assert_eq!(response_header(&headers, "Retry-After"), Some("5"));
    assert_eq!(overloaded["error"]["code"], "server_busy");
    assert_eq!(overloaded["error"]["retryable"], true);
    assert_eq!(fs::read(&journal_path).ok(), inbox_before_rejection);

    for client in partial_clients {
        let (status, _, response) = collect_http_response(client);
        assert_eq!(status, 408);
        assert_eq!(response["error"]["code"], "request_timeout");
        assert_eq!(response["error"]["retryable"], true);
    }
    for client in queued_clients {
        let (status, _, response) = collect_http_response(client);
        assert_eq!(status, 405);
        assert_eq!(response["error"]["code"], "method_not_allowed");
    }

    let (status, accepted) = send_with_timeout(address, &exact_retry, Duration::from_secs(3));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    let (status, duplicate) = send_with_timeout(address, &exact_retry, Duration::from_secs(3));
    assert_eq!(status, 202);
    assert_eq!(duplicate["duplicate"], true);

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay_webhook_inbox()
        .expect("replay the inbox after overload and retry");
    assert_eq!(events.len(), 1, "the rejected attempt was never journaled");
    assert_eq!(
        events[0].payload["delivery_id"],
        "delivery-retry-after-server-busy-41"
    );
}

#[test]
fn actual_receiver_reports_retryable_journal_busy_and_accepts_same_delivery_after_release() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    let lock_path = journal_path.with_extension("jsonl.lock");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize the signed webhook fixture");
    let signed_request = request("delivery-journal-busy-41", &body);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let lock_file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&lock_path)
        .expect("open the receiver's initialized stable inbox lock file");
    assert_eq!(
        unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "hold the inbox lock from an independent file descriptor"
    );
    let inbox_before = fs::read(&journal_path).ok();

    let (status, busy_headers, busy) =
        send_with_headers_and_timeout(address, &signed_request, Duration::from_secs(8));
    assert_eq!(status, 503);
    assert_eq!(response_header(&busy_headers, "Retry-After"), Some("5"));
    assert_eq!(busy["error"]["code"], "journal_busy");
    assert_eq!(busy["error"]["retryable"], true);
    assert!(
        busy["error"]["action"]
            .as_str()
            .unwrap()
            .contains("retry the exact signed delivery")
    );
    assert_eq!(fs::read(&journal_path).ok(), inbox_before);

    drop(lock_file);
    let (status, accepted_headers, accepted) =
        send_with_headers_and_timeout(address, &signed_request, Duration::from_secs(3));
    assert_eq!(status, 202);
    assert_eq!(response_header(&accepted_headers, "Retry-After"), None);
    assert_eq!(accepted["duplicate"], false);
    let (status, duplicate_headers, duplicate) =
        send_with_headers_and_timeout(address, &signed_request, Duration::from_secs(3));
    assert_eq!(status, 202);
    assert_eq!(response_header(&duplicate_headers, "Retry-After"), None);
    assert_eq!(duplicate["duplicate"], true);
    assert!(stop_receiver(&mut receiver).success());

    let inbox = EventJournal::open(&journal_path)
        .replay_webhook_inbox()
        .expect("replay the inbox after releasing the lock");
    assert_eq!(inbox.len(), 1, "the busy attempt was not appended");
    assert_eq!(
        inbox[0].idempotency_key,
        "forgejo-delivery-v1:delivery-journal-busy-41"
    );
}

#[test]
fn actual_receiver_rejects_duplicate_signature_headers_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let valid = String::from_utf8(request("duplicate-signature-41", &body))
        .expect("valid request template is UTF-8");
    let signature_header = valid
        .split("\r\n")
        .find(|line| line.starts_with("X-Forgejo-Signature:"))
        .expect("request has a signature header");
    let needle = format!("{signature_header}\r\n");
    let replacement = format!("{signature_header}\r\n{signature_header}\r\n");
    let duplicated = valid.replacen(&needle, &replacement, 1);
    assert_ne!(duplicated, valid, "fixture duplicates the signature header");

    let (status, rejected) = send(address, duplicated.as_bytes());

    assert_eq!(status, 401);
    assert_eq!(rejected["error"]["code"], "signature_missing");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        fs::read(&journal_path).ok(),
        journal_before,
        "an ambiguous signature must not append a signed event"
    );
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after rejected duplicate signature");
    assert!(
        events.is_empty(),
        "refused signature must not poison the inbox"
    );
}

#[test]
fn actual_receiver_accepts_gitea_legacy_header_aliases() {
    const DELIVERY_ID: &str = "gitea-legacy-alias-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let forgejo_headers =
        String::from_utf8(request(DELIVERY_ID, &body)).expect("request template is UTF-8");
    let gitea_only_headers = forgejo_headers
        .replace("X-Forgejo-Delivery:", "X-Gitea-Delivery:")
        .replace("X-Forgejo-Event:", "X-Gitea-Event:")
        .replace("X-Forgejo-Signature:", "X-Gitea-Signature:");
    assert!(!gitea_only_headers.contains("X-Forgejo-"));
    assert!(gitea_only_headers.contains("X-Gitea-Delivery:"));
    assert!(gitea_only_headers.contains("X-Gitea-Event:"));
    assert!(gitea_only_headers.contains("X-Gitea-Signature:"));

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, gitea_only_headers.as_bytes());
    assert_eq!(status, 202, "Gitea aliases should be accepted: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox accepted through Gitea aliases");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(events[0].payload["delivery_id"], DELIVERY_ID);
}

#[test]
#[ignore = "run via scripts/run_webhook_reconcile_cli_e2e.py; fixture reserves the fixed loopback API endpoint"]
fn actual_reconcile_cli_process_reads_gitea_alias_only_delivery() {
    const DELIVERY_ID: &str = "gitea-alias-cli-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize stale signed issue event");

    let forgejo_headers = String::from_utf8(request(DELIVERY_ID, &body))
        .expect("signed Forgejo request template is UTF-8");
    let gitea_only_headers = forgejo_headers
        .replace("X-Forgejo-Delivery:", "X-Gitea-Delivery:")
        .replace("X-Forgejo-Event:", "X-Gitea-Event:")
        .replace("X-Forgejo-Signature:", "X-Gitea-Signature:");
    assert!(!gitea_only_headers.contains("X-Forgejo-"));

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, gitea_only_headers.as_bytes());
    assert_eq!(
        status, 202,
        "Gitea-only aliases should be accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());

    let inbox = EventJournal::open(&journal_path)
        .replay()
        .expect("replay accepted Gitea-only delivery");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(inbox[0].payload["delivery_id"], DELIVERY_ID);
    let journal_before = fs::read(&journal_path).expect("snapshot accepted inbox");

    let current_url = "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": current_url
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("issue API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let result: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "current_state_read");
    assert_eq!(result["delivery_id"], DELIVERY_ID);
    assert_eq!(result["resource"]["kind"], "issue");
    assert_eq!(result["resource"]["number"], ISSUE_NUMBER);
    assert_eq!(result["canonical_state"]["state"], "open");
    assert_eq!(result["canonical_state"]["html_url"], current_url);
    assert_eq!(result["upstream_read_attempts"], 1);
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after CLI reconciliation"),
        journal_before,
        "readback is non-mutating"
    );
}

#[test]
#[ignore = "run via scripts/run_webhook_reconcile_cli_e2e.py; fixture reserves the fixed loopback API endpoint"]
fn actual_reconcile_cli_process_reads_mixed_forgejo_gitea_aliases() {
    const DELIVERY_ID: &str = "mixed-alias-cli-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize stale signed issue event");

    let forgejo_headers = String::from_utf8(request(DELIVERY_ID, &body))
        .expect("signed Forgejo request template is UTF-8");
    let mixed_alias_headers = forgejo_headers
        .replace("X-Forgejo-Delivery:", "X-Gitea-Delivery:")
        .replace("X-Forgejo-Signature:", "X-Gitea-Signature:");
    assert!(mixed_alias_headers.contains("X-Gitea-Delivery:"));
    assert!(mixed_alias_headers.contains("X-Forgejo-Event: issues"));
    assert!(mixed_alias_headers.contains("X-Gitea-Signature:"));
    assert!(!mixed_alias_headers.contains("X-Gitea-Event:"));

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, mixed_alias_headers.as_bytes());
    assert_eq!(
        status, 202,
        "one alias per field across both header families should be accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());

    let inbox = EventJournal::open(&journal_path)
        .replay()
        .expect("replay accepted mixed-alias delivery");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(inbox[0].payload["delivery_id"], DELIVERY_ID);
    let journal_before = fs::read(&journal_path).expect("snapshot accepted inbox");

    let current_url = "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": current_url
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual reconciliation CLI process");
    let requests = fixture.join().expect("issue API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let result: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns JSON");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "current_state_read");
    assert_eq!(result["delivery_id"], DELIVERY_ID);
    assert_eq!(result["resource"]["kind"], "issue");
    assert_eq!(result["resource"]["number"], ISSUE_NUMBER);
    assert_eq!(result["canonical_state"]["state"], "open");
    assert_eq!(result["canonical_state"]["html_url"], current_url);
    assert_eq!(result["upstream_read_attempts"], 1);
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after CLI reconciliation"),
        journal_before,
        "readback is non-mutating"
    );
}

#[test]
fn actual_receiver_rejects_duplicate_event_headers_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let delivery_id = "duplicate-event-header-41";
    let valid = String::from_utf8(request_event(delivery_id, "issues", &body))
        .expect("valid request template is UTF-8");
    let event_header = "X-Forgejo-Event: issues\r\n";
    let duplicated = valid.replacen(event_header, &format!("{event_header}{event_header}"), 1);
    assert_ne!(duplicated, valid, "fixture duplicates the event header");
    let case_variant_duplicate = valid.replacen(
        event_header,
        &format!("{event_header}x-fOrGeJo-eVeNt: pull_request\r\n"),
        1,
    );
    assert_ne!(case_variant_duplicate, valid);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    for (case, malformed) in [
        ("same-case duplicate", duplicated.as_str()),
        (
            "case-variant conflicting duplicate",
            case_variant_duplicate.as_str(),
        ),
    ] {
        let (status, rejected) = send(address, malformed.as_bytes());
        assert_eq!(status, 400, "{case}: {rejected}");
        assert_eq!(rejected["error"]["code"], "event_header_invalid");
        assert_eq!(rejected["error"]["retryable"], false);
        assert_eq!(
            fs::read(&journal_path).ok(),
            journal_before,
            "{case} must not append or reserve the delivery ID"
        );
    }

    let (status, accepted) = send(address, &request_event(delivery_id, "issues", &body));
    assert_eq!(
        status, 202,
        "corrected single header should be accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after corrected event-header retry");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(events[0].payload["delivery_id"], delivery_id);
}

#[test]
fn actual_receiver_rejects_duplicate_host_fields_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let journal = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&journal_path)
        .expect("create empty inbox before receiver startup");
    journal.sync_all().expect("sync empty inbox");
    drop(journal);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).expect("read inbox before request");
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let valid = String::from_utf8(request("duplicate-host-41", &body))
        .expect("valid request template is UTF-8");
    let duplicated = valid.replacen(
        "Host: 127.0.0.1\r\n",
        "Host: 127.0.0.1\r\nHost: conflicting.example\r\n",
        1,
    );
    assert_ne!(duplicated, valid, "fixture adds a second Host field");

    let (status, rejected) = send(address, duplicated.as_bytes());

    assert_eq!(status, 400);
    assert_eq!(rejected["error"]["code"], "request_malformed");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after rejection"),
        journal_before,
        "ambiguous Host fields must not append a signed event"
    );
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after rejected duplicate Host fields");
    assert!(
        events.is_empty(),
        "rejected request must not enter the inbox"
    );
}

#[test]
fn actual_receiver_rejects_case_variant_conflicting_content_lengths_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let delivery_id = "case-variant-content-length-41";
    let valid = String::from_utf8(request_event(delivery_id, "issues", &body))
        .expect("signed valid request is UTF-8");
    let content_length = format!("Content-Length: {}\r\n", body.len());
    let ambiguous = valid.replacen(
        &content_length,
        &format!(
            "{content_length}cOnTeNt-LeNgTh: {}\r\n",
            body.len().saturating_add(1)
        ),
        1,
    );
    assert_ne!(
        ambiguous, valid,
        "fixture adds a conflicting field name variant"
    );

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let (status, rejected) = send(address, ambiguous.as_bytes());
    assert_eq!(
        status, 400,
        "ambiguous framing must be rejected: {rejected}"
    );
    assert_eq!(rejected["error"]["code"], "request_malformed");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        rejected["error"]["action"],
        "send a complete, well-formed request with at most one Content-Length, no Transfer-Encoding, and one Host for HTTP/1.1"
    );
    assert_eq!(
        fs::read(&journal_path).ok(),
        journal_before,
        "ambiguous Content-Length fields must not append or reserve the delivery"
    );

    let (status, accepted) = send(address, &request_event(delivery_id, "issues", &body));
    assert_eq!(
        status, 202,
        "corrected framing should accept the same ID: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after corrected Content-Length retry");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(events[0].payload["delivery_id"], delivery_id);
}

#[test]
fn actual_receiver_rejects_content_length_with_transfer_encoding_before_corrected_retry() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let delivery_id = "content-length-transfer-encoding-41";
    let valid = String::from_utf8(request_event(delivery_id, "issues", &body))
        .expect("signed valid request is UTF-8");
    let ambiguous = valid.replacen(
        "Content-Type: application/json\r\n",
        "Transfer-Encoding: chunked\r\nContent-Type: application/json\r\n",
        1,
    );
    assert_ne!(
        ambiguous, valid,
        "fixture adds Transfer-Encoding alongside Content-Length"
    );

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let (status, rejected) = send(address, ambiguous.as_bytes());
    assert_eq!(
        status, 400,
        "Content-Length with Transfer-Encoding must be rejected: {rejected}"
    );
    assert_eq!(rejected["error"]["code"], "request_malformed");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        rejected["error"]["action"],
        "send a complete, well-formed request with at most one Content-Length, no Transfer-Encoding, and one Host for HTTP/1.1"
    );
    assert_eq!(
        fs::read(&journal_path).ok(),
        journal_before,
        "ambiguous framing must not append or reserve the delivery"
    );

    let (status, accepted) = send(address, &request_event(delivery_id, "issues", &body));
    assert_eq!(
        status, 202,
        "corrected framing should accept the same delivery ID: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after corrected framing retry");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(events[0].payload["delivery_id"], delivery_id);
}

#[test]
fn actual_receiver_binds_repository_identity_fields_before_durable_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let journal = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&journal_path)
        .expect("create empty inbox before receiver startup");
    journal.sync_all().expect("sync empty inbox");
    drop(journal);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let consistent_payload = json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {
            "full_name": format!("{OWNER}/{REPOSITORY}"),
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        }
    });
    let valid_body = serde_json::to_vec(&consistent_payload).expect("serialize valid event");
    let (status, accepted) = send(
        address,
        &request("repository-identity-valid-41", &valid_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);

    let journal_after_valid = fs::read(&journal_path).expect("read journal after valid event");
    let conflicting_payloads = [
        json!({
            "action": "opened",
            "issue": {"number": ISSUE_NUMBER, "state": "open"},
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": "other-repository",
                "owner": {"login": OWNER}
            }
        }),
        json!({
            "action": "opened",
            "issue": {"number": ISSUE_NUMBER, "state": "open"},
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": "other-owner"}
            }
        }),
    ];

    for (index, payload) in conflicting_payloads.iter().enumerate() {
        let delivery_id = format!("repository-identity-conflict-{index}");
        let body = serde_json::to_vec(payload).expect("serialize conflicting event");
        let (status, rejected) = send(address, &request(&delivery_id, &body));

        assert_eq!(status, 400, "delivery {delivery_id}");
        assert_eq!(rejected["error"]["code"], "event_payload_invalid");
        assert_eq!(rejected["error"]["retryable"], false);
        assert_eq!(
            fs::read(&journal_path).expect("read inbox after conflicting event"),
            journal_after_valid,
            "contradictory identity fields must not append delivery {delivery_id}"
        );
    }

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after identity rejections");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].idempotency_key,
        "forgejo-delivery-v1:repository-identity-valid-41"
    );
}

#[test]
fn actual_receiver_rejects_zero_sha_branch_create_before_durable_append() {
    const DELIVERY_ID: &str = "branch-create-zero-sha-refusal";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let journal = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&journal_path)
        .expect("create empty inbox before receiver startup");
    journal.sync_all().expect("sync empty inbox");
    drop(journal);
    let empty_inbox = fs::read(&journal_path).expect("snapshot empty inbox");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let body = serde_json::to_vec(&json!({
        "ref": "feature/zero-object",
        "ref_type": "branch",
        "sha": "0000000000000000000000000000000000000000",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize create event with the null object ID");

    let (status, rejected) = send(address, &request_event(DELIVERY_ID, "create", &body));

    assert_eq!(status, 400, "zero-SHA create must be rejected: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(rejected["error"]["details"]["field_path"], "payload.sha");
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after rejection"),
        empty_inbox,
        "a rejected create event must not change the durable inbox"
    );
    assert!(stop_receiver(&mut receiver).success());
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after rejection")
            .is_empty()
    );
}

#[test]
fn actual_receiver_localizes_missing_branch_ref_type_before_durable_append() {
    const DELIVERY_ID: &str = "branch-create-missing-ref-type-refusal";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let journal = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&journal_path)
        .expect("create empty inbox before receiver startup");
    journal.sync_all().expect("sync empty inbox");
    drop(journal);
    let empty_inbox = fs::read(&journal_path).expect("snapshot empty inbox");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let body = serde_json::to_vec(&json!({
        "ref": "feature/missing-kind",
        "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize create event without ref_type");

    let (status, rejected) = send(address, &request_event(DELIVERY_ID, "create", &body));

    assert_eq!(
        status, 400,
        "create without ref_type must be rejected: {rejected}"
    );
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        rejected["error"]["details"]["field_path"],
        "payload.ref_type"
    );
    assert_eq!(
        rejected["error"]["action"],
        "set payload.ref_type to branch for branch create/delete events; tag refs are not accepted by this adapter; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after rejection"),
        empty_inbox,
        "a create event without ref_type must not change the durable inbox"
    );
    assert!(stop_receiver(&mut receiver).success());
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after rejection")
            .is_empty()
    );
}

#[test]
fn actual_receiver_binds_pull_request_base_repository_but_allows_fork_head() {
    const PRIVATE_MARKER: &str = "foreign-pr-base-private-marker";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let journal = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&journal_path)
        .expect("create empty inbox before receiver startup");
    journal.sync_all().expect("sync empty inbox");
    drop(journal);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let matching_base_with_fork_head = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {
            "full_name": format!("{OWNER}/{REPOSITORY}"),
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        },
        "pull_request": {
            "number": 42,
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            },
            "head": {
                "ref": "feature/forge-proof",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            }
        }
    }))
    .expect("serialize pull request with matching base and fork head");
    let (status, accepted) = send(
        address,
        &request_event(
            "pr-base-repository-valid-42",
            "pull_request",
            &matching_base_with_fork_head,
        ),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    let journal_after_valid = fs::read(&journal_path).expect("read valid PR inbox entry");

    let mut mismatched_base_payload = json!({
        "action": "synchronized",
        "repository": {
            "full_name": format!("{OWNER}/{REPOSITORY}"),
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        },
        "pull_request": {
            "number": 42,
            "title": PRIVATE_MARKER,
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {
                    "full_name": "other-owner/other-repository",
                    "name": "other-repository",
                    "owner": {"login": "other-owner"}
                }
            },
            "head": {
                "ref": "feature/forge-proof",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            }
        }
    });
    let mismatched_base_repository = serde_json::to_vec(&mismatched_base_payload)
        .expect("serialize pull request with conflicting base repository");
    let (status, rejected) = send(
        address,
        &request_event(
            "pr-base-repository-conflict-42",
            "pull_request",
            &mismatched_base_repository,
        ),
    );
    assert_eq!(status, 400);
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        rejected["error"]["details"]["field_path"],
        "payload.pull_request.base.repo"
    );
    assert_eq!(
        rejected["error"]["action"],
        "ask the trusted sender to provide a signed payload whose pull_request.base.repo identity matches repository.full_name; head.repo may remain a contributor fork; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(
        !rejected.to_string().contains(PRIVATE_MARKER),
        "the rejected private PR title must not be copied into the error response"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after base repository rejection"),
        journal_after_valid,
        "a PR whose base repository conflicts with the delivery repository must not append"
    );

    mismatched_base_payload["pull_request"]["base"]["repo"] = json!({
        "full_name": format!("{OWNER}/{REPOSITORY}"),
        "name": REPOSITORY,
        "owner": {"login": OWNER}
    });
    let corrected_base_repository =
        serde_json::to_vec(&mismatched_base_payload).expect("serialize corrected PR base");
    let (status, corrected) = send(
        address,
        &request_event(
            "pr-base-repository-conflict-42",
            "pull_request",
            &corrected_base_repository,
        ),
    );
    assert_eq!(
        status, 202,
        "corrected same-ID PR should be accepted: {corrected}"
    );
    assert_eq!(corrected["accepted"], true);
    assert_eq!(corrected["duplicate"], false);
    assert_eq!(corrected["sequence"], 2);

    let journal_after_corrected = fs::read(&journal_path).expect("read inbox after corrected PR");
    let mut malformed_base_payload = mismatched_base_payload.clone();
    let corrected_base = malformed_base_payload["pull_request"]["base"].clone();
    malformed_base_payload["pull_request"]["base"] = json!("not-an-object");
    let malformed_base_bytes =
        serde_json::to_vec(&malformed_base_payload).expect("serialize malformed PR base");
    let (status, malformed_base) = send(
        address,
        &request_event("pr-base-envelope-42", "pull_request", &malformed_base_bytes),
    );
    assert_eq!(status, 400);
    assert_eq!(malformed_base["error"]["code"], "event_payload_invalid");
    assert_eq!(
        malformed_base["error"]["details"]["field_path"],
        "payload.pull_request.base"
    );
    assert_eq!(
        malformed_base["error"]["action"],
        "when supplied, set payload.pull_request.base to a JSON object; its repo identity must match repository.full_name while head.repo may remain a contributor fork; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(!malformed_base.to_string().contains(PRIVATE_MARKER));
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after malformed PR base"),
        journal_after_corrected,
        "a malformed pull_request.base must not append"
    );

    malformed_base_payload["pull_request"]["base"] = corrected_base;
    let corrected_base_bytes =
        serde_json::to_vec(&malformed_base_payload).expect("serialize corrected PR base");
    let (status, corrected_base) = send(
        address,
        &request_event("pr-base-envelope-42", "pull_request", &corrected_base_bytes),
    );
    assert_eq!(
        status, 202,
        "corrected base retry should be accepted: {corrected_base}"
    );
    assert_eq!(corrected_base["sequence"], 3);

    let journal_after_corrected_base =
        fs::read(&journal_path).expect("read inbox after corrected base retry");
    let mut malformed_base_repository_payload = mismatched_base_payload.clone();
    malformed_base_repository_payload["pull_request"]["base"]["repo"]["full_name"] = json!(42);
    let malformed_base_repository_bytes = serde_json::to_vec(&malformed_base_repository_payload)
        .expect("serialize malformed PR base repository");
    let (status, malformed_base_repository) = send(
        address,
        &request_event(
            "pr-base-repository-malformed-42",
            "pull_request",
            &malformed_base_repository_bytes,
        ),
    );
    assert_eq!(status, 400);
    assert_eq!(
        malformed_base_repository["error"]["details"]["field_path"],
        "payload.pull_request.base.repo"
    );
    assert_eq!(
        malformed_base_repository["error"]["action"],
        "ask the trusted sender to provide a signed payload whose pull_request.base.repo identity matches repository.full_name; head.repo may remain a contributor fork; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(
        !malformed_base_repository
            .to_string()
            .contains(PRIVATE_MARKER)
    );
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after malformed base repository"),
        journal_after_corrected_base,
        "a malformed base repo identity must not append"
    );

    malformed_base_repository_payload["pull_request"]["base"]["repo"] =
        mismatched_base_payload["pull_request"]["base"]["repo"].clone();
    let corrected_base_repository_bytes = serde_json::to_vec(&malformed_base_repository_payload)
        .expect("serialize corrected PR base repository");
    let (status, corrected_base_repository) = send(
        address,
        &request_event(
            "pr-base-repository-malformed-42",
            "pull_request",
            &corrected_base_repository_bytes,
        ),
    );
    assert_eq!(status, 202);
    assert_eq!(corrected_base_repository["sequence"], 4);

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after rejected base repository");
    assert_eq!(events.len(), 4);
    assert_eq!(
        events[0].idempotency_key,
        "forgejo-delivery-v1:pr-base-repository-valid-42"
    );
    assert_eq!(
        events[1].idempotency_key,
        "forgejo-delivery-v1:pr-base-repository-conflict-42"
    );
    assert_eq!(
        events[1].payload["payload"]["pull_request"]["base"]["repo"]["full_name"],
        format!("{OWNER}/{REPOSITORY}")
    );
    assert_eq!(
        events[2].idempotency_key,
        "forgejo-delivery-v1:pr-base-envelope-42"
    );
    assert_eq!(
        events[3].idempotency_key,
        "forgejo-delivery-v1:pr-base-repository-malformed-42"
    );
}

#[test]
fn actual_receiver_rejects_ambiguous_message_framing_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");

    let valid_length_request = String::from_utf8(request("duplicate-length-41", &body))
        .expect("valid request template is UTF-8");
    let content_length = format!("Content-Length: {}\r\n", body.len());
    let duplicate_content_length = valid_length_request.replacen(
        &content_length,
        &format!("{content_length}Content-Length: 0\r\n"),
        1,
    );

    let valid_chunked_request = String::from_utf8(request("chunked-with-length-41", &body))
        .expect("valid request template is UTF-8");
    let transfer_encoding = valid_chunked_request.replacen(
        "Content-Type: application/json\r\n",
        "Transfer-Encoding: chunked\r\nContent-Type: application/json\r\n",
        1,
    );

    for ambiguous_request in [duplicate_content_length, transfer_encoding] {
        let (status, rejected) = send(address, ambiguous_request.as_bytes());

        assert_eq!(status, 400);
        assert_eq!(rejected["error"]["code"], "request_malformed");
        assert_eq!(rejected["error"]["retryable"], false);
        assert_eq!(
            fs::read(&journal_path).ok(),
            journal_before,
            "ambiguous message framing must not append a signed event"
        );
    }

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after rejected ambiguous framing");
    assert!(
        events.is_empty(),
        "rejected framing must not poison the inbox"
    );
}

#[test]
fn actual_receiver_rejects_oversized_content_length_without_waiting_for_the_body() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let request = format!(
        "POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Forgejo-Delivery: oversized-body-41\r\nX-Forgejo-Event: issues\r\n\r\n",
        MAX_WEBHOOK_BODY_BYTES + 1
    );

    // `send` half-closes after writing only the headers. A 413 proves the
    // process rejected the declared size before waiting for body bytes or
    // reaching webhook signature verification.
    let (status, response) = send(address, request.as_bytes());

    assert_eq!(status, 413);
    assert_eq!(response["error"]["code"], "body_too_large");
    assert_eq!(response["error"]["retryable"], false);
    assert_eq!(
        fs::read(&journal_path).ok(),
        journal_before,
        "an oversized declaration must not create an inbox record"
    );
    assert!(stop_receiver(&mut receiver).success());
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after oversized body refusal")
            .is_empty()
    );
}

#[test]
fn actual_receiver_rejects_a_signed_body_with_a_mismatched_event_header_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    // Forgejo's signature covers the body. The claimed event header is checked
    // against that body before it can create a durable inbox record.
    let mismatched = request_event("event-kind-confusion-41", "pull_request", &issue_body);
    let (status, response) = send(address, &mismatched);
    assert_eq!(status, 400);
    assert_eq!(response["error"]["code"], "event_payload_invalid");
    assert_eq!(response["error"]["retryable"], false);
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox before corrected retry")
            .is_empty(),
        "the mismatched event header must not append or reserve the delivery ID"
    );

    let (status, accepted) = send(
        address,
        &request_event("event-kind-confusion-41", "issues", &issue_body),
    );
    assert_eq!(
        status, 202,
        "corrected retry should be accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after corrected event-kind retry");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(events[0].payload["delivery_id"], "event-kind-confusion-41");

    let fixture = FixtureCurrentState::new(address);
    let reconciled = reconcile_inbox_events(&events, &fixture)
        .expect("reconcile the corrected retry through the issue adapter");
    let state: ReconciledState = reconciled
        .into_iter()
        .next()
        .expect("one corrected delivery result")
        .expect("issue fixture resolves the corrected delivery");
    assert_eq!(state.delivery_id, "event-kind-confusion-41");
    assert_eq!(state.resource.kind, ResourceKind::Issue);
    assert_eq!(state.resource.id, ResourceId::Number(ISSUE_NUMBER));
    assert_eq!(state.state, "open");
    assert_eq!(state.html_url, fixture.response["html_url"]);
    assert_eq!(fixture.reads.lock().unwrap().len(), 1);
}

#[test]
fn actual_receiver_refuses_unknown_or_incompatible_workflow_actions_without_journaling() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let repository = json!({
        "full_name": format!("{OWNER}/{REPOSITORY}"),
        "name": REPOSITORY,
        "owner": {"login": OWNER}
    });
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let cases = [
        (
            "workflow-run-unknown-action-41",
            "workflow_run_success",
            json!({
                "action": "future_workflow_action",
                "run": {
                    "id": 7001,
                    "index_in_repo": 41,
                    "status": "success",
                    "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "updated": "2026-10-26T11:00:00Z",
                    "html_url": run_url.clone(),
                    "repository": repository.clone()
                }
            }),
        ),
        (
            "workflow-job-unknown-action-41",
            "workflow_job_success",
            json!({
                "action": "future_workflow_action",
                "job": {
                    "id": 9001,
                    "run_id": 7001,
                    "status": "success",
                    "html_url": format!("{run_url}/jobs/1/attempt/1")
                },
                "run": {
                    "id": 7001,
                    "index_in_repo": 41,
                    "status": "running",
                    "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "updated": "2026-10-26T11:00:00Z",
                    "html_url": run_url.clone()
                },
                "repository": repository.clone()
            }),
        ),
        (
            "workflow-run-incompatible-action-41",
            "workflow_run_blocked",
            json!({
                "action": "run_completed",
                "run": {
                    "id": 7001,
                    "index_in_repo": 41,
                    "status": "blocked",
                    "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "updated": "2026-10-26T11:00:00Z",
                    "html_url": run_url.clone(),
                    "repository": repository.clone()
                }
            }),
        ),
        (
            "workflow-job-incompatible-action-41",
            "workflow_job_waiting",
            json!({
                "action": "job_completed",
                "job": {
                    "id": 9001,
                    "run_id": 7001,
                    "status": "waiting",
                    "html_url": format!("{run_url}/jobs/1/attempt/1")
                },
                "run": {
                    "id": 7001,
                    "index_in_repo": 41,
                    "status": "running",
                    "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "updated": "2026-10-26T11:00:00Z",
                    "html_url": run_url.clone()
                },
                "repository": repository.clone()
            }),
        ),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    for (delivery_id, event, payload) in cases {
        let body = serde_json::to_vec(&payload).expect("serialize signed workflow payload");
        let (status, response) = send(address, &request_event(delivery_id, event, &body));
        assert_eq!(
            status, 400,
            "unknown or incompatible action must be rejected: {response}"
        );
        assert_eq!(response["accepted"], false);
        assert_eq!(response["error"]["code"], "event_payload_invalid");
        assert_eq!(response["error"]["retryable"], false);
        assert_eq!(response["error"]["details"]["field_path"], "payload.action");
        assert!(
            response["error"]["action"]
                .as_str()
                .is_some_and(|action| action.contains("event contract")),
            "refusal should direct the sender to the action contract: {response}"
        );
        assert_eq!(
            fs::read(&journal_path).ok(),
            journal_before,
            "a rejected workflow action must not append or mutate the inbox"
        );
    }
    assert!(stop_receiver(&mut receiver).success());
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay journal after unknown-action refusals")
            .is_empty()
    );
}

#[test]
fn actual_receiver_refuses_workflow_run_invalid_commit_sha_with_actionable_guidance_before_reserving_delivery()
 {
    const DELIVERY_ID: &str = "workflow-run-invalid-commit-sha-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let valid_payload = json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-28T15:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41"),
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let mut invalid_payload = valid_payload.clone();
    invalid_payload["run"]["commit_sha"] = json!("not-a-commit-id");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let invalid_body = serde_json::to_vec(&invalid_payload).expect("serialize invalid run");
    let (status, refusal) = send(
        address,
        &request_event(DELIVERY_ID, "workflow_run_success", &invalid_body),
    );
    assert_eq!(status, 400);
    assert_eq!(refusal["error"]["code"], "event_payload_invalid");
    assert_eq!(refusal["error"]["retryable"], false);
    assert_eq!(
        refusal["error"]["details"]["field_path"],
        "payload.run.commit_sha"
    );
    let action = refusal["error"]["action"]
        .as_str()
        .expect("invalid commit SHA has repair guidance");
    assert!(action.contains("payload.run.commit_sha"), "{action}");
    assert!(
        action.contains("40- or 64-character hexadecimal"),
        "{action}"
    );
    assert!(action.contains("same delivery ID"), "{action}");
    assert!(action.contains("not journaled"), "{action}");
    assert_eq!(fs::read(&journal_path).ok(), journal_before);
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay after invalid run SHA")
            .is_empty()
    );

    let valid_body = serde_json::to_vec(&valid_payload).expect("serialize corrected run");
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "workflow_run_success", &valid_body),
    );
    assert_eq!(status, 202, "same-ID corrected retry: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected workflow-run delivery");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["delivery_id"], DELIVERY_ID);
    assert_eq!(
        events[0].payload["payload"]["run"]["commit_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}

#[test]
fn actual_receiver_guides_workflow_run_field_repairs_before_retry() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let valid_payload = json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-28T15:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41"),
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let cases = [
        (
            "workflow-run-index-in-repo-41",
            "run.index_in_repo",
            "index_in_repo",
            json!(0),
            "positive JSON integer",
        ),
        (
            "workflow-run-invalid-updated-41",
            "run.updated",
            "updated",
            json!("2026-02-30T11:00:00Z"),
            "RFC3339 timestamp",
        ),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, field, mutation_field, invalid_value, expected_guidance) in cases {
        let count_before = EventJournal::open(&journal_path)
            .replay()
            .expect("replay before malformed workflow-run delivery")
            .len();
        let journal_before = fs::read(&journal_path).ok();
        let mut invalid_payload = valid_payload.clone();
        invalid_payload["run"][mutation_field] = invalid_value;
        let invalid_body =
            serde_json::to_vec(&invalid_payload).expect("serialize malformed workflow-run field");

        let (status, refusal) = send(
            address,
            &request_event(delivery_id, "workflow_run_success", &invalid_body),
        );
        assert_eq!(status, 400);
        assert_eq!(refusal["error"]["code"], "event_payload_invalid");
        assert_eq!(refusal["error"]["retryable"], false);
        assert_eq!(
            refusal["error"]["details"]["field_path"],
            format!("payload.{field}")
        );
        let action = refusal["error"]["action"]
            .as_str()
            .expect("malformed workflow-run field has repair guidance");
        assert!(action.contains(&format!("payload.{field}")), "{action}");
        assert!(action.contains(expected_guidance), "{action}");
        assert!(action.contains("same delivery ID"), "{action}");
        assert!(action.contains("not journaled"), "{action}");
        assert_eq!(fs::read(&journal_path).ok(), journal_before);
        assert_eq!(
            EventJournal::open(&journal_path)
                .replay()
                .expect("replay after malformed workflow-run field")
                .len(),
            count_before
        );

        let valid_body =
            serde_json::to_vec(&valid_payload).expect("serialize corrected workflow-run delivery");
        let (status, accepted) = send(
            address,
            &request_event(delivery_id, "workflow_run_success", &valid_body),
        );
        assert_eq!(status, 202, "same-ID corrected retry: {accepted}");
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], count_before + 1);
    }
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected workflow-run deliveries");
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].payload["delivery_id"],
        "workflow-run-index-in-repo-41"
    );
    assert_eq!(
        events[1].payload["delivery_id"],
        "workflow-run-invalid-updated-41"
    );
}

#[test]
#[ignore = "run via scripts/run_webhook_reconcile_cli_e2e.py; fixture reserves the fixed loopback API endpoint"]
fn actual_reconcile_cli_process_reads_corrected_header_retry_from_receiver_journal() {
    const DELIVERY_ID: &str = "event-kind-confusion-cli-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize stale signed issue event");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, rejected) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 400);
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after header mismatch")
            .is_empty(),
        "mismatched unsigned header must not reserve the delivery ID"
    );

    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issues", &body));
    assert_eq!(
        status, 202,
        "corrected delivery should be accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let inbox = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox before CLI reconciliation");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(inbox[0].payload["delivery_id"], DELIVERY_ID);
    let journal_before = fs::read(&journal_path).expect("snapshot accepted inbox");

    let current_url = "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": current_url
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("issue API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let result: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "current_state_read");
    assert_eq!(result["delivery_id"], DELIVERY_ID);
    assert_eq!(result["resource"]["kind"], "issue");
    assert_eq!(result["resource"]["number"], ISSUE_NUMBER);
    assert_eq!(result["canonical_state"]["state"], "open");
    assert_eq!(result["canonical_state"]["html_url"], current_url);
    assert_eq!(result["upstream_read_attempts"], 1);
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after CLI reconciliation"),
        journal_before,
        "readback is non-mutating"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_rejects_missing_and_malformed_workflow_repository_full_name_with_nested_field_path()
 {
    const DELIVERY_ID: &str = "workflow-run-repository-identity-retry-001";
    const PRIVATE_MARKER: &str = "workflow-run-repository-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let mut payload = json!({
        "action": "run_completed",
        "run": {
            "id": 7002,
            "index_in_repo": 42,
            "status": "success",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-28T15:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/42"),
            "repository": {
                "name": REPOSITORY,
                "owner": {"login": OWNER},
                "description": PRIVATE_MARKER
            }
        }
    });
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    for full_name in [
        None,
        Some("missing-slash"),
        Some("other-owner/other-repository"),
    ] {
        if let Some(full_name) = full_name {
            payload["run"]["repository"]["full_name"] = json!(full_name);
        } else {
            payload["run"]["repository"]
                .as_object_mut()
                .expect("run.repository is an object")
                .remove("full_name");
        }

        let invalid_body =
            serde_json::to_vec(&payload).expect("serialize invalid workflow repository identity");
        let (status, rejection) = send(
            address,
            &request_event(DELIVERY_ID, "workflow_run_success", &invalid_body),
        );
        assert_eq!(status, 400);
        assert_eq!(rejection["accepted"], false);
        assert_eq!(rejection["error"]["code"], "event_payload_invalid");
        assert_eq!(rejection["error"]["retryable"], false);
        assert_eq!(
            rejection["error"]["details"]["field_path"],
            "payload.run.repository.full_name"
        );
        assert_eq!(
            rejection["error"]["action"],
            "ask the trusted sender to provide a signed payload with run.repository.full_name set to a valid owner/repository pair consistent with run.repository.name and run.repository.owner.login; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
        );
        assert!(
            !rejection.to_string().contains(PRIVATE_MARKER),
            "invalid workflow content must not be copied into the error response"
        );
        assert!(
            EventJournal::open(&journal_path)
                .replay()
                .expect("replay after invalid workflow repository identity")
                .is_empty(),
            "invalid repository identity must not reserve its delivery ID"
        );
    }

    payload["run"]["repository"]["full_name"] = json!(format!("{OWNER}/{REPOSITORY}"));
    let corrected_body =
        serde_json::to_vec(&payload).expect("serialize corrected workflow repository identity");
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "workflow_run_success", &corrected_body),
    );
    assert_eq!(
        status, 202,
        "corrected same-ID workflow event should be admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected workflow event");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["delivery_id"], DELIVERY_ID);
    assert_eq!(
        events[0].payload["payload"]["run"]["repository"]["full_name"],
        format!("{OWNER}/{REPOSITORY}")
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_refuses_workflow_job_status_header_substitution_before_append() {
    const DELIVERY_ID: &str = "workflow-job-status-confusion-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let job_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "html_url": format!(
                "https://forgejo.invalid/{OWNER}/{REPOSITORY}/actions/runs/41/jobs/0/attempt/1"
            )
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
        }
    }))
    .expect("serialize signed workflow-job failure notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    // Forgejo signs the body but not X-Forgejo-Event. A substituted success
    // suffix must be rejected before it can reserve the delivery ID.
    let (status, rejection) = send(
        address,
        &request_event(DELIVERY_ID, "workflow_job_success", &job_body),
    );
    assert_eq!(status, 400);
    assert_eq!(rejection["error"]["code"], "event_payload_invalid");
    assert_eq!(rejection["error"]["retryable"], false);
    assert_eq!(
        rejection["error"]["details"]["field_path"],
        "payload.job.status"
    );
    let action = rejection["error"]["action"]
        .as_str()
        .expect("mismatched workflow-job status has repair guidance");
    assert!(action.contains("payload.job.status"), "{action}");
    assert!(action.contains("X-Forgejo-Event"), "{action}");
    assert!(action.contains("workflow_job"), "{action}");
    assert!(action.contains("trusted sender"), "{action}");
    assert!(action.contains("GET /forgejo/capabilities"), "{action}");
    assert!(action.contains("same delivery ID"), "{action}");
    assert!(action.contains("original signed body"), "{action}");
    assert!(action.contains("do not alter the signed body"), "{action}");
    assert!(action.contains("not journaled"), "{action}");
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay after substituted workflow-job status")
            .is_empty(),
        "the mismatched unsigned header must not append or reserve a delivery"
    );

    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "workflow_job_failure", &job_body),
    );
    assert_eq!(
        status, 202,
        "correcting the header should permit this delivery: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected workflow-job delivery");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.workflow_job_failure");
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_requires_reviewed_action_before_reserving_delivery() {
    const DELIVERY_ID: &str = "review-action-required-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let mut invalid_payload = json!({
        "pull_request": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "review": {"type": "pull_request_review_approved"}
    });
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    // The valid HMAC covers this body, while the missing action violates the
    // reviewed notification contract. Refusal must happen before the durable
    // inbox reserves the delivery ID.
    let (status, rejection) = send(
        address,
        &request_event(
            DELIVERY_ID,
            "pull_request_approved",
            &serde_json::to_vec(&invalid_payload).expect("serialize missing-action review"),
        ),
    );
    assert_eq!(status, 400);
    assert_eq!(rejection["error"]["code"], "event_payload_invalid");
    assert_eq!(
        rejection["error"]["details"]["field_path"],
        "payload.action"
    );
    assert_eq!(rejection["error"]["retryable"], false);
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay after missing-action refusal")
            .is_empty(),
        "invalid signed review must not reserve its delivery ID"
    );

    invalid_payload["action"] = json!("reviewed");
    let (status, accepted) = send(
        address,
        &request_event(
            DELIVERY_ID,
            "pull_request_approved",
            &serde_json::to_vec(&invalid_payload).expect("serialize corrected review"),
        ),
    );
    assert_eq!(
        status, 202,
        "corrected notification should be accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected review notification");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.pull_request_approved");
}

#[test]
fn actual_receiver_rejects_invalid_issue_actions_before_append_and_accepts_forgejo_v17_actions() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let issue_fields = json!({
        "issue": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    });
    let invalid_payloads = [
        ("issues-missing-action-retry", issue_fields.clone()),
        ("issues-non-string-action", {
            let mut payload = issue_fields.clone();
            payload["action"] = json!(17);
            payload
        }),
        ("issues-action-from-other-event", {
            let mut payload = issue_fields.clone();
            payload["action"] = json!("synchronized");
            payload
        }),
    ];

    for (delivery_id, payload) in invalid_payloads {
        let (status, rejected) = send(
            address,
            &request_event(
                delivery_id,
                "issues",
                &serde_json::to_vec(&payload).expect("serialize signed invalid issue event"),
            ),
        );
        assert_eq!(
            status, 400,
            "invalid action must fail at the receiver process"
        );
        assert_eq!(rejected["error"]["code"], "event_payload_invalid");
        assert_eq!(rejected["error"]["details"]["field_path"], "payload.action");
        assert_eq!(rejected["error"]["retryable"], false);
        assert!(
            EventJournal::open(&journal_path)
                .replay()
                .expect("replay inbox after invalid issue action")
                .is_empty(),
            "invalid signed issue delivery must not reserve its delivery ID"
        );
    }

    let actions = [
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
    ];
    for (index, action) in actions.into_iter().enumerate() {
        let delivery_id = if action == "opened" {
            "issues-missing-action-retry"
        } else {
            match action {
                "closed" => "issues-closed-action",
                "reopened" => "issues-reopened-action",
                "edited" => "issues-edited-action",
                "assigned" => "issues-assigned-action",
                "unassigned" => "issues-unassigned-action",
                "label_updated" => "issues-label-updated-action",
                "label_cleared" => "issues-label-cleared-action",
                "milestoned" => "issues-milestoned-action",
                "demilestoned" => "issues-demilestoned-action",
                _ => unreachable!("all supported issue actions have unique delivery IDs"),
            }
        };
        let mut payload = issue_fields.clone();
        payload["action"] = json!(action);
        let (status, accepted) = send(
            address,
            &request_event(
                delivery_id,
                "issues",
                &serde_json::to_vec(&payload).expect("serialize supported issue action"),
            ),
        );
        assert_eq!(
            status, 202,
            "Forgejo v17 `issues` action {action}: {accepted}"
        );
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], index as u64 + 1);
    }

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay supported issue actions after receiver shutdown");
    assert_eq!(events.len(), actions.len());
    assert_eq!(
        events
            .iter()
            .map(|event| event.payload["payload"]["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        actions
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_rejects_missing_and_malformed_release_repository_identity_then_accepts_same_delivery()
 {
    const DELIVERY_ID: &str = "release-repository-identity-corrected-retry-001";
    const PRIVATE_MARKER: &str = "release-repository-identity-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let mut payload = json!({
        "action": "published",
        "release": {
            "id": 617,
            "tag_name": "v1.4.6",
            "body": PRIVATE_MARKER
        },
        "repository": {
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        }
    });
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    for full_name in [
        None,
        Some("missing-slash"),
        Some("other-owner/other-repository"),
    ] {
        if let Some(full_name) = full_name {
            payload["repository"]["full_name"] = json!(full_name);
        } else {
            payload["repository"]
                .as_object_mut()
                .expect("repository is an object")
                .remove("full_name");
        }

        let invalid_body =
            serde_json::to_vec(&payload).expect("serialize invalid repository identity");
        let (status, rejection) = send(
            address,
            &request_event(DELIVERY_ID, "release", &invalid_body),
        );
        assert_eq!(status, 400);
        assert_eq!(rejection["accepted"], false);
        assert_eq!(rejection["error"]["code"], "event_payload_invalid");
        assert_eq!(rejection["error"]["retryable"], false);
        assert_eq!(
            rejection["error"]["details"]["field_path"],
            "payload.repository.full_name"
        );
        assert_eq!(
            rejection["error"]["action"],
            "ask the trusted sender to provide a signed payload with repository.full_name set to a valid owner/repository pair consistent with repository.name and repository.owner.login; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
        );
        assert!(
            !rejection.to_string().contains(PRIVATE_MARKER),
            "invalid release content must not be copied into the error response"
        );
        assert!(
            EventJournal::open(&journal_path)
                .replay()
                .expect("replay after invalid repository identity")
                .is_empty(),
            "invalid repository identity must not reserve its delivery ID"
        );
    }

    payload["repository"]["full_name"] = json!(format!("{OWNER}/{REPOSITORY}"));
    let corrected_body =
        serde_json::to_vec(&payload).expect("serialize corrected release repository identity");
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "release", &corrected_body),
    );
    assert_eq!(
        status, 202,
        "corrected same-ID release event should be admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected release delivery");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["delivery_id"], DELIVERY_ID);
    assert_eq!(
        events[0].payload["payload"]["repository"]["full_name"],
        format!("{OWNER}/{REPOSITORY}")
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_rejects_zero_release_id_then_accepts_same_id_retry() {
    const DELIVERY_ID: &str = "release-id-zero-corrected-retry-71";
    const PRIVATE_MARKER: &str = "release-id-zero-private-body-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let mut payload = json!({
        "action": "published",
        "release": {"id": 0, "tag_name": "v1.4.2", "body": PRIVATE_MARKER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    });
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let invalid_body = serde_json::to_vec(&payload)
        .expect("serialize signed release event with a zero resource ID");
    let (status, rejection) = send(
        address,
        &request_event(DELIVERY_ID, "release", &invalid_body),
    );
    assert_eq!(status, 400);
    assert_eq!(rejection["accepted"], false);
    assert_eq!(rejection["error"]["code"], "event_payload_invalid");
    assert_eq!(rejection["error"]["retryable"], false);
    assert_eq!(
        rejection["error"]["details"]["field_path"],
        "payload.release.id"
    );
    assert_eq!(
        rejection["error"]["action"],
        "set payload.release.id to a positive JSON integer; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(
        !rejection.to_string().contains(PRIVATE_MARKER),
        "invalid release content must not be copied into the error response"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay after rejecting a zero Release ID")
            .is_empty(),
        "invalid Release ID must not reserve its delivery ID"
    );

    payload["release"]["id"] = json!(616);
    let corrected_body = serde_json::to_vec(&payload)
        .expect("serialize corrected release event with the same delivery ID");
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "release", &corrected_body),
    );
    assert_eq!(
        status, 202,
        "corrected same-ID release event should be admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected release delivery");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["delivery_id"], DELIVERY_ID);
    assert_eq!(events[0].payload["payload"]["release"]["id"], 616);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_rejects_invalid_release_action_then_accepts_corrected_same_delivery() {
    const DELIVERY_ID: &str = "release-action-invalid-retry-61";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let mut payload = json!({
        "action": "created",
        "release": {"id": 610, "tag_name": "v1.4.2"},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    });
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let invalid_body = serde_json::to_vec(&payload)
        .expect("serialize signed release event with unsupported action");
    let (status, rejection) = send(
        address,
        &request_event(DELIVERY_ID, "release", &invalid_body),
    );
    assert_eq!(status, 400);
    assert_eq!(rejection["error"]["code"], "event_payload_invalid");
    assert_eq!(rejection["error"]["retryable"], false);
    assert_eq!(
        rejection["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(
        rejection["error"]["action"]
            .as_str()
            .is_some_and(|action| action.contains("event contract")),
        "refusal should direct the sender to the release action contract: {rejection}"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay after unsupported release action")
            .is_empty(),
        "unsupported release action must not reserve its delivery ID"
    );

    let mut accepted_actions = Vec::new();
    for (sequence, action) in ["published", "updated", "deleted"].into_iter().enumerate() {
        let delivery_id = match action {
            "published" => DELIVERY_ID,
            "updated" => "release-action-updated-62",
            "deleted" => "release-action-deleted-63",
            _ => unreachable!("the fixture lists only reviewed release actions"),
        };
        payload["action"] = json!(action);
        let body = serde_json::to_vec(&payload).expect("serialize supported release action");
        let (status, accepted) = send(address, &request_event(delivery_id, "release", &body));
        assert_eq!(
            status, 202,
            "corrected/supported release action {action} should be accepted: {accepted}"
        );
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence as u64 + 1);
        accepted_actions.push(action);
    }

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay accepted release notifications after receiver shutdown");
    assert_eq!(events.len(), accepted_actions.len());
    assert!(
        events
            .iter()
            .all(|event| event.kind == "forgejo.webhook.v1.release")
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.payload["payload"]["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        accepted_actions
    );
}

#[test]
fn actual_receiver_rejects_conflicting_issue_root_number_before_append_and_accepts_corrected_retry()
{
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let delivery_id = "issues-root-number-corrected-retry";
    let mut payload = json!({
        "action": "opened",
        "number": 42,
        "issue": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    });
    let (status, rejected) = send(
        address,
        &request_event(
            delivery_id,
            "issues",
            &serde_json::to_vec(&payload).expect("serialize contradictory signed issue event"),
        ),
    );
    assert_eq!(status, 400, "root/nested number conflict: {rejected}");
    assert_eq!(rejected["accepted"], false);
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert!(
        rejected["error"]["action"]
            .as_str()
            .unwrap()
            .contains("not journaled")
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after conflicting number refusal")
            .is_empty(),
        "contradictory signed event must not reserve its delivery ID"
    );

    payload["number"] = json!(ISSUE_NUMBER);
    let (status, accepted) = send(
        address,
        &request_event(
            delivery_id,
            "issues",
            &serde_json::to_vec(&payload).expect("serialize corrected signed issue event"),
        ),
    );
    assert_eq!(status, 202, "corrected redelivery: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected event after receiver shutdown");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["payload"]["number"], ISSUE_NUMBER);
    assert_eq!(
        events[0].payload["payload"]["issue"]["number"],
        ISSUE_NUMBER
    );
}

#[test]
fn actual_receiver_rejects_conflicting_pull_request_root_number_before_append_and_accepts_corrected_retry()
 {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let delivery_id = "pull-request-root-number-corrected-retry";
    let mut payload = json!({
        "action": "opened",
        "number": 42,
        "pull_request": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    });
    let (status, rejected) = send(
        address,
        &request_event(
            delivery_id,
            "pull_request",
            &serde_json::to_vec(&payload)
                .expect("serialize contradictory signed pull-request event"),
        ),
    );
    assert_eq!(status, 400, "root/nested number conflict: {rejected}");
    assert_eq!(rejected["accepted"], false);
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert!(
        rejected["error"]["action"]
            .as_str()
            .unwrap()
            .contains("not journaled")
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after conflicting pull-request number refusal")
            .is_empty(),
        "contradictory signed event must not reserve its delivery ID"
    );

    payload["number"] = json!(ISSUE_NUMBER);
    let (status, accepted) = send(
        address,
        &request_event(
            delivery_id,
            "pull_request",
            &serde_json::to_vec(&payload).expect("serialize corrected signed pull-request event"),
        ),
    );
    assert_eq!(status, 202, "corrected redelivery: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected pull-request event after receiver shutdown");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["payload"]["number"], ISSUE_NUMBER);
    assert_eq!(
        events[0].payload["payload"]["pull_request"]["number"],
        ISSUE_NUMBER
    );
}

#[test]
fn actual_receiver_rejects_conflicting_forgejo_and_gitea_event_aliases_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let mut conflicting_aliases = request_event("event-alias-conflict-42", "issues", &issue_body);
    let event_header = b"X-Forgejo-Event: issues\r\n";
    let header_offset = conflicting_aliases
        .windows(event_header.len())
        .position(|window| window == event_header)
        .expect("request contains the signed event name");
    conflicting_aliases.splice(
        header_offset + event_header.len()..header_offset + event_header.len(),
        b"X-Gitea-Event: pull_request\r\n".iter().copied(),
    );

    // The body signature is valid, but the two supported event-header aliases
    // make incompatible claims. The receiver must refuse before journaling.
    let (status, response) = send(address, &conflicting_aliases);
    assert_eq!(status, 400);
    assert_eq!(response["error"]["code"], "event_header_invalid");
    assert_eq!(response["error"]["retryable"], false);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after conflicting event aliases");
    assert!(
        events.is_empty(),
        "conflicting event aliases must not create a durable inbox record"
    );
}

#[test]
fn actual_receiver_accepts_gitea_compatibility_headers_without_forgejo_aliases() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue payload");

    let forgejo_headers = String::from_utf8(request_event(
        "gitea-compatibility-43",
        "issues",
        &issue_body,
    ))
    .expect("request headers are UTF-8");
    let gitea_only = forgejo_headers
        .replace("X-Forgejo-Delivery:", "X-Gitea-Delivery:")
        .replace("X-Forgejo-Event:", "X-Gitea-Event:")
        .replace("X-Forgejo-Signature:", "X-Gitea-Signature:");
    assert!(
        !gitea_only.contains("X-Forgejo-"),
        "the compatibility request must use only Gitea aliases"
    );
    for header in ["X-Gitea-Delivery:", "X-Gitea-Event:", "X-Gitea-Signature:"] {
        assert!(gitea_only.contains(header), "missing Gitea alias {header}");
    }

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, response) = send(address, gitea_only.as_bytes());
    assert_eq!(status, 202);
    assert_eq!(response["accepted"], true);
    assert_eq!(response["duplicate"], false);
    assert_eq!(response["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after Gitea-compatible delivery");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issues");
    assert_eq!(events[0].payload["delivery_id"], "gitea-compatibility-43");
}

#[test]
fn actual_receiver_rejects_create_payload_relabelled_as_delete_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let create_body = serde_json::to_vec(&json!({
        "ref": "feature/agent-proof",
        "ref_type": "branch",
        "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize signed Forgejo create payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    // The HMAC is valid for these body bytes, but Forgejo does not include the
    // event header in the signature. The receiver must bind it to the shape.
    let relabelled = request_event(
        "create-payload-relabelled-delete-41",
        "delete",
        &create_body,
    );
    let (status, response) = send(address, &relabelled);
    assert_eq!(status, 400);
    assert_eq!(response["error"]["code"], "event_payload_invalid");
    assert_eq!(response["error"]["details"]["field_path"], "payload.sha");
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after create/delete confusion refusal");
    assert!(
        events.is_empty(),
        "rejected event must not poison the inbox"
    );
}

#[test]
fn actual_receiver_rejects_signed_duplicate_json_members_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let cases: [(&str, &[u8]); 2] = [
        (
            "duplicate-member-literal-41",
            br#"{"action":"opened","repository":{"full_name":"forge-admin/agent-forge-core","full_name":"attacker/agent-forge-core"},"issue":{"number":41,"state":"open"}}"#,
        ),
        (
            "duplicate-member-escaped-41",
            br#"{"action":"opened","repository":{"full_name":"forge-admin/agent-forge-core","full\u005fname":"attacker/agent-forge-core"},"issue":{"number":41,"state":"open"}}"#,
        ),
    ];

    for (delivery_id, body) in cases {
        let signed = request_event(delivery_id, "issues", body);
        let (status, response) = send(address, &signed);
        assert_eq!(status, 400, "delivery {delivery_id} must fail closed");
        assert_eq!(response["error"]["code"], "payload_invalid");
        assert_eq!(response["error"]["retryable"], false);
        assert!(response["error"]["details"]["line"].as_u64().unwrap() > 0);
        assert!(response["error"]["details"]["column"].as_u64().unwrap() > 0);
        assert!(!response.to_string().contains("attacker/agent-forge-core"));
        assert_eq!(
            fs::read(&journal_path).ok(),
            journal_before,
            "rejected delivery {delivery_id} must not change the inbox"
        );
    }
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after duplicate-member refusals");
    assert!(events.is_empty(), "no rejected delivery may be journaled");
}

#[test]
fn actual_receiver_reports_line_and_column_for_signed_malformed_json_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let malformed_body = b"{\n  \"UNTRUSTED_SENTINEL\": }";
    let signed = request_event("malformed-json-location-41", "issues", malformed_body);

    let (status, response) = send(address, &signed);

    assert_eq!(status, 400);
    assert_eq!(response["error"]["code"], "payload_invalid");
    assert_eq!(response["error"]["retryable"], false);
    assert_eq!(response["error"]["details"]["line"], 2);
    assert_eq!(response["error"]["details"]["column"], 25);
    assert!(!response.to_string().contains("UNTRUSTED_SENTINEL"));
    assert_eq!(
        response["error"]["action"],
        "correct the JSON at its source using the reported line and column, then redeliver it with a valid signature; the rejected body was not journaled"
    );
    assert!(stop_receiver(&mut receiver).success());

    assert_eq!(fs::read(&journal_path).ok(), journal_before);
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after malformed signed JSON")
            .is_empty()
    );
}

#[test]
fn actual_receiver_withholds_json_location_for_invalid_signature() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let malformed_body = b"{\n  \"UNTRUSTED_SENTINEL\": }";
    let mut signed = request_event(
        "unsigned-malformed-json-location-41",
        "issues",
        malformed_body,
    );
    let signature_start = signed
        .windows(b"X-Forgejo-Signature: ".len())
        .position(|window| window == b"X-Forgejo-Signature: ")
        .expect("request has a signature header")
        + b"X-Forgejo-Signature: ".len();
    signed[signature_start] = if signed[signature_start] == b'0' {
        b'1'
    } else {
        b'0'
    };

    let (status, response) = send(address, &signed);

    assert_eq!(status, 401);
    assert_eq!(response["error"]["code"], "invalid_signature");
    assert_eq!(response["error"]["retryable"], false);
    assert!(response["error"].get("details").is_none());
    assert!(!response.to_string().contains("UNTRUSTED_SENTINEL"));
    assert!(stop_receiver(&mut receiver).success());

    assert_eq!(fs::read(&journal_path).ok(), journal_before);
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after invalid signature")
            .is_empty()
    );
}

#[test]
fn actual_receiver_rejects_review_event_subtype_that_disagrees_with_signed_body() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let mismatches = [
        (
            "review-approved-header",
            "pull_request_approved",
            "pull_request_review_rejected",
        ),
        (
            "review-rejected-header",
            "pull_request_rejected",
            "pull_request_review_comment",
        ),
        (
            "review-comment-header",
            "pull_request_comment",
            "pull_request_review_approved",
        ),
        (
            "review-generic-header",
            "pull_request",
            "pull_request_review_approved",
        ),
    ];
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    for (delivery_id, event_name, body_review_type) in mismatches {
        let body = serde_json::to_vec(&json!({
            "action": "reviewed",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "pull_request": {"number": 14},
            "review": {"type": body_review_type}
        }))
        .expect("serialize signed pull request review payload");
        let request = request_event(delivery_id, event_name, &body);
        let (status, response) = send(address, &request);
        assert_eq!(status, 400, "event {event_name}");
        assert_eq!(
            response["error"]["code"], "event_payload_invalid",
            "event {event_name}"
        );
        assert_eq!(response["error"]["retryable"], false);
    }

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after rejected review subtype mismatches");
    assert!(
        events.is_empty(),
        "mismatched review subtypes must not create durable inbox records"
    );
}

#[test]
fn actual_receiver_rejects_signed_issue_review_member_before_append() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let cases = [
        ("null", Value::Null),
        ("boolean", Value::Bool(false)),
        ("string", Value::String("unexpected".to_owned())),
        ("array", json!([])),
    ];
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();

    for (shape, review) in cases {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER, "state": "open"},
            "review": review
        }))
        .expect("serialize signed issue payload with unexpected review member");
        let delivery_id = format!("issue-review-member-{shape}");
        let (status, response) = send(address, &request(&delivery_id, &body));
        assert_eq!(status, 400, "review member shape {shape} must be rejected");
        assert_eq!(response["error"]["code"], "event_payload_invalid");
        assert_eq!(response["error"]["retryable"], false);
        assert_eq!(response["error"]["details"]["field_path"], "payload.review");
        assert_eq!(
            fs::read(&journal_path).ok(),
            journal_before,
            "rejected review member shape {shape} must not change the inbox"
        );
    }

    assert!(stop_receiver(&mut receiver).success());
    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay inbox after rejected issue payloads");
    assert!(
        events.is_empty(),
        "rejected issue events must not be journaled"
    );
}

#[test]
fn actual_receiver_accepts_forgejo_v17_review_event_names() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let official_events = [
        (
            "forgejo-review-approved",
            "pull_request_approved",
            "pull_request_review_approved",
        ),
        (
            "forgejo-review-rejected",
            "pull_request_rejected",
            "pull_request_review_rejected",
        ),
        (
            "forgejo-review-comment",
            "pull_request_comment",
            "pull_request_review_comment",
        ),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, event_name, review_type) in official_events {
        let body = serde_json::to_vec(&json!({
            "action": "reviewed",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "pull_request": {"number": 14},
            "review": {"type": review_type}
        }))
        .expect("serialize Forgejo v17 review payload");
        let request = request_event(delivery_id, event_name, &body);
        let (status, response) = send(address, &request);
        assert_eq!(status, 202, "Forgejo event {event_name}: {response}");
        assert_eq!(response["accepted"], true);
        assert_eq!(response["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay accepted Forgejo review events");
    assert_eq!(events.len(), official_events.len());
    for (event, (_, event_name, review_type)) in events.iter().zip(official_events) {
        assert_eq!(event.kind, format!("forgejo.webhook.v1.{event_name}"));
        assert_eq!(event.payload["event"], event_name);
        assert_eq!(event.payload["payload"]["review"]["type"], review_type);
    }
}

#[test]
fn actual_receiver_accepts_forgejo_v17_issue_comment_event_for_pull_request_conversation() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "created",
        "issue": {"number": 14},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {"number": 14},
        "comment": {"id": 21, "body": "untrusted comment"},
        "is_pull": true
    }))
    .expect("serialize Forgejo v17 issue-comment payload for a pull request");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let request = request_event("forgejo-pr-comment", "issue_comment", &body);
    let (status, response) = send(address, &request);
    assert_eq!(status, 202, "Forgejo PR comment: {response}");
    assert_eq!(response["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay accepted Forgejo PR conversation comment");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issue_comment");
    assert_eq!(events[0].payload["event"], "issue_comment");
}

#[test]
fn actual_receiver_refuses_pull_comment_without_pull_request_then_accepts_same_delivery() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let delivery_id = "pr-comment-missing-parent-corrected-retry";
    let mut payload = json!({
        "action": "created",
        "issue": {"number": 14},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "comment": {"id": 22, "body": "untrusted comment"},
        "is_pull": true
    });
    let (status, rejected) = send(
        address,
        &request_event(
            delivery_id,
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize incomplete PR-comment payload"),
        ),
    );
    assert_eq!(status, 400, "is_pull without pull_request: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert!(
        rejected["error"]["action"]
            .as_str()
            .unwrap()
            .contains("not journaled")
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after missing-PR refusal")
            .is_empty(),
        "an invalid signed event must not reserve its delivery ID"
    );

    payload["pull_request"] = json!({"number": 14});
    let (status, accepted) = send(
        address,
        &request_event(
            delivery_id,
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize corrected PR-comment payload"),
        ),
    );
    assert_eq!(status, 202, "corrected same-ID redelivery: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected PR-comment event");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["payload"]["pull_request"]["number"], 14);
    assert_eq!(events[0].payload["payload"]["is_pull"], true);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_refuses_issue_comment_marked_non_pull_with_pr_parent_then_accepts_same_delivery()
{
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let delivery_id = "issue-comment-false-is-pull-parent-corrected-retry";
    let mut payload = json!({
        "action": "created",
        "issue": {"number": 14},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "comment": {"id": 24, "body": "private-mismatch-marker"},
        "is_pull": false,
        "pull_request": {"number": 14}
    });
    let (status, rejected) = send(
        address,
        &request_event(
            delivery_id,
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize contradictory issue-comment payload"),
        ),
    );
    assert_eq!(status, 400, "false is_pull with PR parent: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert!(
        !rejected.to_string().contains("private-mismatch-marker"),
        "rejected user content must not be reflected in the HTTP error"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after issue/PR classification refusal")
            .is_empty(),
        "a contradictory signed event must not reserve its delivery ID"
    );

    payload
        .as_object_mut()
        .expect("issue-comment fixture is an object")
        .remove("pull_request");
    let (status, accepted) = send(
        address,
        &request_event(
            delivery_id,
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize corrected issue-comment payload"),
        ),
    );
    assert_eq!(status, 202, "corrected same-ID redelivery: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected issue-comment event");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["delivery_id"], delivery_id);
    assert_eq!(events[0].payload["payload"]["is_pull"], false);
    assert!(events[0].payload["payload"].get("pull_request").is_none());

    let journal_before = fs::read(&journal_path).expect("snapshot corrected issue-comment inbox");
    let issue_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/14");
    let fixture = start_cli_http_fixture_responses(vec![
        (
            200,
            json!({
                "number": 14,
                "state": "closed",
                "updated_at": "2026-10-28T14:00:00Z",
                "html_url": issue_url
            }),
        ),
        (
            200,
            json!({
                "id": 24,
                "body": "canonical current issue comment",
                "updated_at": "2026-10-28T14:01:00Z",
                "html_url": format!("{issue_url}#issuecomment-24"),
                "issue_url": issue_url
            }),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI for the corrected issue comment");
    let requests = fixture
        .join()
        .expect("issue and comment API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed with status {:?}; stdout={}; stderr={}; requests={requests:?}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("private-mismatch-marker"),
        "the rejected/comment webhook body must not become canonical CLI output"
    );
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/14 HTTP/1.1\r\n")
    );
    assert!(requests[1].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/issues/comments/24 HTTP/1.1\r\n"
    ));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("corrected issue comment returns structured reconciliation JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "issue_comment");
    assert_eq!(value["resource"]["issue_number"], 14);
    assert_eq!(value["resource"]["comment_id"], 24);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["html_url"], issue_url);
    assert_eq!(value["canonical_state"]["comment"]["state"], "present");
    assert_eq!(
        value["canonical_state"]["comment"]["body"],
        "canonical current issue comment"
    );
    assert_eq!(
        value["canonical_state"]["comment"]["body_trust"],
        "untrusted_user_content"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read reconciled issue-comment inbox"),
        journal_before,
        "reconciliation must not rewrite the accepted corrected event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_refuses_issue_comment_without_is_pull_before_reserving_delivery() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let payload = json!({
        "action": "created",
        "issue": {"number": 14},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "comment": {"id": 23, "body": "untrusted comment"}
    });
    let (status, rejected) = send(
        address,
        &request_event(
            "issue-comment-missing-is-pull",
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize missing-is_pull payload"),
        ),
    );
    assert_eq!(status, 400, "missing Forgejo is_pull field: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        rejected["error"]["details"]["field_path"],
        "payload.is_pull"
    );
    assert_eq!(
        rejected["error"]["action"],
        "set payload.is_pull to a JSON boolean; true requires payload.pull_request.number to equal payload.issue.number, false requires payload.pull_request to be absent; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after missing-is_pull refusal")
            .is_empty(),
        "a payload missing the required classification field must not reserve its delivery ID"
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_refuses_zero_issue_comment_id_with_actionable_field_guidance() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let payload = json!({
        "action": "created",
        "issue": {"number": 14},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "comment": {"id": 0, "body": "untrusted comment"},
        "is_pull": false
    });
    let delivery_id = "issue-comment-zero-id-actionable-error";
    let (status, rejected) = send(
        address,
        &request_event(
            delivery_id,
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize zero-comment-ID payload"),
        ),
    );
    assert_eq!(status, 400, "zero comment ID: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        rejected["error"]["details"]["field_path"],
        "payload.comment.id"
    );
    assert_eq!(
        rejected["error"]["action"],
        "set payload.comment.id to a positive JSON integer; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay after zero-comment-ID refusal")
            .is_empty()
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_refuses_missing_create_sha_with_actionable_guidance() {
    const DELIVERY_ID: &str = "branch-create-missing-sha-actionable-error";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": "feature/missing-object-id",
        "ref_type": "branch",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize create payload with missing object ID");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, rejected) = send(address, &request_event(DELIVERY_ID, "create", &body));
    assert_eq!(status, 400, "create without sha: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(rejected["error"]["details"]["field_path"], "payload.sha");
    assert_eq!(
        rejected["error"]["action"],
        "set payload.sha to a nonzero 40- or 64-character hexadecimal commit ID; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after missing-create-sha refusal")
            .is_empty()
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_refuses_invalid_create_branch_ref_with_actionable_guidance() {
    const DELIVERY_ID: &str = "branch-create-invalid-ref-actionable-error";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": ".hidden/release",
        "ref_type": "branch",
        "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize create payload with invalid branch name");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, rejected) = send(address, &request_event(DELIVERY_ID, "create", &body));
    assert_eq!(status, 400, "invalid create ref: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(rejected["error"]["details"]["field_path"], "payload.ref");
    assert_eq!(
        rejected["error"]["action"],
        "set payload.ref to a Git-valid branch name with no empty path component or component starting with dot, no trailing slash or dot, and no Git ref metacharacters; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after invalid-create-ref refusal")
            .is_empty()
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_reports_tag_create_as_unsupported_payload_field_without_journaling() {
    const DELIVERY_ID: &str = "branch-create-tag-unsupported-field";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let journal = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&journal_path)
        .expect("create empty inbox before receiver startup");
    journal.sync_all().expect("sync empty inbox");
    drop(journal);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let (capabilities_status, capabilities) = send(
        address,
        b"GET /forgejo/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    );
    assert_eq!(capabilities_status, 200);
    let create_contract = capabilities["webhook_ingest"]["event_contracts_v8"]["contracts"]
        .as_array()
        .expect("machine-readable event contracts")
        .iter()
        .find(|contract| contract["event"] == "create")
        .expect("create contract is advertised");
    assert!(
        create_contract["payload_constraints"]
            .as_array()
            .expect("payload constraints")
            .iter()
            .any(|constraint| constraint == "ref_type is branch")
    );
    let empty_inbox = fs::read(&journal_path).expect("read inbox after capabilities GET");

    let body = serde_json::to_vec(&json!({
        "ref": "v1.0.0",
        "ref_type": "tag",
        "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize valid Forgejo tag-create payload");
    let (status, rejected) = send(address, &request_event(DELIVERY_ID, "create", &body));
    assert_eq!(status, 422, "tag ref is outside this adapter: {rejected}");
    assert_eq!(rejected["error"]["code"], "unsupported_event");
    assert_eq!(rejected["error"]["retryable"], false);
    assert_eq!(
        rejected["error"]["details"]["field_path"],
        "payload.ref_type"
    );
    assert_eq!(
        rejected["error"]["action"],
        "this adapter accepts branch refs only; exclude tag create/delete deliveries at a trusted sender-side filter, or disable those event subscriptions until explicit tag support is added; consult GET /forgejo/capabilities; the rejected body was not journaled"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after unsupported tag refusal"),
        empty_inbox,
        "an unsupported tag event must not alter the durable inbox"
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_classifies_tag_delete_and_non_string_ref_type_without_journaling() {
    const TAG_DELIVERY_ID: &str = "branch-delete-tag-unsupported-field";
    const MALFORMED_DELIVERY_ID: &str = "branch-delete-ref-type-malformed";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let journal = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&journal_path)
        .expect("create empty inbox before receiver startup");
    journal.sync_all().expect("sync empty inbox");
    drop(journal);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);

    let (capabilities_status, capabilities) = send(
        address,
        b"GET /forgejo/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    );
    assert_eq!(capabilities_status, 200);
    let delete_contract = capabilities["webhook_ingest"]["event_contracts_v8"]["contracts"]
        .as_array()
        .expect("machine-readable event contracts")
        .iter()
        .find(|contract| contract["event"] == "delete")
        .expect("delete contract is advertised");
    assert!(
        delete_contract["payload_constraints"]
            .as_array()
            .expect("payload constraints")
            .iter()
            .any(|constraint| constraint == "ref_type is branch")
    );
    let empty_inbox = fs::read(&journal_path).expect("read inbox after capabilities GET");

    let tag_delete = serde_json::to_vec(&json!({
        "ref": "refs/tags/v1.0.0",
        "ref_type": "tag",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize valid Forgejo tag-delete payload");
    let (tag_status, tag_rejected) = send(
        address,
        &request_event(TAG_DELIVERY_ID, "delete", &tag_delete),
    );
    assert_eq!(
        tag_status, 422,
        "tag ref is outside this adapter: {tag_rejected}"
    );
    assert_eq!(tag_rejected["error"]["code"], "unsupported_event");
    assert_eq!(tag_rejected["error"]["retryable"], false);
    assert_eq!(
        tag_rejected["error"]["details"]["field_path"],
        "payload.ref_type"
    );
    assert_eq!(
        tag_rejected["error"]["action"],
        "this adapter accepts branch refs only; exclude tag create/delete deliveries at a trusted sender-side filter, or disable those event subscriptions until explicit tag support is added; consult GET /forgejo/capabilities; the rejected body was not journaled"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after unsupported tag-delete refusal"),
        empty_inbox,
        "an unsupported tag-delete event must not alter the durable inbox"
    );

    let malformed_ref_type = serde_json::to_vec(&json!({
        "ref": "feature/proof",
        "ref_type": 1,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize malformed Forgejo delete payload");
    let (malformed_status, malformed_rejected) = send(
        address,
        &request_event(MALFORMED_DELIVERY_ID, "delete", &malformed_ref_type),
    );
    assert_eq!(
        malformed_status, 400,
        "non-string ref_type is malformed, not a supported tag: {malformed_rejected}"
    );
    assert_eq!(malformed_rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(malformed_rejected["error"]["retryable"], false);
    assert_eq!(
        malformed_rejected["error"]["details"]["field_path"],
        "payload.ref_type"
    );
    assert_eq!(
        malformed_rejected["error"]["action"],
        "set payload.ref_type to branch for branch create/delete events; tag refs are not accepted by this adapter; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after malformed ref_type refusal"),
        empty_inbox,
        "a malformed delete event must not alter the durable inbox"
    );

    assert!(stop_receiver(&mut receiver).success());
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after both delete refusals")
            .is_empty()
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; receiver uses an ephemeral loopback port"]
fn actual_receiver_restarts_after_missing_is_pull_and_accepts_corrected_same_delivery() {
    const DELIVERY_ID: &str = "issue-comment-missing-is-pull-recovered-after-restart";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let malformed = json!({
        "action": "created",
        "issue": {"number": 14},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "comment": {"id": 24, "body": "untrusted comment"}
    });
    let (mut first_receiver, first_address) = start_ready_receiver(&secret, &journal_path);
    let (status, rejected) = send(
        first_address,
        &request_event(
            DELIVERY_ID,
            "issue_comment",
            &serde_json::to_vec(&malformed).expect("serialize malformed issue comment"),
        ),
    );
    assert_eq!(status, 400, "missing Forgejo is_pull field: {rejected}");
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert!(stop_receiver(&mut first_receiver).success());
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay inbox after first receiver stops")
            .is_empty(),
        "the rejected payload must not reserve its delivery ID before restart"
    );

    let corrected = json!({
        "action": "created",
        "issue": {"number": 14},
        "pull_request": {"number": 14},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "comment": {"id": 24, "body": "untrusted corrected comment"},
        "is_pull": true
    });
    let (mut second_receiver, second_address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        second_address,
        &request_event(
            DELIVERY_ID,
            "issue_comment",
            &serde_json::to_vec(&corrected).expect("serialize corrected issue comment"),
        ),
    );
    assert_eq!(status, 202, "corrected same-ID delivery: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut second_receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected delivery after receiver restart");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].idempotency_key,
        format!("forgejo-delivery-v1:{DELIVERY_ID}")
    );
    assert_eq!(events[0].payload["payload"]["is_pull"], true);
    assert_eq!(events[0].payload["payload"]["pull_request"]["number"], 14);
}

#[test]
fn actual_receiver_process_handles_parallel_delivery_conflict_submissions_across_restart() {
    const DELIVERY_ID: &str = "delivery-concurrent-conflict-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let bodies = ["open", "closed"].map(|state| {
        serde_json::to_vec(&json!({
            "action": "edited",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER, "state": state}
        }))
        .expect("serialize conflicting signed webhook payload")
    });
    let requests = bodies.map(|body| request(DELIVERY_ID, &body));

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let start = std::sync::Arc::new(Barrier::new(3));
    let senders = requests
        .clone()
        .into_iter()
        .enumerate()
        .map(|(index, request)| {
            let start = start.clone();
            thread::spawn(move || {
                start.wait();
                let (status, response) = send(address, &request);
                (index, status, response)
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    let responses = senders
        .into_iter()
        .map(|sender| sender.join().expect("concurrent delivery sender completed"))
        .collect::<Vec<_>>();

    let accepted = responses
        .iter()
        .find(|(_, status, response)| *status == 202 && response["duplicate"] == false)
        .expect("exactly one distinct payload wins the delivery ID");
    let conflict = responses
        .iter()
        .find(|(_, status, response)| {
            *status == 409 && response["error"]["code"] == "delivery_conflict"
        })
        .expect("the competing payload is rejected as a conflict");
    assert_ne!(accepted.0, conflict.0);
    assert_eq!(responses.len(), 2);
    assert!(stop_receiver(&mut receiver).success());

    let winner_state = if accepted.0 == 0 { "open" } else { "closed" };
    let journal = EventJournal::open(&journal_path);
    let durable = journal
        .replay()
        .expect("replay the journal after concurrent writes");
    assert_eq!(durable.len(), 1, "only the winning body is durable");
    assert_eq!(
        durable[0].payload["payload"]["issue"]["state"],
        winner_state
    );
    assert_eq!(
        durable[0].idempotency_key,
        format!("forgejo-delivery-v1:{DELIVERY_ID}")
    );

    let (mut recovered_receiver, recovered_address) = start_ready_receiver(&secret, &journal_path);
    let (status, duplicate) = send(recovered_address, &requests[accepted.0]);
    assert_eq!(status, 202);
    assert_eq!(duplicate["duplicate"], true);
    let (status, conflict_after_restart) = send(recovered_address, &requests[conflict.0]);
    assert_eq!(status, 409);
    assert_eq!(conflict_after_restart["error"]["code"], "delivery_conflict");
    assert!(stop_receiver(&mut recovered_receiver).success());
    let recovered = EventJournal::open(&journal_path)
        .replay()
        .expect("replay the journal after retrying both bodies");
    assert_eq!(recovered.len(), 1, "neither retry appends another event");
    assert_eq!(
        recovered[0].payload["payload"]["issue"]["state"],
        winner_state
    );
}

#[test]
fn receiver_creates_private_journal_and_restarts_under_umask_022() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER}
    }))
    .expect("serialize minimal issue event");

    let (mut receiver, address) = start_ready_receiver_with_umask(&secret, &journal_path, 0o022);
    let (status, accepted) = send(address, &request("private-journal-41", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let journal_mode = fs::metadata(&journal_path)
        .expect("receiver created the inbox journal")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(journal_mode, 0o600, "new journal must be owner-only");

    let (mut recovered_receiver, _) =
        start_ready_receiver_with_umask(&secret, &journal_path, 0o022);
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);
    assert!(stop_receiver(&mut recovered_receiver).success());
}

#[test]
fn capabilities_endpoint_describes_the_versioned_webhook_contract_without_mutating_inbox() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let request = b"GET /forgejo/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";

    let (status, capabilities) = send(address, request);

    assert_eq!(status, 200);
    assert_eq!(capabilities["schema_version"], 9);
    assert_eq!(capabilities["provider"], "forgejo");
    assert_eq!(
        capabilities["capability_endpoint"],
        json!({"method":"GET", "path":"/forgejo/capabilities"})
    );
    assert_eq!(
        capabilities["error_contract"],
        json!({
            "schema_version": 2,
            "required_error_fields": ["code", "retryable", "action"],
            "optional_error_fields": ["details"],
            "event_payload_invalid": {
                "status": 400,
                "details_field_path": "payload.<field>"
            },
            "journal_busy": {
                "status": 503,
                "retryable": true,
                "retry_after_seconds": 5,
                "action": "retry the exact signed delivery after the active inbox operation completes; do not edit the inbox"
            },
            "server_busy": {
                "status": 503,
                "retryable": true,
                "retry_after_seconds": 5,
                "action": "retry the exact signed delivery after the receiver has drained its bounded request queue"
            },
            "malformed_json": {
                "status": 400,
                "error_code": "payload_invalid",
                "details_fields": ["line", "column"]
            }
        })
    );
    assert_eq!(
        capabilities["webhook_ingest"],
        json!({
            "method": "POST",
            "path": "/forgejo/webhook",
            "content_type": "application/json",
            "signature": {
                "algorithm": "HMAC-SHA256",
                "headers": ["X-Forgejo-Signature", "X-Gitea-Signature"]
            },
            "delivery_id_headers": ["X-Forgejo-Delivery", "X-Gitea-Delivery"],
            "event_headers": ["X-Forgejo-Event", "X-Gitea-Event"],
            "supported_events": [
                "issues",
                "issue_comment",
                "pull_request",
                "push",
                "create",
                "delete",
                "pull_request_approved",
                "pull_request_rejected",
                "pull_request_comment",
                "workflow_run_blocked",
                "workflow_run_cancelled",
                "workflow_run_failure",
                "workflow_run_running",
                "workflow_run_skipped",
                "workflow_run_success",
                "workflow_run_waiting",
                "workflow_job_blocked",
                "workflow_job_cancelled",
                "workflow_job_failure",
                "workflow_job_running",
                "workflow_job_skipped",
                "workflow_job_success",
                "workflow_job_waiting",
                "release"
            ],
            "event_contracts": {
                "schema_version": 1,
                "webhook_payload_is_authoritative": false,
                "contracts": SUPPORTED_EVENT_CONTRACTS_V7,
            },
            "event_contracts_v2": {
                "schema_version": 2,
                "webhook_payload_is_authoritative": false,
                "contracts": SUPPORTED_EVENT_CONTRACTS_V7,
                "workflow_statuses": SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
            },
            "event_contracts_v3": {
                "schema_version": 3,
                "webhook_payload_is_authoritative": false,
                "contracts": SUPPORTED_EVENT_CONTRACTS_V7,
                "workflow_statuses": SUPPORTED_WORKFLOW_STATUS_CONTRACTS,
                "workflow_actions": SUPPORTED_WORKFLOW_ACTION_CONTRACTS,
            },
            "event_contracts_v4": WEBHOOK_EVENT_CONTRACTS_V4,
            "event_contracts_v5": WEBHOOK_EVENT_CONTRACTS_V5,
            "event_contracts_v6": WEBHOOK_EVENT_CONTRACTS_V6,
            "event_contracts_v7": WEBHOOK_EVENT_CONTRACTS_V7,
            "event_contracts_v8": WEBHOOK_EVENT_CONTRACTS_V8,
            "event_contracts_v9": WEBHOOK_EVENT_CONTRACTS_V9,
            "idempotency": {
                "same_delivery_and_payload": {"status": 202, "duplicate": true},
                "same_delivery_with_different_payload": {
                    "status": 409,
                    "error_code": "delivery_conflict"
                }
            }
        })
    );
    assert_eq!(
        capabilities["limits"],
        json!({"max_header_bytes": 16384, "max_body_bytes": 1048576})
    );
    assert!(capabilities.get("secret").is_none());
    assert!(capabilities.get("credential").is_none());

    let wrong_method =
        b"POST /forgejo/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n";
    let (status, rejected) = send(address, wrong_method);
    assert_eq!(status, 405);
    assert_eq!(rejected["error"]["code"], "method_not_allowed");
    assert_eq!(
        fs::read(&journal_path).ok(),
        journal_before,
        "capability discovery must not append to the webhook inbox"
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
fn duplicate_host_header_is_rejected_before_the_webhook_is_journaled() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let valid = String::from_utf8(request("duplicate-host-41", &body)).unwrap();
    let duplicated = valid.replacen(
        "Host: 127.0.0.1\r\n",
        "Host: 127.0.0.1\r\nHost: attacker.invalid\r\n",
        1,
    );

    let (status, rejected) = send(address, duplicated.as_bytes());

    assert_eq!(status, 400);
    assert_eq!(rejected["error"]["code"], "request_malformed");
    assert_eq!(
        fs::read(&journal_path).ok(),
        journal_before,
        "a malformed HTTP authority must not append a signed event"
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
fn missing_host_header_in_http11_is_rejected_before_the_webhook_is_journaled() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");
    let valid = String::from_utf8(request("missing-host-41", &body)).unwrap();
    let without_host = valid.replace("Host: 127.0.0.1\r\n", "");

    let (status, rejected) = send(address, without_host.as_bytes());

    assert_eq!(status, 400);
    assert_eq!(rejected["error"]["code"], "request_malformed");
    assert_eq!(
        fs::read(&journal_path).ok(),
        journal_before,
        "HTTP/1.1 without Host must not append a signed event"
    );
    assert!(stop_receiver(&mut receiver).success());
}

#[test]
fn malformed_host_authority_is_rejected_before_the_webhook_is_journaled() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let journal_before = fs::read(&journal_path).ok();
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "issue": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
    }))
    .expect("serialize signed issue event");

    let mut failures = Vec::new();
    for (index, invalid_host) in [
        "forgejo.local/path",
        "forgejo.local:abc",
        "[not-ipv6]",
        "forgejo.local extra",
        "user@forgejo.local",
    ]
    .into_iter()
    .enumerate()
    {
        let valid = String::from_utf8(request(&format!("invalid-host-{index}-41"), &body))
            .expect("valid request template is UTF-8");
        let malformed = valid.replace("Host: 127.0.0.1", &format!("Host: {invalid_host}"));

        let (status, rejected) = send(address, malformed.as_bytes());

        if status != 400 || rejected["error"]["code"] != "request_malformed" {
            failures.push(format!(
                "Host value {invalid_host:?} returned HTTP {status} with {:?}",
                rejected["error"]["code"]
            ));
        }
        if fs::read(&journal_path).ok() != journal_before {
            failures.push(format!(
                "Host value {invalid_host:?} changed the inbox journal"
            ));
        }
    }
    assert!(stop_receiver(&mut receiver).success());
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

fn read_cli_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound fixture request read");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while request.len() < 16 * 1024 && !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => request.extend_from_slice(&buffer[..count]),
            Err(error) => panic!("read reconciliation CLI request headers: {error}"),
        }
    }
    assert!(
        request.windows(4).any(|window| window == b"\r\n\r\n"),
        "reconciliation CLI request headers are incomplete or oversized"
    );
    String::from_utf8(request).expect("fixture request headers are UTF-8")
}

fn wait_for_process_output_with_timeout(mut child: Child, timeout: Duration) -> (Output, bool) {
    let stdout = child.stdout.take().expect("child stdout is piped");
    let stderr = child.stderr.take().expect("child stderr is piped");
    let stdout_reader = thread::spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let process_group = i32::try_from(child.id()).expect("child PID fits pid_t");
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().expect("poll bounded CLI process") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
            break (
                child
                    .wait()
                    .expect("reap cache-budget CLI after its process-group deadline"),
                true,
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    // The timeout branch kills the CLI and any curl descendant in its isolated
    // group before joining pipe readers.
    let stdout = stdout_reader
        .join()
        .expect("join bounded CLI stdout reader")
        .expect("read bounded CLI stdout");
    let stderr = stderr_reader
        .join()
        .expect("join bounded CLI stderr reader")
        .expect("read bounded CLI stderr");
    (
        Output {
            status,
            stdout,
            stderr,
        },
        timed_out,
    )
}

#[test]
fn process_output_deadline_kills_a_cli_process_group() {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("sleep 30")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn bounded process-group fixture");
    let started = Instant::now();
    let (output, timed_out) =
        wait_for_process_output_with_timeout(child, Duration::from_millis(100));

    assert!(timed_out);
    assert_eq!(output.status.signal(), Some(libc::SIGKILL));
    assert!(started.elapsed() < Duration::from_secs(5));
}

fn start_cli_http_fixture(
    status: u16,
    response_body: Value,
    expected_requests: usize,
) -> thread::JoinHandle<Vec<String>> {
    let fixture = start_cli_http_fixture_with_headers(
        status,
        response_body,
        expected_requests,
        Vec::new(),
        None,
        None,
    );
    thread::spawn(move || fixture.join().expect("HTTP fixture worker completes").0)
}

fn start_cli_http_raw_body_fixture(response_body: Vec<u8>) -> thread::JoinHandle<String> {
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make raw response fixture accept deadline bounded");

    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "reconciliation CLI made no request to raw API fixture"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept raw reconciliation request: {error}"),
            }
        };
        let request = read_cli_http_request(&mut stream);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response_body.len(),
        );
        stream
            .write_all(response.as_bytes())
            .expect("write raw fixture response headers");
        stream
            .write_all(&response_body)
            .expect("write raw fixture response body");
        request
    })
}

fn start_cli_http_fixture_responses(
    responses: Vec<(u16, Value)>,
) -> thread::JoinHandle<Vec<String>> {
    assert!(
        !responses.is_empty(),
        "fixture must have at least one response"
    );
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make fixture accept deadline bounded");

    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut requests = Vec::with_capacity(responses.len());
        for (status, response_body) in responses {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "reconciliation CLI made too few requests"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept reconciliation CLI request: {error}"),
                }
            };
            let request = read_cli_http_request(&mut stream);
            let body = serde_json::to_vec(&response_body).expect("serialize sequenced response");
            let reason = match status {
                200 => "OK",
                403 => "Forbidden",
                404 => "Not Found",
                _ => "Fixture",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .expect("write sequenced fixture response headers");
            stream
                .write_all(&body)
                .expect("write sequenced fixture response body");
            requests.push(request);
        }
        requests
    })
}

fn start_slow_cli_http_fixture(
    fast_response_delay: Duration,
    fast_response_count: usize,
    pending_response_delay: Duration,
    stop_receiver: mpsc::Receiver<()>,
) -> thread::JoinHandle<Vec<(Instant, String)>> {
    assert!(!fast_response_delay.is_zero());
    assert!(fast_response_count > 0);
    assert!(!pending_response_delay.is_zero());
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make slow fixture accept deadline bounded");

    thread::spawn(move || {
        let hard_deadline = Instant::now() + Duration::from_secs(42);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_count = Arc::new(AtomicUsize::new(0));
        let stop_handlers = Arc::new(AtomicBool::new(false));
        let mut handlers = Vec::new();
        loop {
            if stop_receiver.try_recv().is_ok() {
                stop_handlers.store(true, Ordering::Release);
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let request = read_cli_http_request(&mut stream);
                            let received_at = Instant::now();
                            request_count.fetch_add(1, Ordering::Relaxed);
                            requests
                                .lock()
                                .expect("lock slow fixture request log")
                                .push((received_at, request));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) => panic!("drain slow reconciliation fixture: {error}"),
                    }
                }
                break;
            }
            assert!(
                Instant::now() < hard_deadline,
                "slow fixture did not receive stop after bounded CLI completion"
            );
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("accept slow reconciliation request: {error}"),
            };
            let request_index = request_count.fetch_add(1, Ordering::Relaxed) + 1;
            let requests = Arc::clone(&requests);
            let stop_handlers = Arc::clone(&stop_handlers);
            handlers.push(thread::spawn(move || {
                let request = read_cli_http_request(&mut stream);
                let received_at = Instant::now();
                let number = request
                    .split_whitespace()
                    .nth(1)
                    .and_then(|path| path.rsplit('/').next())
                    .and_then(|number| number.parse::<u64>().ok())
                    .expect("fixture request ends in the issue number");
                requests
                    .lock()
                    .expect("lock slow fixture request log")
                    .push((received_at, request));

                let response_delay = if request_index <= fast_response_count {
                    fast_response_delay
                } else {
                    pending_response_delay
                };
                let response_deadline = Instant::now() + response_delay;
                while Instant::now() < response_deadline {
                    if stop_handlers.load(Ordering::Acquire) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                if stop_handlers.load(Ordering::Acquire) {
                    return;
                }

                let body = serde_json::to_vec(&json!({
                    "number": number,
                    "state": "open",
                    "updated_at": "2026-10-22T18:00:00Z",
                    "html_url": format!(
                        "http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{number}"
                    )
                }))
                .expect("serialize delayed current-state fixture");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&body);
            }));
        }

        stop_handlers.store(true, Ordering::Release);
        for handler in handlers {
            handler.join().expect("slow response handler completes");
        }
        let mut requests = Arc::try_unwrap(requests)
            .expect("all slow fixture request handlers release the log")
            .into_inner()
            .expect("slow fixture request log is not poisoned");
        requests.sort_by_key(|(received_at, _)| *received_at);
        requests
    })
}

fn http_date_after(seconds_from_now: u64) -> String {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let target_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after UNIX_EPOCH")
        .as_secs()
        .checked_add(seconds_from_now)
        .expect("future HTTP date fits in u64 seconds");
    let target = libc::time_t::try_from(target_seconds).expect("future HTTP date fits time_t");
    let mut utc = unsafe { std::mem::zeroed::<libc::tm>() };
    assert!(
        !unsafe { libc::gmtime_r(&target, &mut utc) }.is_null(),
        "convert Retry-After epoch to UTC"
    );
    let weekday = WEEKDAYS[usize::try_from(utc.tm_wday).expect("valid weekday")];
    let month = MONTHS[usize::try_from(utc.tm_mon).expect("valid month")];
    format!(
        "{weekday}, {:02} {month} {:04} {:02}:{:02}:{:02} GMT",
        utc.tm_mday,
        utc.tm_year + 1900,
        utc.tm_hour,
        utc.tm_min,
        utc.tm_sec
    )
}

fn start_cli_http_fixture_with_headers(
    status: u16,
    response_body: Value,
    expected_requests: usize,
    response_headers: Vec<(String, String)>,
    observe_after_expected: Option<Duration>,
    retry_after_http_date_after: Option<Duration>,
) -> thread::JoinHandle<(Vec<String>, Vec<Duration>)> {
    assert!(
        expected_requests > 0,
        "fixture must expect at least one request"
    );
    if let Some(window) = observe_after_expected {
        assert!(!window.is_zero(), "an observation window must be positive");
    }
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make fixture accept deadline bounded");
    let body = serde_json::to_vec(&response_body).expect("serialize HTTP response fixture");
    let extra_headers = response_headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();

    thread::spawn(move || {
        let deadline = Instant::now()
            + Duration::from_secs(5)
            + retry_after_http_date_after.unwrap_or_default();
        let mut requests = Vec::with_capacity(expected_requests);
        let mut response_to_next_request = Vec::new();
        let mut last_response_at = None;
        let mut observe_until = None;
        loop {
            if requests.len() >= expected_requests {
                match (observe_after_expected, observe_until) {
                    (None, _) => break,
                    (Some(window), None) => observe_until = Some(Instant::now() + window),
                    (Some(_), Some(until)) if Instant::now() >= until => break,
                    (Some(_), Some(_)) => {}
                }
            }
            let accept_deadline = observe_until.unwrap_or(deadline);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= accept_deadline {
                            if observe_until.is_some() && requests.len() >= expected_requests {
                                return (requests, response_to_next_request);
                            }
                            panic!(
                                "reconciliation CLI made too few API requests or did not become quiet"
                            );
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept reconciliation CLI request: {error}"),
                }
            };
            if let Some(response_at) = last_response_at.take() {
                response_to_next_request
                    .push(Instant::now().saturating_duration_since(response_at));
            }
            let request = read_cli_http_request(&mut stream);
            let reason = match status {
                200 => "OK",
                403 => "Forbidden",
                429 => "Too Many Requests",
                503 => "Service Unavailable",
                _ => "Fixture",
            };
            let retry_after_header = if requests.is_empty() {
                retry_after_http_date_after
                    .map(|delay| format!("Retry-After: {}\r\n", http_date_after(delay.as_secs())))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n{extra_headers}{retry_after_header}Connection: close\r\n\r\n",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .expect("write fixture response headers");
            stream
                .write_all(&body)
                .expect("write fixture response body");
            requests.push(request);
            last_response_at = Some(Instant::now());
        }
        (requests, response_to_next_request)
    })
}

fn start_cli_http_fixture_until_stopped(
    status: u16,
    response_body: Value,
    response_headers: Vec<(String, String)>,
) -> (mpsc::Sender<()>, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make fixture accept deadline bounded");
    let body = serde_json::to_vec(&response_body).expect("serialize HTTP response fixture");
    let extra_headers = response_headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    let (stop_sender, stop_receiver) = mpsc::channel();

    let fixture = thread::spawn(move || {
        const QUIET_AFTER_STOP: Duration = Duration::from_millis(500);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut stop_quiet_deadline = None;
        let mut requests = Vec::new();

        loop {
            assert!(
                Instant::now() < deadline,
                "reconciliation CLI fixture was not stopped before its deadline"
            );

            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_cli_http_request(&mut stream);
                    let reason = if status == 302 { "Found" } else { "Fixture" };
                    let response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n",
                        body.len(),
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write fixture response headers");
                    stream
                        .write_all(&body)
                        .expect("write fixture response body");
                    requests.push(request);

                    if stop_quiet_deadline.is_some() {
                        stop_quiet_deadline = Some(Instant::now() + QUIET_AFTER_STOP);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stop_quiet_deadline.is_none() {
                        match stop_receiver.try_recv() {
                            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                                stop_quiet_deadline = Some(Instant::now() + QUIET_AFTER_STOP);
                            }
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                    }

                    if stop_quiet_deadline
                        .is_some_and(|quiet_deadline| Instant::now() >= quiet_deadline)
                    {
                        break;
                    }

                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("accept reconciliation CLI request: {error}"),
            }
        }

        requests
    });

    (stop_sender, fixture)
}

fn start_scripted_cli_http_fixture(
    responses: Vec<(u16, Value, Vec<(String, String)>)>,
    observe_after_expected: Duration,
) -> thread::JoinHandle<Vec<String>> {
    assert!(!responses.is_empty(), "fixture needs at least one response");
    assert!(
        !observe_after_expected.is_zero(),
        "an observation window must be positive"
    );
    start_scripted_cli_http_fixture_inner(
        responses,
        ScriptedFixtureCompletion::QuietWindow(observe_after_expected),
    )
}

fn start_scripted_cli_http_fixture_with_retry_timing(
    responses: Vec<(u16, Value, Vec<(String, String)>)>,
    observe_after_expected: Duration,
) -> thread::JoinHandle<(Vec<String>, Duration)> {
    assert_eq!(
        responses.len(),
        2,
        "timed fixture needs one retry and one readback"
    );
    assert!(
        !observe_after_expected.is_zero(),
        "an observation window must be positive"
    );
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make timed fixture accept deadline bounded");

    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut requests = Vec::with_capacity(responses.len());
        let mut first_response_sent = None;
        let mut retry_delay = None;
        let mut drain_until = None;
        let fallback_response = (
            429,
            json!({"message":"unexpected retry response"}),
            vec![("Retry-After".to_owned(), "3600".to_owned())],
        );
        loop {
            if requests.len() >= responses.len()
                && drain_until.is_some_and(|until| Instant::now() >= until)
            {
                return (requests, retry_delay.expect("second request was timed"));
            }
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "timed fixture exceeded its deadline"
                    );
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("accept timed reconciliation CLI request: {error}"),
            };
            let accepted_at = Instant::now();
            let request = read_cli_http_request(&mut stream);
            let index = requests.len();
            requests.push(request);
            let (status, body, headers) = responses.get(index).unwrap_or(&fallback_response);
            let reason = match *status {
                200 => "OK",
                429 => "Too Many Requests",
                503 => "Service Unavailable",
                _ => "Fixture",
            };
            let body = serde_json::to_vec(body).expect("serialize timed fixture response");
            let extra_headers = headers
                .iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>();
            let response = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
                status,
                reason,
                body.len(),
                extra_headers,
            );
            stream
                .write_all(response.as_bytes())
                .expect("write timed fixture response headers");
            stream
                .write_all(&body)
                .expect("write timed fixture response body");
            if index == 0 {
                first_response_sent = Some(Instant::now());
            } else if index == 1 {
                retry_delay = Some(
                    accepted_at.duration_since(first_response_sent.expect("first response sent")),
                );
                drain_until = Some(Instant::now() + observe_after_expected);
            }
        }
    })
}

fn start_retry_after_wait_cancellation_fixture() -> (
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    thread::JoinHandle<Vec<String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make cancellation fixture accept deadline bounded");
    let (response_sent_sender, response_sent_receiver) = mpsc::channel();
    let (stop_sender, stop_receiver) = mpsc::channel();
    let fixture = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let (mut first_stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "CLI did not reach rate-limit fixture"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept first rate-limit request: {error}"),
            }
        };
        let mut requests = vec![read_cli_http_request(&mut first_stream)];
        let first_body = br#"{"message":"synthetic-cancelled-retry-body-must-not-leak"}"#;
        let first_response = format!(
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: {}\r\nRetry-After: 5\r\nConnection: close\r\n\r\n",
            first_body.len()
        );
        first_stream
            .write_all(first_response.as_bytes())
            .expect("write cancellation fixture 429 headers");
        first_stream
            .write_all(first_body)
            .expect("write cancellation fixture 429 body");
        drop(first_stream);
        response_sent_sender
            .send(())
            .expect("test is waiting for the complete 429 response");

        let mut stopping_at = None;
        loop {
            if stopping_at.is_none() {
                match stop_receiver.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                        stopping_at = Some(Instant::now() + Duration::from_millis(300));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if stopping_at.is_some_and(|until| Instant::now() >= until) {
                return requests;
            }
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "cancellation fixture exceeded deadline"
                    );
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("accept request before or after cancellation: {error}"),
            };
            requests.push(read_cli_http_request(&mut stream));
            let body = serde_json::to_vec(&json!({
                "number": ISSUE_NUMBER,
                "state": "open",
                "updated_at": "2026-10-27T00:00:00Z",
                "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
            }))
            .expect("serialize unexpected retry response");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write unexpected retry response headers");
            stream
                .write_all(&body)
                .expect("write unexpected retry response body");
        }
    });
    (response_sent_receiver, stop_sender, fixture)
}

fn start_scripted_cli_http_fixture_until_finished(
    responses: Vec<(u16, Value, Vec<(String, String)>)>,
) -> (mpsc::Sender<()>, thread::JoinHandle<Vec<String>>) {
    assert!(!responses.is_empty(), "fixture needs at least one response");
    let (finished_sender, finished_receiver) = mpsc::channel();
    let fixture = start_scripted_cli_http_fixture_inner(
        responses,
        ScriptedFixtureCompletion::UntilSignalled(finished_receiver),
    );
    (finished_sender, fixture)
}

enum ScriptedFixtureCompletion {
    QuietWindow(Duration),
    UntilSignalled(mpsc::Receiver<()>),
}

fn start_scripted_cli_http_fixture_inner(
    responses: Vec<(u16, Value, Vec<(String, String)>)>,
    completion: ScriptedFixtureCompletion,
) -> thread::JoinHandle<Vec<String>> {
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make scripted fixture accept deadline bounded");

    thread::spawn(move || {
        let expected_requests = responses.len();
        let deadline = Instant::now() + Duration::from_secs(10);
        let drain_after_finish = Duration::from_millis(100);
        let fallback_response = (
            429,
            json!({"message":"unexpected retry response"}),
            vec![("Retry-After".to_owned(), "3600".to_owned())],
        );
        let mut requests = Vec::with_capacity(expected_requests);
        let mut observe_until = None;
        let mut finished = false;
        let mut drain_until = None;
        loop {
            match &completion {
                ScriptedFixtureCompletion::QuietWindow(window) => {
                    if requests.len() >= expected_requests {
                        let until = observe_until.get_or_insert_with(|| Instant::now() + *window);
                        if Instant::now() >= *until {
                            return requests;
                        }
                    }
                }
                ScriptedFixtureCompletion::UntilSignalled(receiver) => {
                    if !finished {
                        finished = match receiver.try_recv() {
                            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => true,
                            Err(mpsc::TryRecvError::Empty) => false,
                        };
                        if finished {
                            drain_until = Some(Instant::now() + drain_after_finish);
                        }
                    }
                    if finished && drain_until.is_some_and(|until| Instant::now() >= until) {
                        return requests;
                    }
                }
            }

            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        panic!("CLI made too few API requests or fixture was not stopped");
                    }
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("accept scripted reconciliation request: {error}"),
            };
            let request = read_cli_http_request(&mut stream);
            let index = requests.len();
            requests.push(request);

            // An unexpected retry gets a bounded retryable response so the
            // CLI can exit and the test can report the extra request.
            let (status, body, headers) = responses.get(index).unwrap_or(&fallback_response);
            let reason = match *status {
                200 => "OK",
                429 => "Too Many Requests",
                503 => "Service Unavailable",
                _ => "Fixture",
            };
            let body = serde_json::to_vec(body).expect("serialize scripted fixture response");
            let extra_headers = headers
                .iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>();
            let response = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
                status,
                reason,
                body.len(),
                extra_headers,
            );
            stream
                .write_all(response.as_bytes())
                .expect("write scripted fixture response headers");
            stream
                .write_all(&body)
                .expect("write scripted fixture response body");
        }
    })
}

fn start_authenticated_cli_http_fixture_until_finished(
    response_body: Value,
    expected_authorization: String,
) -> (mpsc::Sender<()>, thread::JoinHandle<Vec<(String, bool)>>) {
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make authenticated fixture accept deadline bounded");
    let body = serde_json::to_vec(&response_body).expect("serialize HTTP response fixture");
    let (finished_sender, finished_receiver) = mpsc::channel();

    let fixture = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut finished = false;
        let mut requests = Vec::new();
        loop {
            if !finished {
                finished = match finished_receiver.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => true,
                    Err(mpsc::TryRecvError::Empty) => false,
                };
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_cli_http_request(&mut stream);
                    let authorized = request.lines().any(|line| {
                        line.split_once(':').is_some_and(|(name, value)| {
                            name.eq_ignore_ascii_case("authorization")
                                && value.trim() == expected_authorization
                        })
                    });
                    let (status, reason) = if authorized {
                        (200, "OK")
                    } else {
                        (401, "Unauthorized")
                    };
                    let response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write authenticated fixture response headers");
                    stream
                        .write_all(&body)
                        .expect("write authenticated fixture response body");
                    requests.push((request, authorized));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if finished {
                        return requests;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "reconciliation CLI did not finish before fixture deadline"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("accept authenticated reconciliation request: {error}"),
            }
        }
    });
    (finished_sender, fixture)
}

fn start_gated_cli_http_fixture() -> (
    thread::JoinHandle<String>,
    mpsc::Receiver<String>,
    mpsc::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make gated fixture accept deadline bounded");
    let (request_sender, request_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let fixture = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "interrupted CLI made no API request"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept interrupted CLI request: {error}"),
            }
        };
        let request = read_cli_http_request(&mut stream);
        request_sender
            .send(request.clone())
            .expect("test is waiting for the intercepted API request");
        // The test controls this barrier after SIGTERM. A dropped sender also
        // releases it during test unwinding, without racing curl's 5s timeout.
        let _ = release_receiver.recv();
        drop(stream);
        request
    });
    (fixture, request_receiver, release_sender)
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_receiver_journal_against_private_http_fixture() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize stale synthetic webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, expected_sequence) in [("cli-process-41-a", 1), ("cli-process-41-b", 2)] {
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence);
    }
    assert!(stop_receiver(&mut receiver).success());

    let (mut recovered_receiver, recovered_address) = start_ready_receiver(&secret, &journal_path);
    let (status, duplicate) = send(recovered_address, &request("cli-process-41-a", &body));
    assert_eq!(status, 202);
    assert_eq!(duplicate["duplicate"], true);
    assert!(stop_receiver(&mut recovered_receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41",
            "labels": [
                {"id": 20, "name": "area/coordination"},
                {"id": 3, "name": "priority/high"}
            ]
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-process-41-a")
        .arg("cli-process-41-b")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n"),
        "unexpected request path: {requests:?}"
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for (result, delivery_id) in results.iter().zip(["cli-process-41-a", "cli-process-41-b"]) {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["canonical_state"]["state"], "open");
        assert_eq!(
            result["canonical_state"]["html_url"],
            "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41"
        );
        assert_eq!(
            result["canonical_state"]["labels"],
            json!({
                "items": [
                    {"id": 3, "name": "priority/high"},
                    {"id": 20, "name": "area/coordination"}
                ],
                "source": "current_forgejo_resource_api",
                "freshness": "current_api_read",
                "trust": "untrusted_user_content"
            })
        );
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_projects_current_issue_assignees_from_api() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let deliveries = [
        (
            41,
            "cli-assignee-current",
            json!([{"id": 99, "login": "stale-webhook-assignee"}]),
        ),
        (
            42,
            "cli-assignee-unknown",
            json!([{"id": 98, "login": "stale-webhook-assignee-unknown"}]),
        ),
        (
            43,
            "cli-assignee-empty",
            json!([{"id": 97, "login": "stale-webhook-assignee-empty"}]),
        ),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (index, (issue_number, delivery_id, stale_assignees)) in deliveries.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "edited",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {
                "number": issue_number,
                "state": "open",
                "assignees": stale_assignees
            }
        }))
        .expect("serialize signed issue webhook with stale assignee payload");
        let (status, accepted) = send(address, &request_event(delivery_id, "issues", &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], index + 1);
    }
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    assert_eq!(journal.replay().unwrap().len(), deliveries.len());
    let journal_before_cli = fs::read(&journal_path).expect("snapshot durable webhook journal");

    let api_responses = [
        json!({
            "number": 41,
            "state": "open",
            "updated_at": "2026-10-28T12:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/41"),
            "assignees": [
                {"id": 20, "login": "reviewer-b"},
                {"id": 3, "login": "reviewer-a"}
            ]
        }),
        json!({
            "number": 42,
            "state": "open",
            "updated_at": "2026-10-28T12:01:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/42"),
            "assignees": null
        }),
        json!({
            "number": 43,
            "state": "open",
            "updated_at": "2026-10-28T12:02:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/43"),
            "assignees": []
        }),
    ];
    let fixture_responses = api_responses
        .into_iter()
        .map(|response| (200, response, Vec::new()))
        .collect();
    let (finish_fixture, fixture) =
        start_scripted_cli_http_fixture_until_finished(fixture_responses);
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(_, delivery_id, _)| delivery_id))
        .stdin(Stdio::null());
    let output = command
        .output()
        .expect("run actual reconciliation CLI process");
    let _ = finish_fixture.send(());
    let requests = fixture.join().expect("assignee API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), deliveries.len());
    for (request, (issue_number, _, _)) in requests.iter().zip(deliveries.iter()) {
        assert!(
            request.starts_with(&format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{issue_number} HTTP/1.1\r\n"
            )),
            "unexpected API request path: {request:?}"
        );
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "the anonymous local CLI must not send an authorization header"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read journal after reconciliation"),
        journal_before_cli,
        "read-only reconciliation must not rewrite the webhook journal"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], deliveries.len());
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), deliveries.len());
    let expected_assignees = [
        json!({
            "items": [
                {"id": 3, "login": "reviewer-a"},
                {"id": 20, "login": "reviewer-b"}
            ],
            "source": "current_forgejo_resource_api",
            "freshness": "current_api_read",
            "trust": "untrusted_forgejo_identity"
        }),
        Value::Null,
        json!({
            "items": [],
            "source": "current_forgejo_resource_api",
            "freshness": "current_api_read",
            "trust": "untrusted_forgejo_identity"
        }),
    ];
    for ((result, (issue_number, delivery_id, _)), expected) in results
        .iter()
        .zip(deliveries.iter())
        .zip(expected_assignees.iter())
    {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["delivery_id"], *delivery_id);
        assert_eq!(result["resource"]["number"], json!(*issue_number));
        assert_eq!(result["canonical_state"]["assignees"], *expected);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("stale-webhook-assignee"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_projects_current_issue_milestone_from_api() {
    const NUMBER: u64 = 48;
    const DELIVERY_ID: &str = "cli-issue-milestone-48";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "milestoned",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": NUMBER,
            "state": "open",
            "milestone": {"id": 99, "title": "stale-webhook-milestone", "state": "closed"}
        }
    }))
    .expect("serialize signed issue webhook with stale milestone payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issues", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot durable webhook journal");

    let current_milestone = json!({"id": 13, "title": "Release 1", "state": "open"});
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "updated_at": "2026-10-28T14:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{NUMBER}"),
            "milestone": current_milestone
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual issue reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("issue milestone API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous reconciliation CLI must not send an authorization header"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read journal after reconciliation"),
        journal_before_cli,
        "read-only reconciliation must not rewrite accepted webhook deliveries"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(
        value["canonical_state"]["milestone"],
        json!({
            "observation": "assigned",
            "value": current_milestone,
            "source": "current_forgejo_resource_api",
            "freshness": "current_api_read",
            "trust": "untrusted_user_content"
        })
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("stale-webhook-milestone"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_projects_current_pull_request_milestone_from_api() {
    const NUMBER: u64 = 49;
    const DELIVERY_ID: &str = "cli-pr-milestone-49";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "milestoned",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "milestone": {"id": 99, "title": "stale-pr-webhook-milestone", "state": "closed"}
        }
    }))
    .expect("serialize signed pull-request webhook with stale milestone payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    assert_eq!(journal.replay().unwrap().len(), 1);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot pull-request journal");
    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let current_milestone = json!({"id": 13, "title": "PR Release 1", "state": "open"});
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-28T14:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base},
            "milestone": current_milestone
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual pull-request reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("pull-request milestone API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read pull-request journal after reconciliation"),
        journal_before_cli,
        "read-only reconciliation must not rewrite the pull-request journal"
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        value["canonical_state"]["milestone"],
        json!({
            "observation": "assigned",
            "value": current_milestone,
            "source": "current_forgejo_resource_api",
            "freshness": "current_api_read",
            "trust": "untrusted_user_content"
        })
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("stale-pr-webhook-milestone"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_projects_current_pull_request_labels_for_batch() {
    const NUMBER: u64 = 54;
    const FIRST_DELIVERY_ID: &str = "cli-pr-label-update-first-54";
    const SECOND_DELIVERY_ID: &str = "cli-pr-label-update-second-54";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let first_body = serde_json::to_vec(&json!({
        "action": "label_updated",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "labels": [{"id": 99, "name": "stale/first-label"}]
        },
        "label": {"id": 99, "name": "stale/first-label"}
    }))
    .expect("serialize signed first PR label event");
    let second_body = serde_json::to_vec(&json!({
        "action": "label_updated",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "cccccccccccccccccccccccccccccccccccccccc"},
            "base": {"sha": "dddddddddddddddddddddddddddddddddddddddd"},
            "labels": [{"id": 3, "name": "stale/second-label"}]
        },
        "label": {"id": 3, "name": "stale/second-label"}
    }))
    .expect("serialize signed second PR label event");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, body, expected_sequence) in [
        (FIRST_DELIVERY_ID, first_body, 1),
        (SECOND_DELIVERY_ID, second_body, 2),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence);
    }
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot PR label event batch");

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let current_labels = json!([
        {"id": 20, "name": "area/coordination"},
        {"id": 3, "name": "priority/high"}
    ]);
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-28T17:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base},
            "labels": current_labels
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(FIRST_DELIVERY_ID)
        .arg(SECOND_DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual PR label batch reconciliation CLI process");
    let requests = fixture.join().expect("PR label API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.len(),
        1,
        "same-PR deliveries share one current-state GET"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local batch CLI must not send an authorization header"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read PR label journal after reconciliation"),
        journal_before_cli,
        "read-only reconciliation must not rewrite accepted deliveries"
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("PR batch returns JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    let expected_labels = json!({
        "items": [
            {"id": 3, "name": "priority/high"},
            {"id": 20, "name": "area/coordination"}
        ],
        "source": "current_forgejo_resource_api",
        "freshness": "current_api_read",
        "trust": "untrusted_user_content"
    });
    for (result, delivery_id) in results.iter().zip([FIRST_DELIVERY_ID, SECOND_DELIVERY_ID]) {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["resource"]["kind"], "pull_request");
        assert_eq!(result["resource"]["number"], NUMBER);
        assert_eq!(result["canonical_state"]["head_sha"], current_head);
        assert_eq!(result["canonical_state"]["base_sha"], current_base);
        assert_eq!(result["canonical_state"]["labels"], expected_labels);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("stale/first-label"));
    assert!(!stdout.contains("stale/second-label"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_malformed_current_pull_request_labels() {
    const NUMBER: u64 = 55;
    const DELIVERY_ID: &str = "cli-pr-malformed-label-55";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "label_updated",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "labels": [{"id": 99, "name": "stale-pr-label"}]
        },
        "label": {"id": 99, "name": "stale-pr-label"}
    }))
    .expect("serialize signed PR label notification with an old snapshot");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal_before_cli = fs::read(&journal_path).expect("snapshot accepted PR label event");
    let response = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "updated_at": "2026-10-28T17:30:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {"sha": "2222222222222222222222222222222222222222"},
        "labels": [{"id": 0, "name": "invalid-current-pr-label"}]
    });
    let (finish_fixture, fixture) =
        start_scripted_cli_http_fixture_until_finished(vec![(200, response, Vec::new())]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the CLI against malformed current PR labels");
    let _ = finish_fixture.send(());
    let requests = fixture
        .join()
        .expect("malformed PR label fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("malformed current PR labels produce structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "labels");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert!(value.get("canonical_state").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("stale-pr-label"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("invalid-current-pr-label"));
    assert_eq!(
        fs::read(&journal_path).expect("reread journal after malformed current PR labels"),
        journal_before_cli,
        "malformed API labels must not rewrite the accepted event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_projects_current_unassigned_pull_request_milestone_for_batch() {
    const NUMBER: u64 = 50;
    const ASSIGN_DELIVERY_ID: &str = "cli-pr-milestone-assigned-50";
    const CLEAR_DELIVERY_ID: &str = "cli-pr-milestone-cleared-50";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let assigned_body = serde_json::to_vec(&json!({
        "action": "milestoned",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "milestone": {"id": 13, "title": "historical PR milestone", "state": "open"}
        }
    }))
    .expect("serialize signed pull-request milestoned event");
    let cleared_body = serde_json::to_vec(&json!({
        "action": "demilestoned",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "milestone": null
        }
    }))
    .expect("serialize signed pull-request demilestoned event with explicit null");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, body, expected_sequence) in [
        (ASSIGN_DELIVERY_ID, assigned_body, 1),
        (CLEAR_DELIVERY_ID, cleared_body, 2),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence);
    }
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot PR milestone event batch");

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-28T16:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base},
            "milestone": null
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(ASSIGN_DELIVERY_ID)
        .arg(CLEAR_DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual pull-request milestone batch reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("pull-request demilestone API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.len(),
        1,
        "same-PR deliveries share one current-state GET"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous batch CLI must not send an authorization header"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read PR milestone journal after reconciliation"),
        journal_before_cli,
        "read-only batch reconciliation must not rewrite accepted deliveries"
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("PR batch returns JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for (result, delivery_id) in results.iter().zip([ASSIGN_DELIVERY_ID, CLEAR_DELIVERY_ID]) {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["resource"]["kind"], "pull_request");
        assert_eq!(result["resource"]["number"], NUMBER);
        assert_eq!(result["canonical_state"]["head_sha"], current_head);
        assert_eq!(result["canonical_state"]["base_sha"], current_base);
        assert_eq!(
            result["canonical_state"]["milestone"],
            json!({
                "observation": "unassigned",
                "value": null,
                "source": "current_forgejo_resource_api",
                "freshness": "current_api_read",
                "trust": "untrusted_user_content"
            })
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("historical PR milestone"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_distinguishes_unknown_and_unassigned_pull_request_milestone_in_batch()
 {
    const UNKNOWN_NUMBER: u64 = 52;
    const UNASSIGNED_NUMBER: u64 = 53;
    const UNKNOWN_DELIVERY_ID: &str = "cli-pr-milestone-unknown-52";
    const UNASSIGNED_DELIVERY_ID: &str = "cli-pr-milestone-unassigned-53";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let unknown_body = serde_json::to_vec(&json!({
        "action": "milestoned",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": UNKNOWN_NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "milestone": {"id": 99, "title": "stale-unknown-pr-milestone", "state": "closed"}
        }
    }))
    .expect("serialize signed PR milestone event for an unknown current observation");
    let unassigned_body = serde_json::to_vec(&json!({
        "action": "demilestoned",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": UNASSIGNED_NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "cccccccccccccccccccccccccccccccccccccccc"},
            "base": {"sha": "dddddddddddddddddddddddddddddddddddddddd"},
            "milestone": null
        }
    }))
    .expect("serialize signed PR demilestone event for a known empty observation");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, body, sequence) in [
        (UNKNOWN_DELIVERY_ID, unknown_body, 1),
        (UNASSIGNED_DELIVERY_ID, unassigned_body, 2),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence);
    }
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot PR milestone observations");

    let current_unknown = json!({
        "number": UNKNOWN_NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "updated_at": "2026-10-28T16:20:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{UNKNOWN_NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {"sha": "2222222222222222222222222222222222222222"}
    });
    let current_unassigned = json!({
        "number": UNASSIGNED_NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "updated_at": "2026-10-28T16:21:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{UNASSIGNED_NUMBER}"),
        "head": {"sha": "3333333333333333333333333333333333333333"},
        "base": {"sha": "4444444444444444444444444444444444444444"},
        "milestone": null
    });
    let fixture_responses = [current_unknown, current_unassigned]
        .into_iter()
        .map(|response| (200, response, Vec::new()))
        .collect();
    let (finish_fixture, fixture) =
        start_scripted_cli_http_fixture_until_finished(fixture_responses);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(UNKNOWN_DELIVERY_ID)
        .arg(UNASSIGNED_DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual CLI against absent and null current milestones");
    let _ = finish_fixture.send(());
    let requests = fixture
        .join()
        .expect("PR milestone observation fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 2);
    for (request, number) in requests.iter().zip([UNKNOWN_NUMBER, UNASSIGNED_NUMBER]) {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{number} HTTP/1.1\r\n"
        )));
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "anonymous reconciliation must not send an authorization header"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("reread PR milestone journal after reconciliation"),
        journal_before_cli,
        "read-only reconciliation must not rewrite accepted events"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed PR milestone batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for (result, delivery_id, number, observation) in [
        (&results[0], UNKNOWN_DELIVERY_ID, UNKNOWN_NUMBER, "unknown"),
        (
            &results[1],
            UNASSIGNED_DELIVERY_ID,
            UNASSIGNED_NUMBER,
            "unassigned",
        ),
    ] {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["resource"]["kind"], "pull_request");
        assert_eq!(result["resource"]["number"], number);
        assert_eq!(
            result["canonical_state"]["milestone"],
            json!({
                "observation": observation,
                "value": null,
                "source": "current_forgejo_resource_api",
                "freshness": "current_api_read",
                "trust": "untrusted_user_content"
            })
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("stale-unknown-pr-milestone"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_projects_current_pull_request_assignees_from_api() {
    const NUMBER: u64 = 47;
    const DELIVERY_ID: &str = "cli-pr-assignees-47";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "assignees": [{"id": 99, "login": "stale-webhook-pr-assignee"}]
        }
    }))
    .expect("serialize signed pull-request webhook with stale assignee payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    assert_eq!(journal.replay().unwrap().len(), 1);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot pull-request journal");
    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let response = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "updated_at": "2026-10-28T13:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": current_head},
        "base": {"sha": current_base},
        "assignees": [
            {"id": 20, "login": "reviewer-b"},
            {"id": 3, "login": "reviewer-a"}
        ]
    });
    let (finish_fixture, fixture) =
        start_scripted_cli_http_fixture_until_finished(vec![(200, response, Vec::new())]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual pull-request reconciliation CLI process");
    let _ = finish_fixture.send(());
    let requests = fixture
        .join()
        .expect("pull-request assignee API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read pull-request journal after reconciliation"),
        journal_before_cli,
        "read-only reconciliation must not rewrite the pull-request journal"
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        value["canonical_state"]["assignees"],
        json!({
            "items": [
                {"id": 3, "login": "reviewer-a"},
                {"id": 20, "login": "reviewer-b"}
            ],
            "source": "current_forgejo_resource_api",
            "freshness": "current_api_read",
            "trust": "untrusted_forgejo_identity"
        })
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("stale-webhook-pr-assignee"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_distinguishes_unknown_and_empty_pull_request_assignees() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let deliveries = [
        (
            49,
            "cli-pr-assignees-unknown-49",
            "stale-webhook-pr-assignee-unknown",
        ),
        (
            50,
            "cli-pr-assignees-empty-50",
            "stale-webhook-pr-assignee-empty",
        ),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (index, (number, delivery_id, stale_login)) in deliveries.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "synchronized",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "pull_request": {
                "number": number,
                "state": "open",
                "merged": false,
                "draft": false,
                "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                "assignees": [{"id": 99, "login": stale_login}]
            }
        }))
        .expect("serialize signed PR webhook with a stale assignee snapshot");
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], index + 1);
    }
    assert!(stop_receiver(&mut receiver).success());

    let journal_before_cli = fs::read(&journal_path).expect("snapshot durable PR journal");
    let responses = [
        json!({
            "number": 49,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-28T15:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/49"),
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {"sha": "2222222222222222222222222222222222222222"},
            "assignees": null
        }),
        json!({
            "number": 50,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-28T15:01:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/50"),
            "head": {"sha": "3333333333333333333333333333333333333333"},
            "base": {"sha": "4444444444444444444444444444444444444444"},
            "assignees": []
        }),
    ];
    let fixture_responses = responses
        .into_iter()
        .map(|response| (200, response, Vec::new()))
        .collect();
    let (finish_fixture, fixture) =
        start_scripted_cli_http_fixture_until_finished(fixture_responses);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(_, delivery_id, _)| delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run the actual PR reconciliation batch process");
    let _ = finish_fixture.send(());
    let requests = fixture.join().expect("PR assignee fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), deliveries.len());
    for (request, (number, _, _)) in requests.iter().zip(deliveries.iter()) {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{number} HTTP/1.1\r\n"
        )));
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "anonymous reconciliation must not send an authorization header"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read PR journal after reconciliation"),
        journal_before_cli,
        "read-only reconciliation must not rewrite the accepted PR events"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("PR batch returns structured reconciliation JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], deliveries.len());
    let results = value["results"].as_array().expect("batch result array");
    assert_eq!(results.len(), deliveries.len());
    assert_eq!(results[0]["delivery_id"], deliveries[0].1);
    assert_eq!(results[0]["canonical_state"]["assignees"], Value::Null);
    assert_eq!(results[1]["delivery_id"], deliveries[1].1);
    assert_eq!(
        results[1]["canonical_state"]["assignees"],
        json!({
            "items": [],
            "source": "current_forgejo_resource_api",
            "freshness": "current_api_read",
            "trust": "untrusted_forgejo_identity"
        })
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("stale-webhook-pr-assignee"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_malformed_current_pull_request_assignees() {
    const NUMBER: u64 = 48;
    const DELIVERY_ID: &str = "cli-pr-malformed-assignees-48";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "assignees": [{"id": 99, "login": "stale-webhook-pr-assignee"}]
        }
    }))
    .expect("serialize pull-request webhook with a stale assignee snapshot");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let journal_before_cli = fs::read(&journal_path).expect("snapshot accepted PR journal");
    let response = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "updated_at": "2026-10-28T14:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {"sha": "2222222222222222222222222222222222222222"},
        "assignees": [
            {"id": 7, "login": "reviewer-a"},
            {"id": 7, "login": "reviewer-b"}
        ]
    });
    let (finish_fixture, fixture) =
        start_scripted_cli_http_fixture_until_finished(vec![(200, response, Vec::new())]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual CLI against a malformed current PR snapshot");
    let _ = finish_fixture.send(());
    let requests = fixture
        .join()
        .expect("malformed PR assignee fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("malformed current PR assignees produce structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "assignees");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert!(value.get("canonical_state").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("stale-webhook-pr-assignee"));
    assert_eq!(
        fs::read(&journal_path).expect("reread journal after malformed current PR snapshot"),
        journal_before_cli,
        "malformed current API state must leave the accepted event unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_malformed_current_pull_request_milestone() {
    const NUMBER: u64 = 51;
    const DELIVERY_ID: &str = "cli-pr-malformed-milestone-51";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "milestoned",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
            "milestone": {"id": 99, "title": "stale-pr-milestone", "state": "closed"}
        }
    }))
    .expect("serialize pull-request webhook with an old milestone snapshot");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal_before_cli = fs::read(&journal_path).expect("snapshot accepted PR milestone event");
    let response = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "updated_at": "2026-10-28T14:30:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {"sha": "2222222222222222222222222222222222222222"},
        "milestone": {"id": 0, "title": "invalid-current-pr-milestone", "state": "open"}
    });
    let (finish_fixture, fixture) =
        start_scripted_cli_http_fixture_until_finished(vec![(200, response, Vec::new())]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the CLI against a malformed current PR milestone");
    let _ = finish_fixture.send(());
    let requests = fixture
        .join()
        .expect("malformed PR milestone fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("malformed current PR milestone produces structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "milestone");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert!(value.get("canonical_state").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("stale-pr-milestone"));
    assert_eq!(
        fs::read(&journal_path).expect("reread journal after malformed current PR milestone"),
        journal_before_cli,
        "malformed current API state must leave the accepted event unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture verifies unknown event never reaches Forgejo"]
fn actual_reconcile_cli_process_refuses_persisted_unknown_workflow_action_before_api_read() {
    const DELIVERY_ID: &str = "workflow-future-action-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize supported workflow-run delivery");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "workflow_run_success", &body),
    );
    assert_eq!(
        status, 202,
        "supported workflow event is admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    // Simulate a hash-chain-valid persisted row whose action is outside the
    // currently published contract. The CLI must validate it again before API I/O.
    let journal = EventJournal::open(&journal_path);
    let mut events = journal.replay().expect("replay accepted workflow event");
    assert_eq!(events.len(), 1);
    events[0].payload["payload"]["action"] = Value::String("future_workflow_action".into());
    let hash_material = json!({
        "sequence": events[0].sequence,
        "event_id": events[0].event_id,
        "idempotency_key": events[0].idempotency_key,
        "kind": events[0].kind,
        "payload": events[0].payload,
        "previous_hash": events[0].previous_hash
    });
    events[0].hash =
        sha256(&serde_json::to_vec(&hash_material).expect("serialize mutated row hash material"));
    let mut journal_bytes = serde_json::to_vec(&events[0]).expect("serialize mutated event row");
    journal_bytes.push(b'\n');
    fs::write(&journal_path, &journal_bytes).expect("write hash-chain-valid future action row");
    let replayed = journal
        .replay()
        .expect("future action row retains a valid journal hash chain");
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].kind, "forgejo.webhook.v1.workflow_run_success");
    let journal_before_cli = fs::read(&journal_path).expect("snapshot future-action journal");

    let api_listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private sandbox reserves the configured Forgejo endpoint");
    api_listener
        .set_nonblocking(true)
        .expect("make the no-request observation nonblocking");
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI against persisted future action");

    assert!(
        !output.status.success(),
        "persisted future action must be refused"
    );
    assert!(
        matches!(api_listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "persisted future action must not open a Forgejo connection"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("CLI refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "invalid_envelope");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field_path"], "payload.action");
    assert_eq!(
        value["error"]["details"]["persistence_state"],
        "already_persisted"
    );
    assert_eq!(value["error"]["details"]["source_hmac_persisted"], false);
    assert_eq!(value["error"]["details"]["source_hmac_reverified"], false);
    assert!(
        value["error"]["action"].as_str().is_some_and(
            |action| action.contains("do not treat replay as signature re-verification")
        ),
        "replay limitation should be explicit: {value}"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read future-action journal after CLI refusal"),
        journal_before_cli,
        "the CLI must leave the persisted row unchanged"
    );
}

fn assert_persisted_incompatible_workflow_action_refused_before_api_read(
    delivery_id: &str,
    event_name: &str,
    expected_kind: &str,
    payload: Value,
    incompatible_action: &str,
) {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&payload).expect("serialize supported workflow delivery");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, event_name, &body));
    assert_eq!(
        status, 202,
        "supported workflow event is admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    // Build a hash-chain-valid row with a known action/status mismatch. The
    // inbox does not retain its original HMAC, so the CLI must reject semantics
    // without claiming to re-verify that signature or contacting Forgejo.
    let journal = EventJournal::open(&journal_path);
    let mut events = journal.replay().expect("replay accepted workflow event");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, expected_kind);
    assert_eq!(events[0].payload["event"], event_name);
    events[0].payload["payload"]["action"] = Value::String(incompatible_action.into());
    let hash_material = json!({
        "sequence": events[0].sequence,
        "event_id": events[0].event_id,
        "idempotency_key": events[0].idempotency_key,
        "kind": events[0].kind,
        "payload": events[0].payload,
        "previous_hash": events[0].previous_hash
    });
    events[0].hash =
        sha256(&serde_json::to_vec(&hash_material).expect("serialize mutated row hash material"));
    let mut journal_bytes = serde_json::to_vec(&events[0]).expect("serialize mutated event row");
    journal_bytes.push(b'\n');
    fs::write(&journal_path, &journal_bytes)
        .expect("write hash-chain-valid incompatible action row");
    let replayed = journal
        .replay()
        .expect("incompatible action row retains a valid journal hash chain");
    assert_eq!(replayed.len(), 1);
    assert_eq!(
        replayed[0].payload["payload"]["action"],
        incompatible_action
    );
    let journal_before_cli = fs::read(&journal_path).expect("snapshot incompatible-action journal");

    let api_listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private sandbox reserves the configured Forgejo endpoint");
    api_listener
        .set_nonblocking(true)
        .expect("make the no-request observation nonblocking");
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI against persisted incompatible action");

    assert!(
        !output.status.success(),
        "persisted incompatible action must be refused"
    );
    assert!(
        matches!(api_listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "persisted incompatible action must not open a Forgejo connection"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("CLI refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "invalid_envelope");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field_path"], "payload.action");
    assert_eq!(
        value["error"]["details"]["persistence_state"],
        "already_persisted"
    );
    assert_eq!(value["error"]["details"]["source_hmac_persisted"], false);
    assert_eq!(value["error"]["details"]["source_hmac_reverified"], false);
    assert!(
        value["error"]["action"].as_str().is_some_and(
            |action| action.contains("do not treat replay as signature re-verification")
        ),
        "replay limitation should be explicit: {value}"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read incompatible-action journal after CLI refusal"),
        journal_before_cli,
        "the CLI must leave the persisted row unchanged"
    );
}

#[test]
#[ignore = "run via scripts/run_webhook_reconcile_cli_e2e.py; fixture verifies incompatible event never reaches Forgejo"]
fn actual_reconcile_cli_process_refuses_persisted_incompatible_workflow_action_before_api_read() {
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    assert_persisted_incompatible_workflow_action_refused_before_api_read(
        "workflow-incompatible-run-action-41",
        "workflow_run_blocked",
        "forgejo.webhook.v1.workflow_run_blocked",
        json!({
            "action": "run_status_changed",
            "run": {
                "id": 7001,
                "index_in_repo": 41,
                "status": "blocked",
                "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "updated": "2026-10-26T11:00:00Z",
                "html_url": run_url,
                "repository": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }),
        "run_completed",
    );
}

#[test]
#[ignore = "run via scripts/run_webhook_reconcile_cli_e2e.py; fixture verifies incompatible event never reaches Forgejo"]
fn actual_reconcile_cli_process_refuses_persisted_incompatible_workflow_job_action_before_api_read()
{
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    assert_persisted_incompatible_workflow_action_refused_before_api_read(
        "workflow-incompatible-job-action-41",
        "workflow_job_waiting",
        "forgejo.webhook.v1.workflow_job_waiting",
        json!({
            "action": "job_status_changed",
            "job": {
                "id": 9001,
                "run_id": 7001,
                "status": "waiting",
                "html_url": format!("{run_url}/jobs/1/attempt/1")
            },
            "run": {
                "id": 7001,
                "index_in_repo": 41,
                "status": "running",
                "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "updated": "2026-10-26T11:00:00Z",
                "html_url": run_url,
                "repository": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }),
        "job_completed",
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture verifies unknown event never reaches Forgejo"]
fn actual_reconcile_cli_process_refuses_unknown_future_event_kind_before_api_read() {
    const DELIVERY_ID: &str = "future-event-kind-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize a supported issue delivery");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    // Model a hash-chain-valid inbox row written by a newer/older producer
    // whose event vocabulary differs from this CLI. The current receiver
    // correctly refuses to create this row itself.
    let journal = EventJournal::open(&journal_path);
    let mut events = journal.replay().expect("replay receiver journal");
    assert_eq!(events.len(), 1);
    events[0].kind = "forgejo.webhook.v1.future_event".into();
    let hash_material = json!({
        "sequence": events[0].sequence,
        "event_id": events[0].event_id,
        "idempotency_key": events[0].idempotency_key,
        "kind": events[0].kind,
        "payload": events[0].payload,
        "previous_hash": events[0].previous_hash
    });
    events[0].hash =
        sha256(&serde_json::to_vec(&hash_material).expect("serialize future event hash material"));
    let mut journal_bytes = serde_json::to_vec(&events[0]).expect("serialize future event row");
    journal_bytes.push(b'\n');
    fs::write(&journal_path, &journal_bytes).expect("write hash-chain-valid future event fixture");
    let replayed = journal
        .replay()
        .expect("future event retains a valid journal hash chain");
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].kind, "forgejo.webhook.v1.future_event");
    let journal_before_cli = fs::read(&journal_path).expect("snapshot future-event journal");

    let api_listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private sandbox reserves the configured Forgejo endpoint");
    api_listener
        .set_nonblocking(true)
        .expect("make the no-request observation nonblocking");
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual reconciliation CLI against a future event kind");

    assert!(
        !output.status.success(),
        "unknown persisted event kind must be refused"
    );
    assert!(
        matches!(api_listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "unknown event kind must not open a Forgejo connection"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("CLI refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "unsupported_event");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["event_contracts_capability"],
        "operations[reconcile_webhook_deliveries].event_contracts"
    );
    assert!(
        value["error"]["action"]
            .as_str()
            .is_some_and(|action| action.contains("--capabilities"))
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read future-event journal after CLI refusal"),
        journal_before_cli,
        "the CLI must leave the journal unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture verifies no legacy journal reaches the API"]
fn actual_reconcile_cli_process_refuses_persisted_review_without_action_before_api_read() {
    const DELIVERY_ID: &str = "persisted-review-action-required-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "reviewed",
        "pull_request": {"number": ISSUE_NUMBER},
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "review": {"type": "pull_request_review_approved"}
    }))
    .expect("serialize valid review event before persisted-payload mutation");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "pull_request_approved", &body),
    );
    assert_eq!(
        status, 202,
        "valid review should enter the journal: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    // Model a hash-chain-valid persisted row from a producer that omitted the
    // now-required review action. The reconciliation process must validate it
    // before contacting Forgejo, even though receiver admission was stricter.
    let journal = EventJournal::open(&journal_path);
    let mut events = journal.replay().expect("replay receiver journal");
    assert_eq!(events.len(), 1);
    events[0].payload["payload"]
        .as_object_mut()
        .expect("signed webhook body is an object")
        .remove("action");
    let hash_material = json!({
        "sequence": events[0].sequence,
        "event_id": events[0].event_id,
        "idempotency_key": events[0].idempotency_key,
        "kind": events[0].kind,
        "payload": events[0].payload,
        "previous_hash": events[0].previous_hash
    });
    events[0].hash =
        sha256(&serde_json::to_vec(&hash_material).expect("serialize replay hash material"));
    let mut record = serde_json::to_vec(&events[0]).expect("serialize persisted review row");
    record.push(b'\n');
    fs::write(&journal_path, &record).expect("write hash-chain-valid persisted review row");
    let replayed = journal
        .replay()
        .expect("mutated review row still has a valid journal hash chain");
    assert_eq!(replayed.len(), 1);
    assert!(replayed[0].payload["payload"].get("action").is_none());
    let journal_before_cli = fs::read(&journal_path).expect("snapshot persisted review journal");

    let api_listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private sandbox reserves configured Forgejo endpoint");
    api_listener
        .set_nonblocking(true)
        .expect("make no-request observation nonblocking");
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run actual reconciliation CLI on persisted review row");

    assert!(
        !output.status.success(),
        "persisted review without action must be refused"
    );
    assert!(
        matches!(api_listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "invalid persisted review must not open a Forgejo connection"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("CLI refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "invalid_envelope");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field_path"], "payload.action");
    assert_eq!(
        value["error"]["details"]["persistence_state"],
        "already_persisted"
    );
    let action = value["error"]["action"]
        .as_str()
        .expect("persisted-event refusal has actionable guidance");
    assert!(action.contains("already persisted"), "{action}");
    assert!(action.contains("do not edit"), "{action}");
    assert!(!action.contains("not journaled"), "{action}");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after CLI refusal"),
        journal_before_cli,
        "refusing the invalid row must not rewrite the append-only journal"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture verifies no legacy journal reaches the API"]
fn actual_reconcile_cli_process_rejects_legacy_issue_review_member_before_api_read() {
    const DELIVERY_ID: &str = "legacy-issue-review-41";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize an issue payload accepted by the current receiver");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let mut events = journal.replay().expect("replay valid receiver journal");
    assert_eq!(events.len(), 1);
    let mut legacy_event = events.pop().expect("one delivered issue event");
    legacy_event.payload["payload"]["review"] = Value::Null;
    let hash_material = json!({
        "sequence": legacy_event.sequence,
        "event_id": legacy_event.event_id,
        "idempotency_key": legacy_event.idempotency_key,
        "kind": legacy_event.kind,
        "payload": legacy_event.payload,
        "previous_hash": legacy_event.previous_hash
    });
    legacy_event.hash =
        sha256(&serde_json::to_vec(&hash_material).expect("serialize journal event hash material"));
    let mut legacy_record =
        serde_json::to_vec(&legacy_event).expect("serialize legacy webhook journal record");
    legacy_record.push(b'\n');
    fs::write(&journal_path, &legacy_record).expect("write legacy-format journal fixture");
    let legacy_events = journal
        .replay()
        .expect("legacy-format record has an intact journal hash chain");
    assert_eq!(legacy_events.len(), 1);
    assert_eq!(legacy_events[0].payload["payload"]["review"], Value::Null);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot legacy journal");

    let api_listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private sandbox reserves the configured Forgejo endpoint");
    api_listener
        .set_nonblocking(true)
        .expect("make the no-request observation nonblocking");
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI on the legacy journal");

    assert!(
        !output.status.success(),
        "invalid persisted payload must be refused"
    );
    assert!(
        matches!(api_listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "invalid persisted payload must not open a Forgejo connection"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("CLI refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "invalid_envelope");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field_path"], "payload.review");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read legacy journal after CLI refusal"),
        journal_before_cli,
        "the CLI must leave the journal unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_continues_batch_after_legacy_issue_review_member() {
    const CURRENT_DELIVERY_ID: &str = "mixed-current-issue-41";
    const LEGACY_DELIVERY_ID: &str = "mixed-legacy-review-42";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let current_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize current issue payload");
    let legacy_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER + 1, "state": "open"}
    }))
    .expect("serialize issue payload accepted by the previous validator");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, body) in [
        (CURRENT_DELIVERY_ID, current_body.as_slice()),
        (LEGACY_DELIVERY_ID, legacy_body.as_slice()),
    ] {
        let (status, accepted) = send(address, &request(delivery_id, body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let mut events = journal
        .replay()
        .expect("replay two accepted issue deliveries");
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[1].payload["delivery_id"], LEGACY_DELIVERY_ID,
        "the legacy-format event is the final hash-chain record"
    );
    events[1].payload["payload"]["review"] = Value::Null;
    let hash_material = json!({
        "sequence": events[1].sequence,
        "event_id": events[1].event_id,
        "idempotency_key": events[1].idempotency_key,
        "kind": events[1].kind,
        "payload": events[1].payload,
        "previous_hash": events[1].previous_hash
    });
    events[1].hash =
        sha256(&serde_json::to_vec(&hash_material).expect("serialize journal event hash material"));
    let mut journal_bytes = serde_json::to_vec(&events[0]).expect("serialize current journal row");
    journal_bytes.push(b'\n');
    journal_bytes.extend_from_slice(
        &serde_json::to_vec(&events[1]).expect("serialize legacy-format journal row"),
    );
    journal_bytes.push(b'\n');
    fs::write(&journal_path, &journal_bytes).expect("write legacy-format mixed journal");
    let replayed = journal
        .replay()
        .expect("both journal records retain a valid hash chain");
    assert_eq!(replayed.len(), 2);
    assert_eq!(replayed[1].payload["payload"]["review"], Value::Null);
    let journal_before_cli = fs::read(&journal_path).expect("snapshot mixed legacy journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-25T16:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(LEGACY_DELIVERY_ID)
        .arg(CURRENT_DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI for a mixed legacy/current batch");
    let requests = fixture.join().expect("valid issue API fixture completed");

    assert!(
        !output.status.success(),
        "one invalid event makes the batch inconclusive"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed batch refusal returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], LEGACY_DELIVERY_ID);
    assert_eq!(results[0]["verdict"], "inconclusive");
    assert_eq!(results[0]["error"]["code"], "invalid_envelope");
    assert_eq!(results[0]["error"]["retryable"], false);
    assert_eq!(
        results[0]["error"]["details"]["field_path"],
        "payload.review"
    );
    assert!(results[0].get("canonical_state").is_none());
    assert_eq!(results[1]["delivery_id"], CURRENT_DELIVERY_ID);
    assert_eq!(results[1]["verdict"], "current_state_read");
    assert_eq!(results[1]["canonical_state"]["state"], "open");
    assert_eq!(results[1]["resource"]["number"], ISSUE_NUMBER);
    assert_eq!(
        fs::read(&journal_path).expect("re-read mixed journal after CLI reconciliation"),
        journal_before_cli,
        "reconciliation must not rewrite either durable delivery"
    );
}

fn assert_current_workflow_unknown_status_refused_before_projection(
    delivery_id: &str,
    event_name: &str,
    event_body: Value,
    api_body: Value,
    api_path: &str,
) {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(
            delivery_id,
            event_name,
            &serde_json::to_vec(&event_body).unwrap(),
        ),
    );
    assert_eq!(status, 202, "valid webhook admission: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot admitted webhook journal");

    let fixture = start_cli_http_fixture(200, api_body, 1);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("current workflow API fixture completed");

    assert!(
        !output.status.success(),
        "unknown current status is not success"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!("GET {api_path} HTTP/1.1\r\n")));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "public fixture read does not need an Authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("unknown workflow status returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "status");
    assert!(value.get("canonical_state").is_none());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("workflow-current-status-event-marker"));
    assert!(!output_text.contains("workflow-current-status-api-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read admitted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_unknown_current_workflow_run_status() {
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let repository = json!({
        "full_name": format!("{OWNER}/{REPOSITORY}"),
        "name": REPOSITORY,
        "owner": {"login": OWNER}
    });
    let event_body = json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "workflow-current-status-event-marker",
            "repository": repository.clone()
        }
    });
    let api_body = json!({
        "id": 7001,
        "index_in_repo": 41,
        "status": "unknown",
        "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "updated": "2026-10-26T11:05:00Z",
        "html_url": run_url,
        "event_payload": "workflow-current-status-api-marker",
        "repository": repository.clone()
    });

    assert_current_workflow_unknown_status_refused_before_projection(
        "workflow-run-unknown-current-status",
        "workflow_run_success",
        event_body,
        api_body,
        &format!("/api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001"),
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_unknown_current_workflow_job_status_before_parent_read() {
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let job_url = format!("{run_url}/jobs/0/attempt/1");
    let repository = json!({
        "full_name": format!("{OWNER}/{REPOSITORY}"),
        "name": REPOSITORY,
        "owner": {"login": OWNER}
    });
    let event_body = json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "success",
            "html_url": job_url
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "running",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": repository.clone(),
            "event_payload": "workflow-current-status-event-marker"
        }
    });
    let api_body = json!({
        "id": 9001,
        "run_id": 7001,
        "status": "unknown",
        "html_url": job_url,
        "event_payload": "workflow-current-status-api-marker"
    });

    assert_current_workflow_unknown_status_refused_before_projection(
        "workflow-job-unknown-current-status",
        "workflow_job_success",
        event_body,
        api_body,
        &format!("/api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001"),
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_unknown_parent_run_status_after_valid_job_readback() {
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let (result, transcript, _) = run_workflow_job_parent_readback_refusal(
        "workflow-job-unknown-parent-status",
        200,
        json!({
            "id": 7001,
            "index_in_repo": 41,
            "status": "unknown",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-26T11:05:00Z",
            "html_url": run_url,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            },
            "event_payload": "unknown-parent-api-marker"
        }),
    );
    assert_eq!(result["error"]["code"], "invalid_current_state");
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(result["error"]["details"]["field"], "status");
    assert!(!transcript.contains("unknown-parent-api-marker"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_missing_parent_run_after_valid_job_readback() {
    let (result, transcript, requests) = run_workflow_job_parent_readback_refusal(
        "workflow-job-missing-parent-run",
        404,
        json!({"message": "workflow-job-parent-404-private-marker"}),
    );
    assert_eq!(result["error"]["code"], "resource_not_found");
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(
        result["error"]["details"]["resource"]["kind"],
        "workflow_job"
    );
    assert_eq!(
        result["error"]["details"]["readback_resource"]["kind"],
        "workflow_run"
    );
    assert_eq!(
        result["error"]["details"]["readback_resource"]["run_id"],
        7001
    );
    assert_eq!(
        result["error"]["details"]["readback_resource"]["index_in_repo"],
        41
    );
    assert!(
        result["error"]["action"]
            .as_str()
            .is_some_and(|action| action.contains("parent workflow run"))
    );
    assert!(!transcript.contains("workflow-job-parent-404-private-marker"));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_parent_run_identity_mismatch_after_valid_job_readback() {
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let (result, transcript, requests) = run_workflow_job_parent_readback_refusal(
        "workflow-job-parent-identity-mismatch",
        200,
        json!({
            "id": 7002,
            "index_in_repo": 41,
            "status": "failure",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-26T11:05:00Z",
            "html_url": run_url,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            },
            "event_payload": "mismatched-parent-run-private-marker"
        }),
    );

    assert_eq!(result["error"]["code"], "resource_mismatch");
    assert_eq!(result["error"]["retryable"], false);
    assert!(!transcript.contains("mismatched-parent-run-private-marker"));
    assert_eq!(
        requests.len(),
        2,
        "the CLI reads the linked job and parent run before rejecting"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_shares_missing_parent_run_readback_across_job_batch() {
    const DELIVERY_IDS: [&str; 2] = [
        "workflow-job-parent-404-batch-first",
        "workflow-job-parent-404-batch-second",
    ];

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let job_url = format!("{run_url}/jobs/0/attempt/1");
    let event_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "success",
            "html_url": job_url
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "running",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            },
            "event_payload": "workflow-job-parent-batch-event-marker"
        }
    }))
    .expect("serialize valid signed workflow-job notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for delivery_id in DELIVERY_IDS {
        let (status, accepted) = send(
            address,
            &request_event(delivery_id, "workflow_job_success", &event_body),
        );
        assert_eq!(status, 202, "accepted {delivery_id}: {accepted}");
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal = EventJournal::open(&journal_path);
    assert_eq!(
        journal.replay().expect("replay accepted job events").len(),
        2
    );
    let journal_before = fs::read(&journal_path).expect("snapshot workflow-job journal");

    let fixture = start_cli_http_fixture_responses(vec![
        (
            200,
            json!({
                "id": 9001,
                "run_id": 7001,
                "status": "success",
                "html_url": job_url,
                "event_payload": "workflow-job-parent-batch-current-marker"
            }),
        ),
        (
            404,
            json!({"message": "workflow-job-parent-batch-404-private-marker"}),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(DELIVERY_IDS)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual batch reconciliation CLI process");
    let requests = fixture.join().expect("job batch API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        2,
        "shared job and parent each require one GET"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for marker in [
        "workflow-job-parent-batch-event-marker",
        "workflow-job-parent-batch-current-marker",
        "workflow-job-parent-batch-404-private-marker",
    ] {
        assert!(
            !transcript.contains(marker),
            "private marker leaked: {marker}"
        );
    }
    let result: Value =
        serde_json::from_slice(&output.stdout).expect("workflow-job batch returns structured JSON");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["upstream_read_attempts"], 2);
    let results = result["results"].as_array().expect("batch result array");
    assert_eq!(results.len(), DELIVERY_IDS.len());
    for (delivery, delivery_id) in results.iter().zip(DELIVERY_IDS) {
        assert_eq!(delivery["delivery_id"], delivery_id);
        assert_eq!(delivery["verdict"], "inconclusive");
        assert_eq!(delivery["error"]["code"], "resource_not_found");
        assert_eq!(delivery["error"]["retryable"], false);
        assert_eq!(
            delivery["error"]["details"]["resource"]["kind"],
            "workflow_job"
        );
        assert_eq!(
            delivery["error"]["details"]["readback_resource"]["kind"],
            "workflow_run"
        );
        assert_eq!(
            delivery["error"]["details"]["readback_resource"]["run_id"],
            7001
        );
        assert!(
            delivery["error"]["action"]
                .as_str()
                .is_some_and(|action| action.contains("parent workflow run"))
        );
        assert!(delivery.get("canonical_state").is_none());
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read workflow-job journal"),
        journal_before,
        "batch reconciliation must not rewrite accepted deliveries"
    );
}

fn run_workflow_job_parent_readback_refusal(
    delivery_id: &str,
    parent_status: u16,
    parent_response: Value,
) -> (Value, String, Vec<String>) {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let job_url = format!("{run_url}/jobs/0/attempt/1");
    let event_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "success",
            "html_url": job_url
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "running",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            },
            "event_payload": "workflow-job-parent-event-private-marker"
        }
    }))
    .expect("serialize valid signed workflow-job notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_job_success", &event_body),
    );
    assert_eq!(status, 202, "valid webhook admission: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot admitted workflow-job event");

    let fixture = start_cli_http_fixture_responses(vec![
        (
            200,
            json!({
                "id": 9001,
                "run_id": 7001,
                "status": "success",
                "html_url": job_url,
                "event_payload": "workflow-job-parent-current-job-private-marker"
            }),
        ),
        (parent_status, parent_response),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("job and parent API fixtures completed");

    assert!(!output.status.success());
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));

    let result: Value = serde_json::from_slice(&output.stdout)
        .expect("unknown parent run status returns structured refusal");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["delivery_id"], delivery_id);
    assert_eq!(result["upstream_read_attempts"], 2);
    assert!(result.get("canonical_state").is_none());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("workflow-job-parent-event-private-marker"));
    assert!(!transcript.contains("workflow-job-parent-current-job-private-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read admitted workflow-job event"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
    (result, transcript, requests)
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");

    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "event-private-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let mut invalid_event: Value =
        serde_json::from_slice(&event_body).expect("decode workflow-run event for mutation");
    invalid_event["run"]["id"] = json!(0);
    let invalid_body =
        serde_json::to_vec(&invalid_event).expect("serialize invalid workflow-run ID");
    let journal_before = fs::read(&journal_path).ok();
    let (status, refusal) = send(
        address,
        &request_event(
            "workflow-run-current-readback",
            "workflow_run_success",
            &invalid_body,
        ),
    );
    assert_eq!(status, 400);
    assert_eq!(refusal["error"]["code"], "event_payload_invalid");
    assert_eq!(refusal["error"]["retryable"], false);
    assert_eq!(refusal["error"]["details"]["field_path"], "payload.run.id");
    let action = refusal["error"]["action"]
        .as_str()
        .expect("invalid workflow-run ID has repair guidance");
    assert!(action.contains("payload.run.id"), "{action}");
    assert!(action.contains("positive JSON integer"), "{action}");
    assert!(action.contains("same delivery ID"), "{action}");
    assert!(action.contains("not journaled"), "{action}");
    assert_eq!(fs::read(&journal_path).ok(), journal_before);

    // Reuse the exact signed success body under the incompatible failure
    // header; the corrected retry changes only the unsigned header.
    let (status, refusal) = send(
        address,
        &request_event(
            "workflow-run-current-readback",
            "workflow_run_failure",
            &event_body,
        ),
    );
    assert_eq!(status, 400);
    assert_eq!(refusal["error"]["code"], "event_payload_invalid");
    assert_eq!(refusal["error"]["retryable"], false);
    assert_eq!(
        refusal["error"]["details"]["field_path"],
        "payload.run.status"
    );
    let action = refusal["error"]["action"]
        .as_str()
        .expect("mismatched workflow-run status has repair guidance");
    assert!(action.contains("payload.run.status"), "{action}");
    assert!(action.contains("X-Forgejo-Event"), "{action}");
    assert!(action.contains("workflow_run"), "{action}");
    assert!(action.contains("trusted sender"), "{action}");
    assert!(action.contains("GET /forgejo/capabilities"), "{action}");
    assert!(action.contains("same delivery ID"), "{action}");
    assert!(action.contains("original signed body"), "{action}");
    assert!(action.contains("do not alter the signed body"), "{action}");
    assert!(action.contains("not journaled"), "{action}");
    assert_eq!(fs::read(&journal_path).ok(), journal_before);

    let (status, accepted) = send(
        address,
        &request_event(
            "workflow-run-current-readback",
            "workflow_run_success",
            &event_body,
        ),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay workflow-run delivery");
    assert_eq!(durable.len(), 1);
    let journal_before = fs::read(&journal_path).expect("snapshot journal before CLI");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": 7001,
            "index_in_repo": 41,
            "status": "failure",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-26T11:05:00Z",
            "html_url": run_url,
            "event_payload": "api-private-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg("workflow-run-current-readback")
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let requests = fixture.join().expect("workflow-run API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
        )),
        "must read by Forgejo's internal run ID, observed: {}",
        requests[0]
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("workflow-run reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["delivery_id"], "workflow-run-current-readback");
    assert_eq!(value["resource"]["kind"], "workflow_run");
    assert_eq!(value["resource"]["run_id"], 7001);
    assert_eq!(value["resource"]["index_in_repo"], 41);
    assert_eq!(value["journal_source"]["sequence"], durable[0].sequence);
    assert_eq!(value["journal_source"]["event_sha256"], durable[0].hash);
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["status"],
        "verified"
    );
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["verification_scope"],
        "receiver_signature_only"
    );
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["key_id_sha256"],
        durable[0].receiver_attestation.as_ref().unwrap().key_id
    );
    assert!(
        value["journal_source"]["receiver_attestation"]
            .get("signature_hex")
            .is_none()
    );
    assert_eq!(value["canonical_state"]["state"], "failure");
    assert_eq!(
        value["canonical_state"]["updated_at"],
        "2026-10-26T11:05:00Z"
    );
    assert_eq!(
        value["canonical_state"]["head_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(value["canonical_state"]["html_url"], run_url);
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("event-private-marker"));
    assert!(!output_text.contains("api-private-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after CLI reconciliation"),
        journal_before,
        "reconciliation reads without rewriting the signed event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_current_release_by_id_and_omits_notes() {
    const DELIVERY_ID: &str = "release-current-readback";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let current_release_url =
        format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/releases/tag/v1.4.2");
    let event_body = serde_json::to_vec(&json!({
        "action": "updated",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "release": {
            "id": 610,
            "tag_name": "stale-tag",
            "name": "event-private-title-marker",
            "body": "event-private-notes-marker",
            "assets": [{"name": "event-private-asset-marker"}]
        }
    }))
    .expect("serialize signed release notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let invalid_event_body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "release": {"id": 610}
    }))
    .expect("serialize signed release event with unsupported action");
    let (status, rejection) = send(
        address,
        &request_event(DELIVERY_ID, "release", &invalid_event_body),
    );
    assert_eq!(status, 400);
    assert_eq!(rejection["error"]["code"], "event_payload_invalid");
    assert_eq!(rejection["error"]["retryable"], false);
    assert_eq!(
        rejection["error"]["details"]["field_path"],
        "payload.action"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay after unsupported release action")
            .is_empty(),
        "invalid action must not reserve the delivery ID before correction"
    );

    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "release", &event_body));
    assert_eq!(
        status, 202,
        "corrected release webhook admission should reuse the delivery ID: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay release delivery");
    assert_eq!(durable.len(), 1);
    let journal_before = fs::read(&journal_path).expect("snapshot journal before CLI");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": 610,
            "tag_name": "v1.4.2",
            "name": "api-private-title-marker",
            "body": "api-private-notes-marker",
            "draft": false,
            "prerelease": true,
            "html_url": current_release_url,
            "assets": [{"name": "api-private-asset-marker"}]
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    assert!(
        output.status.success(),
        "reconciliation CLI failed before contacting the current release API: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = fixture.join().expect("release API fixture completed");

    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/releases/610 HTTP/1.1\r\n"
        )),
        "must read the current release by its repository-scoped ID, observed: {}",
        requests[0]
    );
    assert!(
        !requests[0].to_ascii_lowercase().contains("authorization:"),
        "public release read should not receive a merge credential"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("release reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["delivery_id"], "release-current-readback");
    assert_eq!(value["resource"]["kind"], "release");
    assert_eq!(value["resource"]["release_id"], 610);
    assert_eq!(value["journal_source"]["sequence"], durable[0].sequence);
    assert_eq!(value["journal_source"]["event_sha256"], durable[0].hash);
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["status"],
        "verified"
    );
    assert_eq!(value["canonical_state"]["state"], "present");
    assert!(value["canonical_state"]["updated_at"].is_null());
    assert_eq!(value["canonical_state"]["html_url"], current_release_url);
    assert_eq!(value["canonical_state"]["release"]["id"], 610);
    assert_eq!(value["canonical_state"]["release"]["tag_name"], "v1.4.2");
    assert_eq!(value["canonical_state"]["release"]["draft"], false);
    assert_eq!(value["canonical_state"]["release"]["prerelease"], true);
    assert_eq!(
        value["canonical_state"]["release"]["source"],
        "current_forgejo_resource_api"
    );
    assert_eq!(
        value["canonical_state"]["release"]["freshness"],
        "current_api_read"
    );
    assert_eq!(
        value["canonical_state"]["release"]["trust"],
        "untrusted_user_content"
    );
    assert_eq!(
        value["canonical_state"]["release"]["release_notes_included"],
        false
    );
    assert_eq!(
        value["canonical_state"]["release"]["assets_included"],
        false
    );
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for private_marker in [
        "event-private-title-marker",
        "event-private-notes-marker",
        "event-private-asset-marker",
        "api-private-title-marker",
        "api-private-notes-marker",
        "api-private-asset-marker",
    ] {
        assert!(
            !output_text.contains(private_marker),
            "leaked {private_marker}"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after CLI reconciliation"),
        journal_before,
        "reconciliation reads without rewriting the signed release event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_deleted_release_present_when_current_api_returns_200() {
    const DELIVERY_ID: &str = "release-deleted-current-present";
    const RELEASE_ID: u64 = 612;
    const EVENT_PRIVATE_MARKER: &str = "release-delete-stale-event-private-marker";
    const API_PRIVATE_MARKER: &str = "release-delete-current-api-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event_body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "release": {
            "id": RELEASE_ID,
            "tag_name": "stale-deleted-tag",
            "body": EVENT_PRIVATE_MARKER,
            "assets": [{"name": "release-delete-event-private-asset"}]
        }
    }))
    .expect("serialize signed release deletion notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "release", &event_body));
    assert_eq!(
        status, 202,
        "valid release deletion notification should be durably admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay release deletion delivery");
    assert_eq!(durable.len(), 1);
    assert_eq!(durable[0].payload["payload"]["action"], "deleted");
    let journal_before = fs::read(&journal_path).expect("snapshot journal before readback");
    let current_release_url =
        format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/releases/tag/v1.4.4");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": RELEASE_ID,
            "tag_name": "v1.4.4",
            "body": API_PRIVATE_MARKER,
            "draft": false,
            "prerelease": false,
            "html_url": current_release_url,
            "assets": [{"name": "release-delete-api-private-asset"}]
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI for a deleted notification with a current Release");
    let requests = fixture
        .join()
        .expect("current Release API fixture completed");

    assert!(
        output.status.success(),
        "current API state must govern over a stale deleted notification: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/releases/{RELEASE_ID} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "public release read must not receive a merge credential"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("deleted Release notification with current resource returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["resource"]["kind"], "release");
    assert_eq!(value["resource"]["release_id"], RELEASE_ID);
    assert_eq!(value["journal_source"]["sequence"], durable[0].sequence);
    assert_eq!(value["journal_source"]["event_sha256"], durable[0].hash);
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["status"],
        "verified"
    );
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["verification_scope"],
        "receiver_signature_only"
    );
    assert_eq!(value["canonical_state"]["state"], "present");
    assert_eq!(value["canonical_state"]["html_url"], current_release_url);
    assert_eq!(value["canonical_state"]["release"]["id"], RELEASE_ID);
    assert_eq!(value["canonical_state"]["release"]["tag_name"], "v1.4.4");
    assert_eq!(
        value["canonical_state"]["release"]["source"],
        "current_forgejo_resource_api"
    );
    assert_eq!(
        value["canonical_state"]["release"]["freshness"],
        "current_api_read"
    );
    assert_eq!(
        value["canonical_state"]["release"]["trust"],
        "untrusted_user_content"
    );
    assert_eq!(
        value["canonical_state"]["release"]["release_notes_included"],
        false
    );
    assert_eq!(
        value["canonical_state"]["release"]["assets_included"],
        false
    );
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for private_marker in [
        EVENT_PRIVATE_MARKER,
        "release-delete-event-private-asset",
        API_PRIVATE_MARKER,
        "release-delete-api-private-asset",
    ] {
        assert!(
            !output_text.contains(private_marker),
            "leaked {private_marker}"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after CLI reconciliation"),
        journal_before,
        "readback must not rewrite the signed deletion notification"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_release_forbidden_read_inconclusive_without_projection() {
    const DELIVERY_ID: &str = "release-deleted-current-403";
    const RELEASE_ID: u64 = 613;
    const EVENT_PRIVATE_MARKER: &str = "release-forbidden-event-private-marker";
    const API_PRIVATE_MARKER: &str = "release-forbidden-api-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event_body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "release": {
            "id": RELEASE_ID,
            "tag_name": "forbidden-release-tag",
            "body": EVENT_PRIVATE_MARKER,
            "assets": [{"name": "release-forbidden-event-private-asset"}]
        }
    }))
    .expect("serialize signed release deletion notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "release", &event_body));
    assert_eq!(
        status, 202,
        "valid release deletion should be durably admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay release deletion delivery");
    assert_eq!(durable.len(), 1);
    assert_eq!(durable[0].payload["payload"]["action"], "deleted");
    let journal_before = fs::read(&journal_path).expect("snapshot journal before readback");
    let fixture = start_cli_http_fixture(403, json!({"message": API_PRIVATE_MARKER}), 1);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI for a forbidden Release readback");
    let requests = fixture.join().expect("Release 403 fixture completed");

    assert!(
        !output.status.success(),
        "403 must not be treated as a successful read"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/releases/{RELEASE_ID} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "public Release read must not receive a merge credential"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("forbidden Release read returns a structured inconclusive result");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_forbidden");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["action"],
        "inspect repository visibility and local API permissions"
    );
    assert_eq!(value["error"]["details"]["upstream"], "forgejo");
    assert_eq!(value["error"]["details"]["resource"]["kind"], "release");
    assert_eq!(value["error"]["details"]["resource"]["owner"], OWNER);
    assert_eq!(
        value["error"]["details"]["resource"]["repository"],
        REPOSITORY
    );
    assert_eq!(
        value["error"]["details"]["resource"]["release_id"],
        RELEASE_ID
    );
    assert_eq!(value["journal_source"]["sequence"], durable[0].sequence);
    assert_eq!(value["journal_source"]["event_sha256"], durable[0].hash);
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["status"],
        "verified"
    );
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["verification_scope"],
        "receiver_signature_only"
    );
    assert!(value.get("canonical_state").is_none());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for private_marker in [
        EVENT_PRIVATE_MARKER,
        "release-forbidden-event-private-asset",
        API_PRIVATE_MARKER,
    ] {
        assert!(
            !output_text.contains(private_marker),
            "untrusted content leaked: {private_marker}"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after forbidden API result"),
        journal_before,
        "a forbidden read must not mutate the signed event journal"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_release_unauthorized_read_inconclusive_without_projection() {
    const DELIVERY_ID: &str = "release-deleted-current-401";
    const RELEASE_ID: u64 = 614;
    const EVENT_PRIVATE_MARKER: &str = "release-unauthorized-event-private-marker";
    const API_PRIVATE_MARKER: &str = "release-unauthorized-api-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event_body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "release": {
            "id": RELEASE_ID,
            "tag_name": "unauthorized-release-tag",
            "body": EVENT_PRIVATE_MARKER,
            "assets": [{"name": "release-unauthorized-event-private-asset"}]
        }
    }))
    .expect("serialize signed release deletion notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "release", &event_body));
    assert_eq!(
        status, 202,
        "valid release deletion should be durably admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay release deletion delivery");
    assert_eq!(durable.len(), 1);
    assert_eq!(durable[0].payload["payload"]["action"], "deleted");
    let journal_before = fs::read(&journal_path).expect("snapshot journal before readback");
    let fixture = start_cli_http_fixture(401, json!({"message": API_PRIVATE_MARKER}), 1);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI for an unauthorized Release readback");
    let requests = fixture.join().expect("Release 401 fixture completed");

    assert!(
        !output.status.success(),
        "401 must not be treated as a successful read"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/releases/{RELEASE_ID} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "this public Release read has no host credential and must not invent an Authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("unauthorized Release read returns a structured inconclusive result");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_unauthorized");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["action"],
        "verify the host-scoped Forgejo API credential is present, current, and accepted for the configured repository; keep its value outside agent input"
    );
    assert_eq!(value["error"]["details"]["upstream"], "forgejo");
    assert_eq!(value["error"]["details"]["resource"]["kind"], "release");
    assert_eq!(value["error"]["details"]["resource"]["owner"], OWNER);
    assert_eq!(
        value["error"]["details"]["resource"]["repository"],
        REPOSITORY
    );
    assert_eq!(
        value["error"]["details"]["resource"]["release_id"],
        RELEASE_ID
    );
    assert_eq!(value["journal_source"]["sequence"], durable[0].sequence);
    assert_eq!(value["journal_source"]["event_sha256"], durable[0].hash);
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["status"],
        "verified"
    );
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["verification_scope"],
        "receiver_signature_only"
    );
    assert!(value.get("canonical_state").is_none());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for private_marker in [
        EVENT_PRIVATE_MARKER,
        "release-unauthorized-event-private-asset",
        API_PRIVATE_MARKER,
    ] {
        assert!(
            !output_text.contains(private_marker),
            "untrusted content leaked: {private_marker}"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after unauthorized API result"),
        journal_before,
        "an unauthorized read must not mutate the signed event journal"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_recovers_release_read_after_transient_503() {
    const DELIVERY_ID: &str = "release-deleted-transient-503-readback";
    const RELEASE_ID: u64 = 615;
    const EVENT_PRIVATE_MARKER: &str = "release-retry-stale-event-private-marker";
    const TRANSIENT_PRIVATE_MARKER: &str = "release-retry-503-private-marker";
    const CURRENT_PRIVATE_MARKER: &str = "release-retry-current-api-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event_body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "release": {
            "id": RELEASE_ID,
            "tag_name": "stale-retry-release-tag",
            "body": EVENT_PRIVATE_MARKER,
            "assets": [{"name": "release-retry-event-private-asset"}]
        }
    }))
    .expect("serialize signed Release deletion notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "release", &event_body));
    assert_eq!(
        status, 202,
        "valid Release deletion notification should be durably admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay Release deletion delivery");
    assert_eq!(durable.len(), 1);
    let journal_before = fs::read(&journal_path).expect("snapshot journal before readback");
    let current_release_url =
        format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/releases/tag/v1.4.5");
    let fixture = start_cli_http_fixture_responses(vec![
        (503, json!({"message": TRANSIENT_PRIVATE_MARKER})),
        (
            200,
            json!({
                "id": RELEASE_ID,
                "tag_name": "v1.4.5",
                "body": CURRENT_PRIVATE_MARKER,
                "draft": false,
                "prerelease": false,
                "html_url": current_release_url,
                "assets": [{"name": "release-retry-api-private-asset"}]
            }),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI across transient Release read failure");
    let requests = fixture
        .join()
        .expect("transient and current Release API fixtures completed");

    assert!(
        output.status.success(),
        "a recovered current Release read should succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/releases/{RELEASE_ID} HTTP/1.1\r\n"
        )));
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "public Release read must not receive a host credential"
        );
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("recovered Release read returns structured current state");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 2);
    assert_eq!(value["resource"]["kind"], "release");
    assert_eq!(value["resource"]["release_id"], RELEASE_ID);
    assert_eq!(value["journal_source"]["sequence"], durable[0].sequence);
    assert_eq!(value["journal_source"]["event_sha256"], durable[0].hash);
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["verification_scope"],
        "receiver_signature_only"
    );
    assert_eq!(value["canonical_state"]["state"], "present");
    assert_eq!(value["canonical_state"]["html_url"], current_release_url);
    assert_eq!(value["canonical_state"]["release"]["id"], RELEASE_ID);
    assert_eq!(value["canonical_state"]["release"]["tag_name"], "v1.4.5");
    assert_eq!(
        value["canonical_state"]["release"]["source"],
        "current_forgejo_resource_api"
    );
    assert_eq!(
        value["canonical_state"]["release"]["freshness"],
        "current_api_read"
    );
    assert_eq!(
        value["canonical_state"]["release"]["release_notes_included"],
        false
    );
    assert_eq!(
        value["canonical_state"]["release"]["assets_included"],
        false
    );
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for private_marker in [
        EVENT_PRIVATE_MARKER,
        "release-retry-event-private-asset",
        TRANSIENT_PRIVATE_MARKER,
        CURRENT_PRIVATE_MARKER,
        "release-retry-api-private-asset",
    ] {
        assert!(
            !output_text.contains(private_marker),
            "untrusted content leaked: {private_marker}"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after recovered Release read"),
        journal_before,
        "readback must not rewrite the signed Release notification"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_deleted_release_404_inconclusive_without_projection() {
    const DELIVERY_ID: &str = "release-deleted-current-404";
    const RELEASE_ID: u64 = 611;
    const EVENT_PRIVATE_MARKER: &str = "release-delete-event-private-marker";
    const API_PRIVATE_MARKER: &str = "release-404-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event_body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "release": {
            "id": RELEASE_ID,
            "tag_name": "v1.4.3",
            "body": EVENT_PRIVATE_MARKER,
            "assets": [{"name": "release-delete-event-private-asset"}]
        }
    }))
    .expect("serialize signed release deletion notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "release", &event_body));
    assert_eq!(
        status, 202,
        "valid release deletion is durably admitted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay release deletion delivery");
    assert_eq!(durable.len(), 1);
    assert_eq!(durable[0].payload["payload"]["action"], "deleted");
    let journal_before = fs::read(&journal_path).expect("snapshot journal before readback");
    let fixture = start_cli_http_fixture(404, json!({"message": API_PRIVATE_MARKER}), 1);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process for deleted release");
    let requests = fixture.join().expect("release 404 fixture completed");

    assert!(
        !output.status.success(),
        "a release 404 is inconclusive and must never become successful absence"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/releases/{RELEASE_ID} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "public release read must not receive a merge credential"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("release 404 returns a structured inconclusive result");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["resource"]["kind"], "release");
    assert_eq!(value["error"]["details"]["resource"]["owner"], OWNER);
    assert_eq!(
        value["error"]["details"]["resource"]["repository"],
        REPOSITORY
    );
    assert_eq!(
        value["error"]["details"]["resource"]["release_id"],
        RELEASE_ID
    );
    assert_eq!(value["journal_source"]["sequence"], durable[0].sequence);
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["status"],
        "verified"
    );
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["verification_scope"],
        "receiver_signature_only"
    );
    assert_eq!(value["journal_source"]["event_sha256"], durable[0].hash);
    assert!(value.get("canonical_state").is_none());
    let action = value["error"]["action"]
        .as_str()
        .expect("404 refusal includes release-specific remediation");
    assert!(action.contains("release ID"), "unexpected action: {action}");
    assert!(
        action.contains("repository access"),
        "unexpected action: {action}"
    );
    assert!(
        action.contains("inaccessible repository"),
        "unexpected action: {action}"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(EVENT_PRIVATE_MARKER));
    assert!(!transcript.contains("release-delete-event-private-asset"));
    assert!(!transcript.contains(API_PRIVATE_MARKER));
    assert_eq!(
        fs::read(&journal_path).expect("journal remains durable after inconclusive readback"),
        journal_before,
        "404 reconciliation must not rewrite the webhook record"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_escape_equivalent_duplicate_workflow_status_before_projection()
 {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize valid signed workflow-run notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(
            "workflow-run-duplicate-api-status",
            "workflow_run_success",
            &event_body,
        ),
    );
    assert_eq!(status, 202, "valid webhook admission: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot admitted webhook journal");

    let duplicate_status = r#"{"id":7001,"index_in_repo":41,"status":"failure",
        "sta\u0074us":"success","commit_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "updated":"2026-10-26T11:05:00Z","html_url":"__RUN_URL__",
        "event_payload":"duplicate-api-status-marker","repository":{
        "full_name":"__FULL_NAME__","name":"__REPOSITORY__",
        "owner":{"login":"__OWNER__"}}}"#
        .replace("__RUN_URL__", &run_url)
        .replace("__FULL_NAME__", &format!("{OWNER}/{REPOSITORY}"))
        .replace("__REPOSITORY__", REPOSITORY)
        .replace("__OWNER__", OWNER)
        .into_bytes();
    let fixture = start_cli_http_raw_body_fixture(duplicate_status);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("workflow-run-duplicate-api-status")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let request = fixture
        .join()
        .expect("duplicate-member API fixture completed");

    assert!(!output.status.success());
    assert!(request.starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "public loopback fixture does not need an Authorization header"
    );
    let result: Value = serde_json::from_slice(&output.stdout)
        .expect("duplicate upstream member returns structured refusal");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["delivery_id"], "workflow-run-duplicate-api-status");
    assert_eq!(result["upstream_read_attempts"], 1);
    assert_eq!(result["error"]["code"], "upstream_invalid_response");
    assert_eq!(result["error"]["retryable"], false);
    assert!(result.get("canonical_state").is_none());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("duplicate-api-status-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read admitted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_private_workflow_run_with_host_credential_fd() {
    const TOKEN: &str = "synthetic-private-workflow-run-token-012345";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");

    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "event-private-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(
            "workflow-run-private-current-readback",
            "workflow_run_success",
            &event_body,
        ),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay workflow-run delivery");
    assert_eq!(durable.len(), 1);
    let journal_before = fs::read(&journal_path).expect("snapshot journal before CLI");
    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({
            "id": 7001,
            "index_in_repo": 41,
            "status": "failure",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-26T11:05:00Z",
            "html_url": run_url,
            "event_payload": "api-private-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }),
        format!("token {TOKEN}"),
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic Forgejo token to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg("workflow-run-private-current-readback")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn reconciliation CLI with private workflow-run credential");
    drop(credential_reader);
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    fixture_finished
        .send(())
        .expect("signal that authenticated workflow-run readback has exited");
    let requests = fixture
        .join()
        .expect("authenticated workflow-run API fixture completed");

    assert!(
        !timed_out,
        "authenticated workflow-run readback exceeded its deadline"
    );
    assert_eq!(requests.len(), 1);
    let (request, authorized) = &requests[0];
    assert!(
        *authorized,
        "workflow-run fixture rejected host Authorization header"
    );
    assert!(request.starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("token {TOKEN}")
        })
    }));
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(TOKEN), "credential leaked to stdout");
    assert!(!stderr.contains(TOKEN), "credential leaked to stderr");

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("workflow-run reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(
        value["delivery_id"],
        "workflow-run-private-current-readback"
    );
    assert_eq!(value["resource"]["kind"], "workflow_run");
    assert_eq!(value["resource"]["run_id"], 7001);
    assert_eq!(value["resource"]["index_in_repo"], 41);
    assert_eq!(value["canonical_state"]["state"], "failure");
    assert_eq!(
        value["canonical_state"]["updated_at"],
        "2026-10-26T11:05:00Z"
    );
    assert_eq!(
        value["canonical_state"]["head_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(value["canonical_state"]["html_url"], run_url);
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("event-private-marker"));
    assert!(!output_text.contains("api-private-marker"));
    assert!(!output_text.contains(TOKEN));
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after CLI reconciliation"),
        journal_before,
        "reconciliation reads without rewriting the signed event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_workflow_run_api_id_mismatch() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-run-api-id-mismatch";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_run_success", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": 7002,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-26T11:05:00Z",
            "html_url": run_url,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let requests = fixture.join().expect("workflow-run API fixture completed");

    assert!(
        !output.status.success(),
        "mismatched API run ID is not success"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(!requests[0].contains("/actions/jobs/"));
    assert!(!requests[0].contains("/logs"));

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("run ID mismatch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_workflow_run_foreign_api_html_url() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-run-foreign-api-html-url";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "private-workflow-run-event-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_run_success", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let foreign_url = format!("https://attacker.invalid/{OWNER}/{REPOSITORY}/actions/runs/41");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-26T11:05:00Z",
            "html_url": foreign_url,
            "event_payload": "private-workflow-run-api-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let requests = fixture.join().expect("workflow-run API fixture completed");

    assert!(
        !output.status.success(),
        "foreign canonical URL is not a successful readback"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign workflow-run URL returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("attacker.invalid"));
    assert!(!output_text.contains("private-workflow-run-event-marker"));
    assert!(!output_text.contains("private-workflow-run-api-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_workflow_run_foreign_api_repository() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-run-foreign-api-repository";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "private-workflow-run-event-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_run_success", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "2026-10-26T11:05:00Z",
            "html_url": run_url,
            "event_payload": "private-workflow-run-api-marker",
            "repository": {
                "full_name": "other-owner/other-repo",
                "name": "other-repo",
                "owner": {"login": "other-owner"}
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let requests = fixture.join().expect("workflow-run API fixture completed");

    assert!(
        !output.status.success(),
        "foreign current repository is not a successful readback"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign workflow-run repository returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("other-owner"));
    assert!(!output_text.contains("private-workflow-run-event-marker"));
    assert!(!output_text.contains("private-workflow-run-api-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_workflow_run_null_api_commit_sha() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-run-null-api-commit-sha";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "private-workflow-run-event-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_run_success", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": null,
            "updated": "2026-10-26T11:05:00Z",
            "html_url": run_url,
            "event_payload": "private-workflow-run-api-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let requests = fixture.join().expect("workflow-run API fixture completed");

    assert!(
        !output.status.success(),
        "null current commit SHA is not a successful readback"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("null workflow-run commit SHA returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "commit_sha");
    assert!(value.get("canonical_state").is_none());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("private-workflow-run-event-marker"));
    assert!(!output_text.contains("private-workflow-run-api-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_workflow_run_malformed_api_updated() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-run-malformed-api-updated";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "private-workflow-run-event-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_run_success", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "updated": "not-a-timestamp",
            "html_url": run_url,
            "event_payload": "private-workflow-run-api-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let requests = fixture.join().expect("workflow-run API fixture completed");

    assert!(
        !output.status.success(),
        "malformed current update timestamp is not a successful readback"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("malformed workflow-run timestamp returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "updated");
    assert!(value.get("canonical_state").is_none());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("private-workflow-run-event-marker"));
    assert!(!output_text.contains("private-workflow-run-api-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_workflow_run_404_inconclusive_without_projection() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-run-missing-readback";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_body = serde_json::to_vec(&json!({
        "action": "run_completed",
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "success",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "html_url": run_url,
            "event_payload": "private-workflow-run-event-marker",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-run notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_run_success", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_cli_http_fixture(
        404,
        json!({"message": "workflow-run-404-private-marker"}),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let requests = fixture.join().expect("workflow-run API fixture completed");

    assert!(
        !output.status.success(),
        "a 404 workflow-run readback is inconclusive, never a successful state"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("workflow-run 404 returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["resource"]["kind"],
        "workflow_run"
    );
    assert_eq!(value["error"]["details"]["resource"]["run_id"], 7001);
    assert_eq!(value["error"]["details"]["resource"]["index_in_repo"], 41);
    assert!(value.get("canonical_state").is_none());
    let action = value["error"]["action"]
        .as_str()
        .expect("404 has a remediation action");
    assert!(
        action.contains("workflow run ID"),
        "unexpected action: {action}"
    );
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains("private-workflow-run-event-marker"));
    assert!(!output_text.contains("workflow-run-404-private-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let job_url = format!("{run_url}/jobs/0/attempt/1");
    let event_job_url =
        format!("https://attacker.invalid/{OWNER}/{REPOSITORY}/actions/runs/41/jobs/0/attempt/1");

    let event_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "name": "webhook-job-name-private-marker",
            "html_url": event_job_url,
            "steps": [{"name": "webhook-step-private-marker", "status": "failure"}],
            "runs_on": ["webhook-runner-private-marker"]
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "running",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            },
            "event_payload": "webhook-event-private-marker"
        }
    }))
    .expect("serialize signed workflow-job notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(
            "workflow-job-current-readback",
            "workflow_job_failure",
            &event_body,
        ),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay workflow-job delivery");
    assert_eq!(durable.len(), 1);
    let journal_before = fs::read(&journal_path).expect("snapshot journal before CLI");
    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
        (
            200,
            json!({
                "id": 9001,
                "run_id": 7001,
                "status": "success",
                "name": "api-job-name-private-marker",
                "html_url": job_url,
                "steps": [{"name": "api-step-private-marker", "status": "success"}],
                "runs_on": ["api-runner-private-marker"],
                "task_id": 42
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": 7001,
                "index_in_repo": 41,
                "status": "failure",
                "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "updated": "2026-10-26T11:05:00Z",
                "html_url": run_url,
                "event_payload": "api-event-private-marker",
                "repository": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": 9001,
                "run_id": 7001,
                "status": "success",
                "name": "fresh-api-job-name-private-marker",
                "html_url": job_url,
                "steps": [{"name": "fresh-api-step-private-marker", "status": "success"}],
                "runs_on": ["fresh-api-runner-private-marker"],
                "task_id": 43
            }),
            Vec::new(),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("workflow-job-current-readback")
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let _ = finish_fixture.send(());
    let requests = fixture.join().expect("workflow-job API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(requests[2].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| !request.contains("/logs")));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("workflow-job reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 3);
    assert_eq!(value["delivery_id"], "workflow-job-current-readback");
    assert_eq!(value["resource"]["kind"], "workflow_job");
    assert_eq!(value["resource"]["job_id"], 9001);
    assert_eq!(value["resource"]["run_id"], 7001);
    assert_eq!(value["resource"]["index_in_repo"], 41);
    assert_eq!(value["canonical_state"]["state"], "success");
    assert_eq!(
        value["canonical_state"]["updated_at"],
        "2026-10-26T11:05:00Z"
    );
    assert_eq!(
        value["canonical_state"]["head_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(value["canonical_state"]["html_url"], job_url);
    assert_eq!(value["canonical_state"]["state"], "success");
    assert_eq!(
        value["canonical_state"]["parent_workflow_run"]["state"], "failure",
        "job status is not the workflow run status"
    );
    assert_eq!(
        value["canonical_state"]["parent_workflow_run"]["run_id"],
        7001
    );
    assert_eq!(
        value["canonical_state"]["parent_workflow_run"]["index_in_repo"],
        41
    );
    assert_eq!(
        value["canonical_state"]["parent_workflow_run"]["head_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(
        value["canonical_state"]["parent_workflow_run"]["independent_verification"],
        false
    );
    let output_text = String::from_utf8_lossy(&output.stdout);
    assert!(!output_text.contains("attacker.invalid"));
    for private_marker in [
        "webhook-job-name-private-marker",
        "webhook-step-private-marker",
        "webhook-runner-private-marker",
        "webhook-event-private-marker",
        "api-job-name-private-marker",
        "api-step-private-marker",
        "api-runner-private-marker",
        "api-event-private-marker",
        "fresh-api-job-name-private-marker",
        "fresh-api-step-private-marker",
        "fresh-api-runner-private-marker",
    ] {
        assert!(
            !output_text.contains(private_marker),
            "leaked {private_marker}"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after CLI reconciliation"),
        journal_before,
        "reconciliation reads without rewriting the signed event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_private_workflow_job_and_parent_run_with_host_credential() {
    const TOKEN: &str = "synthetic-private-workflow-job-token-02468";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let job_url = format!("{run_url}/jobs/0/attempt/1");

    let event_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "name": "webhook-job-name-private-marker",
            "html_url": job_url,
            "steps": [{"name": "webhook-step-private-marker", "status": "failure"}],
            "runs_on": ["webhook-runner-private-marker"]
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "status": "running",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "updated": "2026-10-26T11:00:00Z",
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            },
            "event_payload": "webhook-event-private-marker"
        }
    }))
    .expect("serialize signed workflow-job notification");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(
            "workflow-job-private-current-readback",
            "workflow_job_failure",
            &event_body,
        ),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let durable = journal.replay().expect("replay workflow-job delivery");
    assert_eq!(durable.len(), 1);
    let journal_before = fs::read(&journal_path).expect("snapshot journal before CLI");
    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
        (
            200,
            json!({
                "id": 9001,
                "run_id": 7001,
                "status": "success",
                "name": "api-job-name-private-marker",
                "html_url": job_url,
                "steps": [{"name": "api-step-private-marker", "status": "success"}],
                "runs_on": ["api-runner-private-marker"],
                "task_id": 42
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": 7001,
                "index_in_repo": 41,
                "status": "failure",
                "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "updated": "2026-10-26T11:05:00Z",
                "html_url": run_url,
                "event_payload": "api-event-private-marker",
                "repository": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }),
            Vec::new(),
        ),
    ]);
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic private-workflow credential to pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg("workflow-job-private-current-readback")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn reconciliation CLI with private workflow-job credential");
    drop(credential_reader);
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    let _ = finish_fixture.send(());
    let requests = fixture
        .join()
        .expect("authenticated workflow-job API fixture completed");

    assert!(
        !timed_out,
        "authenticated workflow-job readback exceeded its deadline"
    );
    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    for request in &requests {
        assert!(!request.contains("/logs"));
        assert!(
            request.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case("authorization")
                        && value.trim() == format!("token {TOKEN}")
                })
            }),
            "host credential was not sent on exact read route: {request}"
        );
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("workflow-job reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 2);
    assert_eq!(
        value["delivery_id"],
        "workflow-job-private-current-readback"
    );
    assert_eq!(value["resource"]["kind"], "workflow_job");
    assert_eq!(value["resource"]["job_id"], 9001);
    assert_eq!(value["resource"]["run_id"], 7001);
    assert_eq!(value["resource"]["index_in_repo"], 41);
    assert_eq!(value["canonical_state"]["state"], "success");
    assert_eq!(
        value["canonical_state"]["updated_at"],
        "2026-10-26T11:05:00Z"
    );
    assert_eq!(
        value["canonical_state"]["head_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(value["canonical_state"]["html_url"], job_url);
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for private_marker in [
        TOKEN,
        "webhook-job-name-private-marker",
        "webhook-step-private-marker",
        "webhook-runner-private-marker",
        "webhook-event-private-marker",
        "api-job-name-private-marker",
        "api-step-private-marker",
        "api-runner-private-marker",
        "api-event-private-marker",
    ] {
        assert!(
            !output_text.contains(private_marker),
            "leaked {private_marker}"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after CLI reconciliation"),
        journal_before,
        "reconciliation reads without rewriting the signed event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_job_attached_to_another_run_before_parent_read() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-job-foreign-parent-readback";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let job_url = format!("{run_url}/jobs/0/attempt/1");
    let event_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "html_url": job_url
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-job notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_job_failure", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![(
        200,
        json!({
            "id": 9001,
            "run_id": 7002,
            "status": "success",
            "html_url": job_url,
            "name": "api-foreign-parent-private-marker"
        }),
        Vec::new(),
    )]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let _ = finish_fixture.send(());
    let requests = fixture.join().expect("workflow-job API fixture completed");

    assert!(
        !output.status.success(),
        "job bound to another run is not success"
    );
    assert_eq!(
        requests.len(),
        1,
        "parent run must not be read after mismatch"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(!requests[0].contains("/actions/runs/"));
    assert!(!requests[0].contains("/logs"));

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("job/run mismatch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("api-foreign-parent-private-marker"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_unscoped_workflow_job_url() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-job-unscoped-current-readback";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let foreign_url =
        "https://attacker.invalid/forge-admin/agent-forge-core/actions/runs/41/jobs/0/attempt/1";
    let event_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "html_url": foreign_url
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-job notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_job_failure", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
        (
            200,
            json!({
                "id": 9001,
                "run_id": 7001,
                "status": "success",
                "html_url": foreign_url
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": 7001,
                "index_in_repo": 41,
                "status": "failure",
                "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "updated": "2026-10-26T11:05:00Z",
                "html_url": run_url,
                "repository": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }),
            Vec::new(),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let _ = finish_fixture.send(());
    let requests = fixture.join().expect("workflow-job API fixture completed");

    assert!(!output.status.success(), "foreign job URL is not success");
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| !request.contains("/logs")));

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("invalid job URL returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 2);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "html_url");
    assert!(value.get("canonical_state").is_none());
    let output_text = String::from_utf8_lossy(&output.stdout);
    assert!(!output_text.contains("attacker.invalid"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_api_url_for_another_workflow_job_index() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "workflow-job-other-index-current-readback";
    let run_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/actions/runs/41");
    let event_job_url = format!("{run_url}/jobs/1/attempt/1");
    let api_job_url = format!("{run_url}/jobs/0/attempt/1");
    let event_body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {
            "id": 9001,
            "run_id": 7001,
            "status": "failure",
            "html_url": event_job_url
        },
        "run": {
            "id": 7001,
            "index_in_repo": 41,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-job notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "workflow_job_failure", &event_body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
        (
            200,
            json!({
                "id": 9001,
                "run_id": 7001,
                "status": "success",
                "html_url": api_job_url
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": 7001,
                "index_in_repo": 41,
                "status": "failure",
                "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "updated": "2026-10-26T11:05:00Z",
                "html_url": run_url,
                "repository": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }),
            Vec::new(),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI process");
    let _ = finish_fixture.send(());
    let requests = fixture.join().expect("workflow-job API fixture completed");

    assert!(
        !output.status.success(),
        "a different job index is inconclusive"
    );
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/runs/7001 HTTP/1.1\r\n"
    )));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("mismatched job URL returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["error"]["details"]["field"], "html_url");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_workflow_job_404_inconclusive_without_parent_read() {
    const DELIVERY_ID: &str = "workflow-job-missing-readback";
    const PRIVATE_404_MARKER: &str = "workflow-job-404-private-marker";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "job_completed",
        "job": {"id":9001,"run_id":7001,"status":"success"},
        "run": {
            "id":7001,
            "index_in_repo":41,
            "repository": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    }))
    .expect("serialize signed workflow-job success notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "workflow_job_success", &body),
    );
    assert_eq!(
        status, 202,
        "the signed notification should be durable: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted job journal");

    let fixture = start_cli_http_fixture(404, json!({"message":PRIVATE_404_MARKER}), 1);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual workflow-job reconciliation CLI");
    let requests = fixture
        .join()
        .expect("workflow-job API 404 fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        1,
        "parent run must not be read after job 404"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/actions/jobs/9001 HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the local reconciliation CLI must not send an authorization header"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(PRIVATE_404_MARKER));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("job 404 becomes a structured inconclusive result");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(
        value["error"]["details"]["resource"]["kind"],
        "workflow_job"
    );
    assert_eq!(value["error"]["details"]["resource"]["job_id"], 9001);
    assert!(value.get("canonical_state").is_none());
    let action = value["error"]["action"]
        .as_str()
        .expect("404 has a remediation action");
    assert!(
        action.contains("workflow job ID"),
        "unexpected action: {action}"
    );
    assert!(
        action.contains("does not distinguish"),
        "the action must preserve 404 ambiguity: {action}"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted job journal"),
        journal_before,
        "the inconclusive readback does not rewrite the durable notification"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_uses_current_state_after_out_of_order_webhooks() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let newer_body = serde_json::to_vec(&json!({
        "action": "reopened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z"
        }
    }))
    .expect("serialize newer issue event");
    let older_body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-10-22T16:00:00Z"
        }
    }))
    .expect("serialize older issue event");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, body, expected_sequence) in [
        ("out-of-order-newer-first", newer_body.as_slice(), 1),
        ("out-of-order-older-late", older_body.as_slice(), 2),
    ] {
        let (status, accepted) = send(address, &request(delivery_id, body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal = EventJournal::open(&journal_path);
    let durable = journal
        .replay()
        .expect("replay out-of-order webhook journal");
    assert_eq!(durable.len(), 2);
    assert_eq!(
        durable[0].payload["payload"]["issue"]["updated_at"],
        "2026-10-22T17:00:00Z"
    );
    assert_eq!(
        durable[1].payload["payload"]["issue"]["updated_at"], "2026-10-22T16:00:00Z",
        "the older update arrived after the newer update"
    );
    let journal_before = fs::read(&journal_path).expect("snapshot journal before reconciliation");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-10-22T18:00:00Z",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41"
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("out-of-order-older-late")
        .arg("out-of-order-newer-first")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("current-state API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.len(),
        1,
        "both events resolve through one resource read"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for (result, delivery_id) in results
        .iter()
        .zip(["out-of-order-older-late", "out-of-order-newer-first"])
    {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["canonical_state"]["state"], "closed");
        assert_eq!(
            result["canonical_state"]["updated_at"], "2026-10-22T18:00:00Z",
            "canonical state must come from a fresh read, not either webhook payload"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read journal after current-state reconciliation"),
        journal_before,
        "reconciliation reads the journal without rewriting delivery order"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_shares_current_comment_read_across_create_delete_batch() {
    const ISSUE_NUMBER: u64 = 24;
    const COMMENT_ID: u64 = 916;
    const ISSUE_DELIVERY_ID: &str = "cli-comment-cache-parent-24";
    const CREATED_DELIVERY_ID: &str = "cli-comment-cache-created-916";
    const DELETED_DELIVERY_ID: &str = "cli-comment-cache-deleted-916";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let issue_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize signed parent-issue webhook");
    let comment_body = |action: &str| {
        serde_json::to_vec(&json!({
            "action": action,
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER, "state": "open"},
            "comment": {"id": COMMENT_ID, "body": "untrusted webhook comment"},
            "is_pull": false
        }))
        .expect("serialize signed issue-comment webhook")
    };

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (event, delivery_id, body) in [
        ("issues", ISSUE_DELIVERY_ID, issue_body),
        (
            "issue_comment",
            CREATED_DELIVERY_ID,
            comment_body("created"),
        ),
        (
            "issue_comment",
            DELETED_DELIVERY_ID,
            comment_body("deleted"),
        ),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, event, &body));
        assert_eq!(
            status, 202,
            "accepted {event} delivery {delivery_id}: {accepted}"
        );
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable mixed-event journal");
    let journal = EventJournal::open(&journal_path);
    assert_eq!(journal.replay().expect("replay accepted events").len(), 3);

    let delivery_orders = [
        [ISSUE_DELIVERY_ID, CREATED_DELIVERY_ID, DELETED_DELIVERY_ID],
        [DELETED_DELIVERY_ID, CREATED_DELIVERY_ID, ISSUE_DELIVERY_ID],
    ];
    for delivery_order in delivery_orders {
        let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
            (
                200,
                json!({
                    "number": ISSUE_NUMBER,
                    "state": "closed",
                    "updated_at": "2026-10-26T08:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
                }),
                Vec::new(),
            ),
            (
                404,
                json!({"message":"synthetic-comment-404-body-must-not-leak"}),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": ISSUE_NUMBER,
                    "state": "closed",
                    "updated_at": "2026-10-26T08:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
                }),
                Vec::new(),
            ),
        ]);
        let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
            .env_clear()
            .arg(&journal_path)
            .args(delivery_order)
            .stdin(Stdio::null())
            .output()
            .expect("run the actual reconciliation CLI process");
        let _ = finish_fixture.send(());
        let requests = fixture
            .join()
            .expect("mixed issue/comment API fixture completed");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            requests.len(),
            3,
            "parent, comment, then a fresh-parent read are required for order {delivery_order:?}"
        );
        for triple in requests.chunks_exact(3) {
            assert!(triple[0].starts_with(&format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
            )));
            assert!(triple[1].starts_with(&format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
            )));
            assert!(triple[2].starts_with(&format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
            )));
        }
        assert!(requests.iter().all(|request| {
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        }));
        assert_eq!(
            fs::read(&journal_path).expect("re-read durable mixed-event journal"),
            journal_before,
            "reconciliation must not rewrite accepted delivery order"
        );

        let transcript = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!transcript.contains("synthetic-comment-404-body-must-not-leak"));
        assert!(!transcript.contains("untrusted webhook comment"));
        let value: Value =
            serde_json::from_slice(&output.stdout).expect("mixed batch returns structured JSON");
        assert_reconcile_schema_version(&value);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["upstream_read_attempts"], 3);
        let results = value["results"].as_array().expect("batch result array");
        assert_eq!(results.len(), delivery_order.len());
        for (result, delivery_id) in results.iter().zip(delivery_order) {
            assert_eq!(result["delivery_id"], delivery_id);
        }

        let issue_result = results
            .iter()
            .find(|result| result["delivery_id"] == ISSUE_DELIVERY_ID)
            .expect("issue event remains in the batch");
        assert_eq!(issue_result["verdict"], "current_state_read");
        assert_eq!(issue_result["canonical_state"]["state"], "closed");

        let created_result = results
            .iter()
            .find(|result| result["delivery_id"] == CREATED_DELIVERY_ID)
            .expect("created comment remains in the batch");
        assert_eq!(created_result["verdict"], "inconclusive");
        assert_eq!(created_result["error"]["code"], "resource_not_found");
        assert_eq!(created_result["error"]["retryable"], false);
        assert_eq!(
            created_result["error"]["details"]["resource"]["kind"],
            "issue_comment"
        );
        assert_eq!(
            created_result["error"]["details"]["resource"]["comment_id"],
            COMMENT_ID
        );
        assert!(created_result.get("canonical_state").is_none());

        let deleted_result = results
            .iter()
            .find(|result| result["delivery_id"] == DELETED_DELIVERY_ID)
            .expect("deleted comment remains in the batch");
        assert_eq!(deleted_result["verdict"], "current_state_read");
        assert_eq!(
            deleted_result["canonical_state"]["comment"]["state"],
            "absent"
        );
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reuses_pull_request_parent_and_comment_read_across_batch_orders() {
    const NUMBER: u64 = 23;
    const COMMENT_ID: u64 = 915;
    const PULL_DELIVERY_ID: &str = "cli-pr-comment-cache-parent-23";
    const CREATED_DELIVERY_ID: &str = "cli-pr-comment-cache-created-915";
    const DELETED_DELIVERY_ID: &str = "cli-pr-comment-cache-deleted-915";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let pull_body = serde_json::to_vec(&json!({
        "action": "closed",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "closed",
            "merged": true,
            "merge_commit_sha": "3333333333333333333333333333333333333333",
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize signed parent pull-request webhook");
    let comment_body = |action: &str| {
        serde_json::to_vec(&json!({
            "action": action,
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": NUMBER, "state": "closed"},
            "comment": {"id": COMMENT_ID, "body": "untrusted PR comment webhook"},
            "is_pull": true,
            "pull_request": {"number": NUMBER, "state": "closed"}
        }))
        .expect("serialize signed PR-comment webhook")
    };
    let created_body = comment_body("created");
    let deleted_body = comment_body("deleted");
    let deliveries = [
        ("pull_request", PULL_DELIVERY_ID, pull_body.as_slice()),
        (
            "issue_comment",
            CREATED_DELIVERY_ID,
            created_body.as_slice(),
        ),
        (
            "issue_comment",
            DELETED_DELIVERY_ID,
            deleted_body.as_slice(),
        ),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (expected_sequence, (event, delivery_id, body)) in deliveries.iter().enumerate() {
        let (status, accepted) = send(address, &request_event(delivery_id, event, body));
        assert_eq!(
            status, 202,
            "accepted {event} delivery {delivery_id}: {accepted}"
        );
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted PR/comment batch");

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let pull_response = json!({
        "number": NUMBER,
        "state": "closed",
        "merged": true,
        "merge_commit_sha": "4444444444444444444444444444444444444444",
        "updated_at": "2026-10-26T08:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": current_head},
        "base": {"sha": current_base}
    });
    let fixture = start_cli_http_fixture_responses(vec![
        (200, pull_response.clone()),
        (
            404,
            json!({"message":"synthetic-pr-comment-404-body-must-not-leak"}),
        ),
        (200, pull_response.clone()),
        (200, pull_response.clone()),
        (
            404,
            json!({"message":"synthetic-pr-comment-404-body-must-not-leak"}),
        ),
        (200, pull_response),
    ]);
    let delivery_orders = [
        [PULL_DELIVERY_ID, CREATED_DELIVERY_ID, DELETED_DELIVERY_ID],
        [DELETED_DELIVERY_ID, CREATED_DELIVERY_ID, PULL_DELIVERY_ID],
    ];
    let mut outputs = Vec::new();
    for delivery_order in delivery_orders {
        outputs.push(
            Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
                .env_clear()
                .arg(&journal_path)
                .args(delivery_order)
                .stdin(Stdio::null())
                .output()
                .expect("run the actual reconciliation CLI process"),
        );
    }
    let requests = fixture
        .join()
        .expect("PR/comment batch API fixture completed");
    assert_eq!(requests.len(), 6);
    for triple in requests.chunks_exact(3) {
        assert!(triple[0].starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
        )));
        assert!(triple[1].starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
        )));
        assert!(triple[2].starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
        )));
    }
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable PR/comment batch"),
        journal_before,
        "reconciliation must not rewrite webhook delivery order"
    );

    for (output, delivery_order) in outputs.iter().zip(delivery_orders) {
        assert_eq!(output.status.code(), Some(1));
        let transcript = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!transcript.contains("synthetic-pr-comment-404-body-must-not-leak"));
        assert!(!transcript.contains("untrusted PR comment webhook"));
        let value: Value =
            serde_json::from_slice(&output.stdout).expect("PR/comment batch returns JSON");
        assert_reconcile_schema_version(&value);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["upstream_read_attempts"], 3);
        let results = value["results"].as_array().expect("batch results array");
        assert_eq!(results.len(), 3);
        for (result, delivery_id) in results.iter().zip(delivery_order) {
            assert_eq!(result["delivery_id"], delivery_id);
        }
        let pull_result = results
            .iter()
            .find(|result| result["delivery_id"] == PULL_DELIVERY_ID)
            .expect("parent PR remains in the batch");
        assert_eq!(pull_result["verdict"], "current_state_read");
        assert_eq!(pull_result["resource"]["kind"], "pull_request");
        assert_eq!(pull_result["canonical_state"]["merged"], true);
        assert_eq!(pull_result["canonical_state"]["head_sha"], current_head);
        assert_eq!(pull_result["canonical_state"]["base_sha"], current_base);
        let created_result = results
            .iter()
            .find(|result| result["delivery_id"] == CREATED_DELIVERY_ID)
            .expect("created PR comment remains in the batch");
        assert_eq!(created_result["verdict"], "inconclusive");
        assert_eq!(created_result["error"]["code"], "resource_not_found");
        assert_eq!(created_result["error"]["retryable"], false);
        assert_eq!(
            created_result["error"]["details"]["resource"]["kind"],
            "pull_request_comment"
        );
        assert!(created_result.get("canonical_state").is_none());
        let deleted_result = results
            .iter()
            .find(|result| result["delivery_id"] == DELETED_DELIVERY_ID)
            .expect("deleted PR comment remains in the batch");
        assert_eq!(deleted_result["verdict"], "current_state_read");
        assert_eq!(
            deleted_result["canonical_state"]["comment"]["state"],
            "absent"
        );
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_enforces_batch_cache_budget_against_private_http_fixture() {
    const RESPONSE_PADDING_BYTES: usize = MAX_RECONCILE_BATCH_CACHED_JSON_BYTES / 16;
    const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let mut delivery_ids = Vec::new();
    for number in 11..=28 {
        let delivery_id = format!("cli-cache-budget-issue-{number}");
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": number, "state": "open"}
        }))
        .expect("serialize synthetic signed issue webhook");
        let (status, accepted) = send(address, &request(&delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        delivery_ids.push(delivery_id);
    }

    let replay_delivery_id = "cli-cache-budget-issue-11-replay".to_owned();
    let replay_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": 11, "state": "open"}
    }))
    .expect("serialize replay of an already reconciled resource");
    let (status, accepted) = send(address, &request(&replay_delivery_id, &replay_body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    delivery_ids.push(replay_delivery_id);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(
        EventJournal::open(&journal_path).replay().unwrap().len(),
        19
    );

    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make cache-budget fixture accept deadline bounded");
    let fixture = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut responses = Vec::with_capacity(16);
        while responses.len() < 16 {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "reconciliation CLI made too few cache-boundary API requests"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept cache-budget fixture request: {error}"),
                }
            };
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .expect("bound fixture response writes");
            let request = read_cli_http_request(&mut stream);
            let request_line = request.lines().next().expect("HTTP request line");
            let path = request_line
                .split_whitespace()
                .nth(1)
                .expect("HTTP request path");
            let number = path
                .strip_prefix(&format!("/api/v1/repos/{OWNER}/{REPOSITORY}/issues/"))
                .expect("CLI stays on the allowlisted issue route")
                .parse::<u64>()
                .expect("issue number is numeric");
            let body = serde_json::to_vec(&json!({
                "number": number,
                "state": "open",
                "updated_at": "2026-10-22T17:00:00Z",
                "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{number}"),
                "cache_fixture": "x".repeat(RESPONSE_PADDING_BYTES)
            }))
            .expect("serialize bounded synthetic Forgejo issue response");
            assert!(
                body.len() <= MAX_RESPONSE_BYTES,
                "fixture response of {} bytes exceeds the per-response HTTP cap",
                body.len()
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write cache-budget fixture headers");
            stream
                .write_all(&body)
                .expect("write cache-budget fixture body");
            responses.push((request, body.len()));
        }
        responses
    });

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .args(&delivery_ids)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn actual reconciliation CLI process against the bounded fixture");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(40));
    let responses = fixture.join().expect("cache-budget HTTP fixture completed");

    assert!(!timed_out, "reconciliation CLI exceeded its 40 s deadline");
    assert_eq!(output.status.code(), Some(1));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("CLI emits structured batch JSON on failure");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 16);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 19);
    for (result, number) in results[..15].iter().zip(11..=25) {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["resource"]["number"], number);
        assert_eq!(result["canonical_state"]["state"], "open");
    }
    for (result, number) in results[15..18].iter().zip(26..=28) {
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(
            result["delivery_id"],
            format!("cli-cache-budget-issue-{number}")
        );
        assert_eq!(result["error"]["code"], "batch_cache_budget_exceeded");
        assert_eq!(result["error"]["retryable"], false);
        assert!(
            result["error"]["action"]
                .as_str()
                .unwrap()
                .contains("split")
        );
    }
    assert_eq!(results[18]["verdict"], "current_state_read");
    assert_eq!(
        results[18]["delivery_id"],
        "cli-cache-budget-issue-11-replay"
    );
    assert_eq!(results[18]["resource"]["number"], 11);

    assert_eq!(responses.len(), 16);
    let response_lengths = responses
        .iter()
        .map(|(_, body_bytes)| *body_bytes)
        .collect::<Vec<_>>();
    assert!(
        response_lengths
            .iter()
            .all(|length| *length <= MAX_RESPONSE_BYTES)
    );
    assert!(response_lengths[..15].iter().sum::<usize>() <= MAX_RECONCILE_BATCH_CACHED_JSON_BYTES);
    assert!(response_lengths.iter().sum::<usize>() > MAX_RECONCILE_BATCH_CACHED_JSON_BYTES);
    for ((request, _), number) in responses.iter().zip(11..=26) {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{number} HTTP/1.1\r\n"
        )));
        assert!(
            !request
                .lines()
                .any(|line| { line.to_ascii_lowercase().starts_with("authorization:") })
        );
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_repeated_comment_batch_output_budget() {
    const DELIVERY_COUNT: usize = 100;
    const COMMENT_ID: u64 = 9001;
    const COMMENT_BODY_BYTES: usize = 128 * 1024;
    const COMMENT_BODY_MARKER: &str = "x";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let mut delivery_ids = Vec::with_capacity(DELIVERY_COUNT);
    for index in 0..DELIVERY_COUNT {
        let delivery_id = format!("cli-output-budget-comment-{index}");
        let body = serde_json::to_vec(&json!({
            "action": "created",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER, "state": "open"},
            "comment": {"id": COMMENT_ID, "body": "stale webhook comment is not canonical"},
            "is_pull": false
        }))
        .expect("serialize synthetic signed issue-comment delivery");
        let (status, accepted) = send(
            address,
            &request_event(&delivery_id, "issue_comment", &body),
        );
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        delivery_ids.push(delivery_id);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("read completed receiver journal");
    let selected_events = EventJournal::open(&journal_path)
        .replay()
        .expect("read selected source events");
    assert_eq!(selected_events.len(), DELIVERY_COUNT);

    let parent_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}");
    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
        (
            200,
            json!({
                "number": ISSUE_NUMBER,
                "state": "open",
                "updated_at": "2026-10-27T12:30:00Z",
                "html_url": parent_url,
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": COMMENT_ID,
                "body": COMMENT_BODY_MARKER.repeat(COMMENT_BODY_BYTES),
                "updated_at": "2026-10-27T12:31:00Z",
                "html_url": format!("{parent_url}#issuecomment-{COMMENT_ID}"),
                "issue_url": parent_url,
            }),
            Vec::new(),
        ),
    ]);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .args(&delivery_ids)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn actual reconciliation CLI against bounded issue/comment fixture");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(40));
    finish_fixture
        .send(())
        .expect("stop API fixture after the actual CLI exits");
    let requests = fixture.join().expect("bounded output fixture completed");

    assert!(!timed_out, "reconciliation CLI exceeded its 40 s deadline");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(
        output.stdout.len() < 64 * 1024,
        "budget refusal should retain only a compact source index, not partial comment bodies"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&COMMENT_BODY_MARKER.repeat(1024)));

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("CLI emits structured JSON on refusal");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], DELIVERY_COUNT);
    assert_eq!(value["upstream_read_attempts"], 2);
    assert!(value.get("results").is_none());
    assert_eq!(value["error"]["code"], "batch_result_budget_exceeded");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["maximum_projected_output_bytes"],
        MAX_RECONCILE_BATCH_OUTPUT_BYTES
    );
    assert!(value["error"]["action"].as_str().unwrap().contains("split"));

    let selected_sources = value["selected_event_sources"]
        .as_array()
        .expect("budget refusal retains selected source identities");
    assert_eq!(selected_sources.len(), DELIVERY_COUNT);
    for (((source, delivery_id), event), expected_sequence) in selected_sources
        .iter()
        .zip(&delivery_ids)
        .zip(&selected_events)
        .zip(1..=DELIVERY_COUNT as u64)
    {
        assert_eq!(source["delivery_id"].as_str(), Some(delivery_id.as_str()));
        assert_eq!(source["journal_source"]["sequence"], expected_sequence);
        assert_eq!(source["journal_source"]["sequence"], event.sequence);
        assert_eq!(source["journal_source"]["event_sha256"], event.hash);
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["status"],
            "missing"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["verification_scope"],
            "not_verified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["signature_included"],
            false
        );
    }

    assert_eq!(
        requests.len(),
        2,
        "one parent and one comment API read expected"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after reconciliation"),
        journal_before,
        "reconciliation refusal must leave the source journal unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_unbounded_chunked_current_api_response() {
    const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
    const RESPONSE_BODY_BYTES: usize = MAX_RESPONSE_BYTES * 2;
    const DELIVERY_ID: &str = "cli-oversized-current-api-response";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue delivery");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("read signed receiver journal");

    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves local Forgejo API endpoint");
    listener
        .set_nonblocking(true)
        .expect("bound fixture accept is deadline-limited");
    let fixture = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "CLI did not read the selected issue"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept oversized-response fixture request: {error}"),
            }
        };
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .expect("bound oversized response write");
        let request = read_cli_http_request(&mut stream);
        let response_body = vec![b'Z'; RESPONSE_BODY_BYTES];
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(headers.as_bytes())
            .expect("write chunked response headers without a size declaration");
        let chunk_header = format!("{:X}\r\n", response_body.len());
        let _ = stream.write_all(chunk_header.as_bytes());
        let _ = stream.write_all(&response_body);
        let _ = stream.write_all(b"\r\n0\r\n\r\n");
        request
    });

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn actual reconciliation CLI against unbounded chunked HTTP response");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(20));
    let request = fixture
        .join()
        .expect("oversized-response fixture completed");

    assert!(!timed_out, "reconciliation CLI exceeded its 20 s deadline");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 16 * 1024);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&"Z".repeat(1024)));

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("oversized API response stays structured");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_response_too_large");
    assert!(matches!(
        value["error"]["details"]["component"].as_str(),
        Some("body" | "captured_output")
    ));
    assert!(value["error"]["details"]["maximum_bytes"].as_u64().unwrap() > 0);
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert!(request.starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after refused API response"),
        journal_before,
        "an oversized current-state response must not mutate the inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_oversized_current_api_header_json() {
    const DELIVERY_ID: &str = "cli-oversized-current-api-header-json";
    const HEADER_JSON_LIMIT_BYTES: usize = 64 * 1024;
    const HEADER_VALUE_BYTES: usize = 70 * 1024;
    const HEADER_MARKER: &str = "synthetic-private-api-header-marker-";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue delivery");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("read signed receiver journal");

    let header_value = format!(
        "{HEADER_MARKER}{}",
        "H".repeat(HEADER_VALUE_BYTES - HEADER_MARKER.len())
    );
    assert!(header_value.len() > HEADER_JSON_LIMIT_BYTES);
    let parent_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}");
    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-27T12:30:00Z",
            "html_url": parent_url,
        }),
        vec![("X-Forgejo-Fixture-Fill".to_owned(), header_value)],
    )]);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn actual CLI against oversized curl header JSON fixture");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(20));
    finish_fixture
        .send(())
        .expect("finish the single-response API fixture");
    let requests = fixture.join().expect("oversized-header fixture completed");

    assert!(!timed_out, "reconciliation CLI exceeded its 20 s deadline");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 16 * 1024);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(HEADER_MARKER));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("oversized header JSON produces structured refusal");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_response_too_large");
    assert_eq!(value["error"]["details"]["component"], "header_json");
    assert_eq!(
        value["error"]["details"]["maximum_bytes"],
        HEADER_JSON_LIMIT_BYTES
    );
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("read journal after oversized response refusal"),
        journal_before,
        "an oversized response header must not mutate the inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_requires_trusted_attestation_policy_before_host_credential_read() {
    const DELIVERY_IDS: [&str; 2] = [
        "cli-missing-attestation-policy-before-credential-a",
        "cli-missing-attestation-policy-before-credential-b",
    ];
    const TOKEN: &[u8] = b"synthetic-missing-attestation-policy-token-012345";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address, _signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (index, delivery_id) in DELIVERY_IDS.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER + index as u64, "state": "open"}
        }))
        .expect("serialize attested issue delivery");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());
    let source_events = EventJournal::open(&journal_path)
        .replay()
        .expect("read the two attested receiver journal rows");
    assert_eq!(source_events.len(), DELIVERY_IDS.len());
    let journal_before = fs::read(&journal_path).expect("read receiver journal before refusal");

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN)
        .expect("write synthetic host credential to private pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .arg(&journal_path)
        .args(DELIVERY_IDS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler without a trusted attestation policy");

    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read back credential after the missing-policy refusal");
    assert_eq!(remaining, TOKEN, "missing policy consumed the credential");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("missing attestation policy returns structured refusal JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], DELIVERY_IDS.len());
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("results").is_none());
    assert_eq!(
        value["error"]["code"],
        "host_journal_attestation_policy_missing"
    );
    assert_eq!(value["error"]["details"]["host_credential_read"], false);
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
    assert!(
        !String::from_utf8_lossy(&output.stdout)
            .contains(std::str::from_utf8(TOKEN).expect("synthetic credential is UTF-8"))
    );
    let selected_sources = value["selected_event_sources"]
        .as_array()
        .expect("batch attestation refusal retains all selected source identities");
    assert_eq!(selected_sources.len(), DELIVERY_IDS.len());
    for ((source, delivery_id), source_event) in selected_sources
        .iter()
        .zip(DELIVERY_IDS)
        .zip(&source_events)
    {
        assert_eq!(source["delivery_id"].as_str(), Some(delivery_id));
        assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
        assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["status"],
            "present_unverified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["verification_scope"],
            "not_verified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["signature_included"],
            false
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after refusal"),
        journal_before,
        "refusing an incomplete trust policy must not mutate the selected source rows"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixed-origin calls must remain unreachable"]
fn actual_reconcile_cli_process_reports_credential_pipe_read_failure_with_selected_sources() {
    const DELIVERY_IDS: [&str; 2] = [
        "cli-credential-pipe-read-failure-a",
        "cli-credential-pipe-read-failure-b",
    ];

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (index, delivery_id) in DELIVERY_IDS.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER + index as u64, "state": "open"}
        }))
        .expect("serialize attested issue delivery");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());
    let source_events = EventJournal::open(&journal_path)
        .replay()
        .expect("read attested receiver journal rows");
    assert_eq!(source_events.len(), DELIVERY_IDS.len());
    let journal_before = fs::read(&journal_path).expect("read receiver journal before refusal");

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(i32::MAX.to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args(DELIVERY_IDS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler with an invalid host credential descriptor");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("credential pipe read failure returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], DELIVERY_IDS.len());
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("results").is_none());
    assert_eq!(value["error"]["code"], "host_credential_pipe_read_failed");
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_attempted"],
        true
    );
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_succeeded"],
        false
    );
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
    assert_eq!(value["error"]["details"]["canonical_state_created"], false);
    let selected_sources = value["selected_event_sources"]
        .as_array()
        .expect("credential pipe refusal retains all selected source identities");
    assert_eq!(selected_sources.len(), DELIVERY_IDS.len());
    for ((source, delivery_id), source_event) in selected_sources
        .iter()
        .zip(DELIVERY_IDS)
        .zip(&source_events)
    {
        assert_eq!(source["delivery_id"].as_str(), Some(delivery_id));
        assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
        assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["status"],
            "verified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["signature_included"],
            false
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after refusal"),
        journal_before,
        "credential pipe failure must not alter selected source rows"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixed-origin calls must remain unreachable"]
fn actual_reconcile_cli_process_redacts_malformed_open_credential_pipe_and_binds_sources() {
    const DELIVERY_IDS: [&str; 2] = [
        "cli-malformed-credential-pipe-a",
        "cli-malformed-credential-pipe-b",
    ];
    const MALFORMED_CREDENTIAL: &[u8] =
        b"synthetic-malformed-credential MUST_NOT_LEAK_continuation270";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (index, delivery_id) in DELIVERY_IDS.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER + index as u64, "state": "open"}
        }))
        .expect("serialize attested issue delivery");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());
    let source_events = EventJournal::open(&journal_path)
        .replay()
        .expect("read attested receiver journal rows");
    assert_eq!(source_events.len(), DELIVERY_IDS.len());
    let journal_before = fs::read(&journal_path).expect("read receiver journal before refusal");

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(MALFORMED_CREDENTIAL)
        .expect("write synthetic malformed bytes to an open anonymous pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args(DELIVERY_IDS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler with a real open pipe containing malformed bytes");

    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read back credential bytes after structured refusal");
    assert!(
        remaining.is_empty(),
        "the child did not consume the malformed pipe"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("malformed credential returns structured refusal JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], DELIVERY_IDS.len());
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("results").is_none());
    assert_eq!(value["error"]["code"], "host_credential_pipe_read_failed");
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_attempted"],
        true
    );
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_succeeded"],
        false
    );
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
    assert_eq!(value["error"]["details"]["canonical_state_created"], false);
    assert_eq!(
        value["error"]["details"]["reader_error"],
        "host credential input is invalid; inspect it without exposing its value"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let marker =
        std::str::from_utf8(MALFORMED_CREDENTIAL).expect("synthetic malformed credential is UTF-8");
    assert!(!stdout.contains(marker));
    assert!(!stderr.contains(marker));

    let selected_sources = value["selected_event_sources"]
        .as_array()
        .expect("malformed pipe refusal retains selected source identities");
    assert_eq!(selected_sources.len(), DELIVERY_IDS.len());
    for ((source, delivery_id), source_event) in selected_sources
        .iter()
        .zip(DELIVERY_IDS)
        .zip(&source_events)
    {
        assert_eq!(source["delivery_id"].as_str(), Some(delivery_id));
        assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
        assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["status"],
            "verified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["signature_included"],
            false
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after refusal"),
        journal_before,
        "malformed credential refusal must not mutate selected source rows"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixed-origin calls must remain unreachable"]
fn actual_reconcile_cli_process_times_out_open_credential_pipe_and_binds_sources() {
    const DELIVERY_IDS: [&str; 2] = [
        "cli-credential-pipe-timeout-a",
        "cli-credential-pipe-timeout-b",
    ];

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (index, delivery_id) in DELIVERY_IDS.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER + index as u64, "state": "open"}
        }))
        .expect("serialize attested issue delivery");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());
    let source_events = EventJournal::open(&journal_path)
        .replay()
        .expect("read attested receiver journal rows");
    assert_eq!(source_events.len(), DELIVERY_IDS.len());
    let journal_before = fs::read(&journal_path).expect("read receiver journal before refusal");

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args(DELIVERY_IDS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler while the anonymous credential pipe remains open");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(1),
        "the read returned before the pipe timeout: {elapsed:?}"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("credential pipe timeout returns structured refusal JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], DELIVERY_IDS.len());
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("results").is_none());
    assert_eq!(value["error"]["code"], "host_credential_pipe_read_failed");
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_attempted"],
        true
    );
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_succeeded"],
        false
    );
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
    assert_eq!(value["error"]["details"]["canonical_state_created"], false);
    assert_eq!(
        value["error"]["details"]["reader_error"],
        "timed out waiting for host credential pipe EOF"
    );

    drop(credential_writer);
    let mut remaining = Vec::new();
    let mut credential_reader = credential_reader;
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read back credential bytes after timeout");
    assert!(
        remaining.is_empty(),
        "the timed-out pipe contained no bytes"
    );

    let selected_sources = value["selected_event_sources"]
        .as_array()
        .expect("credential pipe timeout retains selected source identities");
    assert_eq!(selected_sources.len(), DELIVERY_IDS.len());
    for ((source, delivery_id), source_event) in selected_sources
        .iter()
        .zip(DELIVERY_IDS)
        .zip(&source_events)
    {
        assert_eq!(source["delivery_id"].as_str(), Some(delivery_id));
        assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
        assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["status"],
            "verified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["signature_included"],
            false
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after refusal"),
        journal_before,
        "credential pipe timeout must not mutate selected source rows"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixed-origin calls must remain unreachable"]
fn actual_reconcile_cli_process_refuses_oversized_credential_pipe_without_leaking_bytes() {
    const DELIVERY_IDS: [&str; 2] = [
        "cli-oversized-credential-pipe-a",
        "cli-oversized-credential-pipe-b",
    ];
    const SENTINEL: &[u8] = b"MUST_NOT_LEAK_continuation272";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (index, delivery_id) in DELIVERY_IDS.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER + index as u64, "state": "open"}
        }))
        .expect("serialize attested issue delivery");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());
    let source_events = EventJournal::open(&journal_path)
        .replay()
        .expect("read attested receiver journal rows");
    assert_eq!(source_events.len(), DELIVERY_IDS.len());
    let journal_before = fs::read(&journal_path).expect("read receiver journal before refusal");

    let mut oversized_credential = vec![b'x'; 4096];
    oversized_credential.extend_from_slice(SENTINEL);
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(&oversized_credential)
        .expect("write a synthetic credential longer than the published byte limit");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args(DELIVERY_IDS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler with an oversized credential on an anonymous pipe");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("oversized credential returns structured refusal JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], DELIVERY_IDS.len());
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("results").is_none());
    assert_eq!(value["error"]["code"], "host_credential_pipe_read_failed");
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_attempted"],
        true
    );
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_succeeded"],
        false
    );
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
    assert_eq!(value["error"]["details"]["canonical_state_created"], false);
    assert_eq!(
        value["error"]["details"]["reader_error"],
        "host credential input exceeds the configured byte limit"
    );
    let sentinel = std::str::from_utf8(SENTINEL).expect("synthetic sentinel is UTF-8");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(sentinel));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(sentinel));

    let selected_sources = value["selected_event_sources"]
        .as_array()
        .expect("oversized credential refusal retains selected source identities");
    assert_eq!(selected_sources.len(), DELIVERY_IDS.len());
    for ((source, delivery_id), source_event) in selected_sources
        .iter()
        .zip(DELIVERY_IDS)
        .zip(&source_events)
    {
        assert_eq!(source["delivery_id"].as_str(), Some(delivery_id));
        assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
        assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["status"],
            "verified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["signature_included"],
            false
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after refusal"),
        journal_before,
        "oversized credential refusal must not mutate selected source rows"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixed-origin calls must remain unreachable"]
fn actual_reconcile_cli_process_refuses_named_fifo_as_host_credential_source() {
    const DELIVERY_IDS: [&str; 2] = ["cli-named-fifo-credential-a", "cli-named-fifo-credential-b"];

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    let fifo_path = scratch.path().join("credential.fifo");
    write_secret(&secret);
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (index, delivery_id) in DELIVERY_IDS.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER + index as u64, "state": "open"}
        }))
        .expect("serialize attested issue delivery");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());
    let source_events = EventJournal::open(&journal_path)
        .replay()
        .expect("read attested receiver journal rows");
    assert_eq!(source_events.len(), DELIVERY_IDS.len());
    let journal_before = fs::read(&journal_path).expect("read receiver journal before refusal");

    let fifo_c_path = std::ffi::CString::new(fifo_path.to_string_lossy().as_bytes())
        .expect("temporary FIFO path contains no NUL byte");
    assert_eq!(
        unsafe { libc::mkfifo(fifo_c_path.as_ptr(), 0o600) },
        0,
        "create a private named FIFO for the descriptor-shape refusal"
    );
    let credential_reader = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo_path)
        .expect("open the named FIFO without a writer or blocking");
    let fd_flags = unsafe { libc::fcntl(credential_reader.as_raw_fd(), libc::F_GETFD) };
    assert!(fd_flags >= 0, "inspect the named FIFO descriptor flags");
    assert_eq!(
        unsafe {
            libc::fcntl(
                credential_reader.as_raw_fd(),
                libc::F_SETFD,
                fd_flags & !libc::FD_CLOEXEC,
            )
        },
        0,
        "inherit the named FIFO descriptor into the reconciler process"
    );

    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args(DELIVERY_IDS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler with a named FIFO descriptor");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "the descriptor-shape refusal unexpectedly waited: {elapsed:?}"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("named FIFO returns structured credential-source refusal JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], DELIVERY_IDS.len());
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("results").is_none());
    assert_eq!(value["error"]["code"], "host_credential_pipe_read_failed");
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_attempted"],
        true
    );
    assert_eq!(
        value["error"]["details"]["credential_pipe_read_succeeded"],
        false
    );
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
    assert_eq!(value["error"]["details"]["canonical_state_created"], false);
    assert_eq!(
        value["error"]["details"]["reader_error"],
        "host credential input must be an anonymous pipe"
    );

    let selected_sources = value["selected_event_sources"]
        .as_array()
        .expect("named FIFO refusal retains selected source identities");
    assert_eq!(selected_sources.len(), DELIVERY_IDS.len());
    for ((source, delivery_id), source_event) in selected_sources
        .iter()
        .zip(DELIVERY_IDS)
        .zip(&source_events)
    {
        assert_eq!(source["delivery_id"].as_str(), Some(delivery_id));
        assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
        assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["status"],
            "verified"
        );
        assert_eq!(
            source["journal_source"]["receiver_attestation"]["signature_included"],
            false
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("read receiver journal after refusal"),
        journal_before,
        "named FIFO refusal must not mutate selected source rows"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_untrusted_attestation_before_host_credential_read() {
    const DELIVERY_ID: &str = "cli-untrusted-attestation-before-credential";
    const TOKEN: &[u8] = b"synthetic-untrusted-attestation-token-012345";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    let signing_seed_path = scratch.path().join("receiver-signing.seed");
    write_secret(&secret);
    let signer = SigningKey::from_bytes(&[0x5a; 32]);
    write_signing_seed(&signing_seed_path, &[0x5a; 32]);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue delivery");
    let (mut receiver, address) =
        start_ready_receiver_with_attestation_key(&secret, &journal_path, &signing_seed_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    read_journaled_webhook_event_verified(
        &EventJournal::open_webhook_inbox(&journal_path),
        DELIVERY_ID,
        &[signer.verifying_key()],
    )
    .expect("receiver event is attested by the expected fixture key");

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN)
        .expect("write synthetic host credential to private pipe");
    drop(credential_writer);

    let untrusted_key = SigningKey::from_bytes(&[0xa5; 32]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .arg("--trusted-webhook-attestation-public-key-hex")
        .arg(hex::encode(untrusted_key.verifying_key().as_bytes()))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler with an untrusted public key");

    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read back credential after the reconciler refuses the untrusted key");
    assert_eq!(
        remaining, TOKEN,
        "attestation refusal consumed the credential"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("synthetic-untrusted-attestation-token")
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("untrusted attestation returns structured refusal JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(
        value["error"]["code"],
        "host_journal_attestation_untrusted_key"
    );
    assert_eq!(value["error"]["details"]["host_credential_read"], false);
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_semantically_tampered_attestation_before_host_credential_read()
 {
    const DELIVERY_ID: &str = "cli-semantic-tamper-before-credential";
    const TOKEN: &[u8] = b"synthetic-tampered-attestation-token-012345";
    const TAMPERED_ISSUE_NUMBER: u64 = ISSUE_NUMBER + 1;

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    let signing_seed_path = scratch.path().join("receiver-signing.seed");
    write_secret(&secret);
    let signer = SigningKey::from_bytes(&[0x5a; 32]);
    write_signing_seed(&signing_seed_path, &[0x5a; 32]);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed issue delivery");
    let (mut receiver, address) =
        start_ready_receiver_with_attestation_key(&secret, &journal_path, &signing_seed_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let journal = EventJournal::open(&journal_path);
    let mut events = journal.replay().expect("replay receiver-signed delivery");
    assert_eq!(events.len(), 1);
    let original_signature = events[0]
        .receiver_attestation
        .as_ref()
        .expect("receiver persisted its attestation")
        .signature_hex
        .clone();
    events[0].payload["payload"]["issue"]["number"] = json!(TAMPERED_ISSUE_NUMBER);
    let hash_material = json!({
        "sequence": events[0].sequence,
        "event_id": events[0].event_id,
        "idempotency_key": events[0].idempotency_key,
        "kind": events[0].kind,
        "payload": events[0].payload,
        "previous_hash": events[0].previous_hash,
        "receiver_attestation": events[0].receiver_attestation,
    });
    events[0].hash =
        sha256(&serde_json::to_vec(&hash_material).expect("serialize tampered row hash material"));
    let mut journal_bytes = serde_json::to_vec(&events[0]).expect("serialize tampered event row");
    journal_bytes.push(b'\n');
    fs::write(&journal_path, &journal_bytes).expect("write hash-chain-valid tampered event row");
    let replayed = journal
        .replay()
        .expect("tampered payload retains a valid unkeyed hash chain");
    assert_eq!(replayed.len(), 1);
    assert_eq!(
        replayed[0].payload["payload"]["issue"]["number"],
        TAMPERED_ISSUE_NUMBER
    );
    assert_eq!(
        replayed[0]
            .receiver_attestation
            .as_ref()
            .expect("tampered row still carries the receiver signature")
            .signature_hex,
        original_signature,
        "the semantic payload changed while the original signature remained"
    );
    let journal_before_cli = fs::read(&journal_path).expect("snapshot tampered journal");

    let (fixture_finished, fixture) = start_scripted_cli_http_fixture_until_finished(vec![(
        200,
        json!({
            "number": TAMPERED_ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-27T12:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{TAMPERED_ISSUE_NUMBER}")
        }),
        Vec::new(),
    )]);

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN)
        .expect("write synthetic host credential to private pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(trusted_attestation_key_args(&signer))
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run reconciler with the correct receiver public key");
    fixture_finished
        .send(())
        .expect("signal that the reconciler CLI has exited");
    let requests = fixture.join().expect("Forgejo API fixture completed");

    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read back credential after the reconciler refuses the invalid signature");
    assert_eq!(
        remaining, TOKEN,
        "signature refusal consumed the credential"
    );
    assert!(
        requests.is_empty(),
        "tampered event triggered an upstream API call: {requests:?}"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("synthetic-tampered-attestation-token")
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid signature returns structured refusal JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "host_journal_attestation_invalid");
    assert_eq!(value["error"]["details"]["host_credential_read"], false);
    assert_eq!(value["error"]["details"]["upstream_calls_attempted"], false);
    assert_eq!(
        fs::read(&journal_path).expect("read journal after refusal"),
        journal_before_cli,
        "refusal must preserve the selected source bytes"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_uses_host_credential_fd_for_receiver_journal() {
    const DELIVERY_ID: &str = "cli-authenticated-41";
    const TOKEN: &str = "synthetic-reconcile-token-0123456789";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize synthetic issue webhook payload");
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        }),
        format!("token {TOKEN}"),
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic token to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn actual reconciliation CLI with private credential pipe");
    drop(credential_reader);
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    fixture_finished
        .send(())
        .expect("signal that the reconciliation CLI has exited");
    let requests = fixture.join().expect("authenticated API fixture completed");

    assert!(
        !timed_out,
        "actual reconciliation CLI exceeded its deadline"
    );
    assert_eq!(requests.len(), 1, "only one API request is allowed");
    let (request, authorized) = &requests[0];
    assert!(*authorized, "fixture rejected host Authorization header");
    assert!(
        request
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("authenticated reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["canonical_state"]["state"], "open");
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_invalid_host_credential_inconclusive() {
    const DELIVERY_ID: &str = "cli-invalid-token-41";
    const TOKEN: &str = "synthetic-invalid-token-012345";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize synthetic issue webhook payload");
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        }),
        "token fixture-only-valid-token".to_owned(),
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write invalid synthetic token to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn actual reconciliation CLI with private credential pipe");
    drop(credential_reader);
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    fixture_finished
        .send(())
        .expect("signal that the reconciliation CLI has exited");
    let requests = fixture.join().expect("unauthorized API fixture completed");

    assert!(
        !timed_out,
        "actual reconciliation CLI exceeded its deadline"
    );
    assert_eq!(requests.len(), 1, "only one API request is allowed");
    let (request, authorized) = &requests[0];
    assert!(
        !*authorized,
        "fixture unexpectedly authorized the invalid token"
    );
    assert!(request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("token {TOKEN}")
        })
    }));
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("unauthorized reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_unauthorized");
    assert_eq!(value["error"]["retryable"], false);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn scripted_http_fixture_stays_open_until_explicitly_stopped() {
    let (fixture_finished, fixture) = start_scripted_cli_http_fixture_until_finished(vec![(
        200,
        json!({"ok": true}),
        Vec::new(),
    )]);

    let send = |path: &str| {
        let result = TcpStream::connect(("127.0.0.1", 3130));
        assert!(
            result.is_ok(),
            "fixture must still accept requests until the CLI has exited"
        );
        let mut stream = result.expect("fixture accepted the request");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bound the fixture client read");
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:3130\r\nConnection: close\r\n\r\n"
        )
        .expect("send scripted fixture request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("read scripted fixture response");
        String::from_utf8(response).expect("fixture response is UTF-8")
    };

    let first = send("/first");
    assert!(first.starts_with("HTTP/1.1 200 OK\r\n"));
    thread::sleep(Duration::from_millis(250));
    let late = send("/late");
    assert!(late.starts_with("HTTP/1.1 429 Too Many Requests\r\n"));

    fixture_finished
        .send(())
        .expect("stop fixture only after the client is done");
    let requests = fixture.join().expect("scripted fixture completed");
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /first HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("GET /late HTTP/1.1\r\n"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reports_actionable_upstream_401_without_leaking_credential() {
    const TOKEN: &str = "synthetic-upstream-401-token-012345";
    const BODY_MARKER: &str = "synthetic-upstream-401-body-must-not-leak";
    let deliveries = ["cli-upstream-401-41-a", "cli-upstream-401-41-b"];

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize synthetic issue webhook payload");
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for delivery_id in deliveries {
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());

    let (fixture_finished, fixture) = start_scripted_cli_http_fixture_until_finished(vec![(
        401,
        json!({"message": BODY_MARKER}),
        Vec::new(),
    )]);
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic API token to private pipe");
    drop(credential_writer);

    let child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args(deliveries)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn actual reconciliation CLI with private credential pipe");
    drop(credential_reader);
    let output = child
        .wait_with_output()
        .expect("wait for actual reconciliation CLI");
    fixture_finished
        .send(())
        .expect("stop HTTP observation after the CLI exits");
    let requests = fixture.join().expect("401 API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        1,
        "401 is terminal and duplicate resources share one read"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[0].lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("token {TOKEN}")
        })
    }));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(TOKEN));
    assert!(!stderr.contains(TOKEN));
    assert!(!stdout.contains(BODY_MARKER));
    assert!(!stderr.contains(BODY_MARKER));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("upstream authorization failure returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for (result, delivery_id) in results.iter().zip(deliveries) {
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "upstream_unauthorized");
        assert_eq!(result["error"]["retryable"], false);
        assert!(
            result["error"]["action"]
                .as_str()
                .unwrap()
                .contains("credential")
        );
        assert!(result.get("canonical_state").is_none());
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_stops_batch_reads_after_401_and_resumes_with_rotated_credential() {
    const REJECTED_TOKEN: &str = "synthetic-batch-401-token-rejected-012345";
    const ROTATED_TOKEN: &str = "synthetic-batch-401-token-rotated-67890";
    const BODY_MARKER: &str = "synthetic-batch-401-body-must-not-leak";
    let deliveries = [("cli-batch-401-41", 41_u64), ("cli-batch-401-42", 42_u64)];

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, number)) in deliveries.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": number, "state": "open"}
        }))
        .expect("serialize synthetic issue webhook payload");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable batch before CLI");

    let run_with_token = |token: &str| {
        let mut descriptors = [-1; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
        let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
        credential_writer
            .write_all(token.as_bytes())
            .expect("write synthetic credential to private pipe");
        drop(credential_writer);

        let child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
            .env_clear()
            .arg("--host-credential-fd")
            .arg(credential_reader.as_raw_fd().to_string())
            .args(trusted_attestation_key_args(&signing_key))
            .arg(&journal_path)
            .args(deliveries.iter().map(|(delivery_id, _)| *delivery_id))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn actual reconciliation CLI with private credential pipe");
        drop(credential_reader);
        child
            .wait_with_output()
            .expect("wait for reconciliation CLI")
    };

    let rejected_fixture = start_scripted_cli_http_fixture(
        vec![(401, json!({"message": BODY_MARKER}), Vec::new())],
        Duration::from_millis(250),
    );
    let rejected = run_with_token(REJECTED_TOKEN);
    let rejected_requests = rejected_fixture
        .join()
        .expect("unauthorized fixture completed");
    assert_eq!(rejected.status.code(), Some(1));
    assert_eq!(
        rejected_requests.len(),
        1,
        "a terminal 401 for the configured repository must stop sibling API reads"
    );
    assert!(
        rejected_requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(rejected_requests[0].lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization")
                && value.trim() == format!("token {REJECTED_TOKEN}")
        })
    }));
    let rejected_stdout = String::from_utf8_lossy(&rejected.stdout);
    let rejected_stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(!rejected_stdout.contains(REJECTED_TOKEN));
    assert!(!rejected_stderr.contains(REJECTED_TOKEN));
    assert!(!rejected_stdout.contains(BODY_MARKER));
    assert!(!rejected_stderr.contains(BODY_MARKER));
    let rejected_value: Value =
        serde_json::from_slice(&rejected.stdout).expect("401 batch refusal is structured JSON");
    assert_reconcile_schema_version(&rejected_value);
    assert_eq!(rejected_value["verdict"], "inconclusive");
    assert_eq!(rejected_value["upstream_read_attempts"], 1);
    let rejected_results = rejected_value["results"]
        .as_array()
        .expect("unauthorized batch results array");
    assert_eq!(rejected_results.len(), deliveries.len());
    for (result, (delivery_id, _)) in rejected_results.iter().zip(deliveries) {
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "upstream_unauthorized");
        assert_eq!(result["error"]["retryable"], false);
        assert!(
            result["error"]["action"]
                .as_str()
                .expect("401 action is text")
                .contains("credential")
        );
        assert!(result.get("canonical_state").is_none());
    }
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after terminal 401"),
        journal_before,
        "authentication refusal must not modify the durable inbox"
    );

    let response_for = |number: u64| {
        (
            200,
            json!({
                "number": number,
                "state": "open",
                "updated_at": "2026-10-25T23:00:00Z",
                "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{number}")
            }),
            Vec::new(),
        )
    };
    let resumed_fixture = start_scripted_cli_http_fixture(
        vec![response_for(41), response_for(42)],
        Duration::from_millis(250),
    );
    let resumed = run_with_token(ROTATED_TOKEN);
    let resumed_requests = resumed_fixture
        .join()
        .expect("rotated credential fixture completed");
    assert!(resumed.status.success());
    assert_eq!(resumed_requests.len(), deliveries.len());
    for (request, (_, number)) in resumed_requests.iter().zip(deliveries) {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{number} HTTP/1.1\r\n"
        )));
        assert!(request.lines().any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.trim() == format!("token {ROTATED_TOKEN}")
            })
        }));
    }
    assert!(!String::from_utf8_lossy(&resumed.stdout).contains(ROTATED_TOKEN));
    assert!(!String::from_utf8_lossy(&resumed.stderr).contains(ROTATED_TOKEN));
    let resumed_value: Value =
        serde_json::from_slice(&resumed.stdout).expect("resumed batch is structured JSON");
    assert_reconcile_schema_version(&resumed_value);
    assert_eq!(resumed_value["verdict"], "current_state_read");
    assert_eq!(resumed_value["upstream_read_attempts"], 2);
    for (result, (delivery_id, _)) in resumed_value["results"]
        .as_array()
        .expect("resumed results array")
        .iter()
        .zip(deliveries)
    {
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["canonical_state"]["state"], "open");
    }
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after credential-rotation resume"),
        journal_before,
        "reconciliation and credential rotation are read-only over the inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_preserves_successful_issue_when_pull_read_latches_401() {
    const TOKEN: &str = "synthetic-cross-resource-401-token-012345";
    const BODY_MARKER: &str = "synthetic-cross-resource-401-body-must-not-leak";
    const ISSUE_41_FIRST: &str = "cross-resource-issue-41-first";
    const PULL_41_FIRST: &str = "cross-resource-pull-41-first";
    const ISSUE_42_AFTER_401: &str = "cross-resource-issue-42-after-401";
    const ISSUE_41_REPLAY: &str = "cross-resource-issue-41-replay";
    const PULL_41_REPLAY: &str = "cross-resource-pull-41-replay";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let issue_41 = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": 41, "state": "open"}
    }))
    .expect("serialize issue webhook payload");
    let pull_41 = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {"number": 41, "state": "open", "merged": false}
    }))
    .expect("serialize pull request webhook payload");
    let issue_42 = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": 42, "state": "open"}
    }))
    .expect("serialize sibling issue webhook payload");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (delivery_id, event, body) in [
        (ISSUE_41_FIRST, "issues", issue_41.as_slice()),
        (PULL_41_FIRST, "pull_request", pull_41.as_slice()),
        (ISSUE_42_AFTER_401, "issues", issue_42.as_slice()),
        (ISSUE_41_REPLAY, "issues", issue_41.as_slice()),
        (PULL_41_REPLAY, "pull_request", pull_41.as_slice()),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, event, body));
        assert_eq!(status, 202, "accepted delivery {delivery_id}");
        assert_eq!(accepted["accepted"], true);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot inbox before reconciliation");

    let issue_response = json!({
        "number": 41,
        "state": "closed",
        "updated_at": "2026-10-25T23:15:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/41")
    });
    let fixture = start_scripted_cli_http_fixture(
        vec![
            (200, issue_response, Vec::new()),
            (401, json!({"message": BODY_MARKER}), Vec::new()),
        ],
        Duration::from_millis(200),
    );
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic token to private pipe");
    drop(credential_writer);

    let child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args([
            ISSUE_41_FIRST,
            PULL_41_FIRST,
            ISSUE_42_AFTER_401,
            ISSUE_41_REPLAY,
            PULL_41_REPLAY,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn actual webhook reconciliation CLI");
    drop(credential_reader);
    let output = child
        .wait_with_output()
        .expect("wait for the actual reconciliation CLI");
    let requests = fixture
        .join()
        .expect("mixed-resource API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        2,
        "the 401 latch suppresses later API reads"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/41 HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/41 HTTP/1.1\r\n"
    )));
    for request in &requests {
        assert!(request.lines().any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.trim() == format!("token {TOKEN}")
            })
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(TOKEN));
    assert!(!stderr.contains(TOKEN));
    assert!(!stdout.contains(BODY_MARKER));
    assert!(!stderr.contains(BODY_MARKER));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed-resource batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 5);
    assert_eq!(results[0]["delivery_id"], ISSUE_41_FIRST);
    assert_eq!(results[0]["verdict"], "current_state_read");
    assert_eq!(results[0]["canonical_state"]["state"], "closed");
    assert_eq!(results[1]["delivery_id"], PULL_41_FIRST);
    assert_eq!(results[1]["error"]["code"], "upstream_unauthorized");
    assert!(results[1].get("canonical_state").is_none());
    assert_eq!(results[2]["delivery_id"], ISSUE_42_AFTER_401);
    assert_eq!(results[2]["error"]["code"], "upstream_unauthorized");
    assert!(results[2].get("canonical_state").is_none());
    assert_eq!(results[3]["delivery_id"], ISSUE_41_REPLAY);
    assert_eq!(results[3]["verdict"], "current_state_read");
    assert_eq!(results[3]["canonical_state"]["state"], "closed");
    assert_eq!(results[4]["delivery_id"], PULL_41_REPLAY);
    assert_eq!(results[4]["error"]["code"], "upstream_unauthorized");
    assert!(results[4].get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after mixed-resource reconciliation"),
        journal_before,
        "a successful partial read and a sibling 401 must not mutate the inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_preserves_out_of_scope_refusal_after_401() {
    const IN_SCOPE_DELIVERY_ID: &str = "cli-401-scope-in-41";
    const OUT_OF_SCOPE_DELIVERY_B: &str = "cli-401-scope-out-42";
    const TOKEN: &str = "synthetic-401-scope-token-012345";
    const BODY_MARKER: &str = "synthetic-401-scope-body-must-not-leak";
    let deliveries = [
        (IN_SCOPE_DELIVERY_ID, OWNER, REPOSITORY, 41_u64),
        (
            OUT_OF_SCOPE_DELIVERY_B,
            "untrusted-owner",
            "other-repository",
            42_u64,
        ),
    ];

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, owner, repository, number)) in deliveries.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{owner}/{repository}")},
            "issue": {"number": number, "state": "open"}
        }))
        .expect("serialize mixed-scope issue webhook");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot mixed-scope inbox");

    let (fixture_finished, fixture) = start_scripted_cli_http_fixture_until_finished(vec![(
        401,
        json!({"message": BODY_MARKER}),
        Vec::new(),
    )]);
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic credential to private pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _, _, _)| *delivery_id))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run actual reconciliation CLI for mixed-scope 401 batch");
    drop(credential_reader);
    fixture_finished
        .send(())
        .expect("stop HTTP observation after the CLI exits");
    let requests = fixture.join().expect("401 fixture completed");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        1,
        "out-of-scope event must not trigger an API read"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(requests[0].lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("token {TOKEN}")
        })
    }));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(TOKEN));
    assert!(!stderr.contains(TOKEN));
    assert!(!stdout.contains(BODY_MARKER));
    assert!(!stderr.contains(BODY_MARKER));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed-scope refusal batch is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], IN_SCOPE_DELIVERY_ID);
    assert_eq!(results[0]["error"]["code"], "upstream_unauthorized");
    assert_eq!(results[0]["error"]["retryable"], false);
    assert!(results[0].get("canonical_state").is_none());
    assert_eq!(results[1]["delivery_id"], OUT_OF_SCOPE_DELIVERY_B);
    assert_eq!(results[1]["error"]["code"], "resource_out_of_scope");
    assert_eq!(results[1]["error"]["retryable"], false);
    assert_eq!(
        results[1]["error"]["details"]["configured_repository"],
        "forge-admin/agent-forge-core"
    );
    assert_eq!(
        results[1]["error"]["details"]["event_repository"],
        "untrusted-owner/other-repository"
    );
    assert!(results[1]["error"]["details"].get("resource").is_none());
    assert!(results[1].get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after mixed-scope 401 batch"),
        journal_before
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixtures reserve loopback ports 3130 and 3131"]
fn actual_reconcile_cli_process_does_not_follow_redirect_with_host_credential() {
    const DELIVERY_ID: &str = "cli-redirect-credential-41";
    const TOKEN: &str = "synthetic-redirect-token-0123456789";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize synthetic issue webhook payload");
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let redirect_target = TcpListener::bind("127.0.0.1:3131")
        .expect("private network namespace reserves the synthetic redirect endpoint");
    redirect_target
        .set_nonblocking(true)
        .expect("make redirect target observation bounded");
    let target_observer = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut requests = Vec::new();
        while Instant::now() < deadline {
            match redirect_target.accept() {
                Ok((mut stream, _)) => {
                    requests.push(read_cli_http_request(&mut stream));
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("observe synthetic redirect target: {error}"),
            }
        }
        requests
    });

    let target_url = "http://127.0.0.1:3131/credential-must-not-follow";
    let (stop_fixture, fixture) = start_cli_http_fixture_until_stopped(
        302,
        json!({"message":"synthetic redirect"}),
        vec![("Location".to_owned(), target_url.to_owned())],
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic token to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("spawn actual reconciliation CLI with private credential pipe");
    drop(credential_reader);
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    stop_fixture
        .send(())
        .expect("stop API fixture after the actual CLI process completes");
    let requests = fixture.join().expect("redirect response fixture completed");
    let redirected_requests = target_observer
        .join()
        .expect("redirect target observer completed");

    assert!(
        !timed_out,
        "actual reconciliation CLI must finish before the bounded process deadline"
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(requests[0].lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("token {TOKEN}")
        })
    }));
    assert!(
        redirected_requests.is_empty(),
        "the adapter followed an upstream redirect: {redirected_requests:?}"
    );
    assert!(
        !output.status.success(),
        "a redirect is not a canonical read"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.contains(TOKEN),
        "host token leaked in CLI output"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("redirect refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_http_status");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["http_status"], 302);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_denied_batch_does_not_read_host_credential_fd() {
    const OUT_OF_SCOPE_DELIVERY_A: &str = "cli-denied-out-of-scope-41";
    const OUT_OF_SCOPE_DELIVERY_B: &str = "cli-denied-out-of-scope-42";
    const TOKEN: &[u8] = b"synthetic-unused-batch-token-012345";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let deliveries = [
        (
            OUT_OF_SCOPE_DELIVERY_A,
            json!({
                "action": "opened",
                "repository": {"full_name": "other-owner/other-repository"},
                "issue": {"number": ISSUE_NUMBER, "state": "open"}
            }),
        ),
        (
            OUT_OF_SCOPE_DELIVERY_B,
            json!({
                "action": "opened",
                "repository": {"full_name": "another-owner/another-repository"},
                "issue": {"number": ISSUE_NUMBER + 1, "state": "open"}
            }),
        ),
    ];
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, payload) in deliveries {
        let body = serde_json::to_vec(&payload).expect("serialize synthetic webhook payload");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN)
        .expect("write synthetic token to private pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .arg(&journal_path)
        .arg(OUT_OF_SCOPE_DELIVERY_A)
        .arg(OUT_OF_SCOPE_DELIVERY_B)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the actual reconciliation CLI with a private pipe FD");

    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read back the unconsumed synthetic credential");
    assert_eq!(
        remaining, TOKEN,
        "denied batch consumed the host credential"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-unused-batch-token"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-unused-batch-token"));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("denied reconciliation batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 0);
    let results = value["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], OUT_OF_SCOPE_DELIVERY_A);
    assert_eq!(results[0]["error"]["code"], "resource_out_of_scope");
    assert_eq!(results[1]["delivery_id"], OUT_OF_SCOPE_DELIVERY_B);
    assert_eq!(results[1]["error"]["code"], "resource_out_of_scope");
    assert!(
        results[1]["error"]["details"].get("resource").is_none(),
        "an out-of-scope event must not disclose its resource identity"
    );
    assert!(!output.status.success());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_partial_selection_before_credential_read() {
    const PRESENT_DELIVERY_ID: &str = "cli-partial-selection-present-41";
    const MISSING_DELIVERY_ID: &str = "cli-partial-selection-missing-42";
    const TOKEN: &[u8] = b"synthetic-partial-selection-token-012345";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize in-scope webhook payload");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(PRESENT_DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN)
        .expect("write synthetic credential to the private pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .arg(&journal_path)
        .arg(PRESENT_DELIVERY_ID)
        .arg(MISSING_DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the actual reconciliation CLI with a partially resolvable batch");

    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read back the credential left untouched by failed selection");
    assert_eq!(remaining, TOKEN, "partial batch consumed its credential");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-partial-selection-token"));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("partially resolved batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], 2);
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "delivery_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["delivery_id"],
        MISSING_DELIVERY_ID
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; duplicate selection must not reach the API"]
fn actual_reconcile_cli_process_refuses_duplicate_delivery_selection_before_credential_read() {
    const DELIVERY_ID: &str = "cli-duplicate-selection-41";
    const TOKEN: &[u8] = b"synthetic-duplicate-selection-token-012345";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize synthetic issue webhook payload");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the local Forgejo endpoint");
    listener
        .set_nonblocking(true)
        .expect("make the duplicate-selection API observer nonblocking");
    let api_requests = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut request_lines = Vec::new();
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_cli_http_request(&mut stream);
                    request_lines.push(
                        request
                            .lines()
                            .next()
                            .unwrap_or("<missing request line>")
                            .to_owned(),
                    );
                    stream
                        .write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .expect("return a bounded response to any unexpected API read");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("observe duplicate-selection API request: {error}"),
            }
        }
        request_lines
    });

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN)
        .expect("write synthetic token to private credential pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run actual CLI with a duplicate delivery selection");

    let observed_requests = api_requests
        .join()
        .expect("join local API request observer");
    assert!(
        observed_requests.is_empty(),
        "duplicate selection reached the local Forgejo API: {observed_requests:?}"
    );

    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("read credential left untouched by duplicate selection");
    assert_eq!(
        remaining, TOKEN,
        "duplicate selection consumed host credential"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("synthetic-duplicate-selection-token")
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("duplicate selection returns batch JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["api_version"], "forgejo-rest-v1");
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["requested_delivery_count"], 2);
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "duplicate_delivery_selection");
    assert_eq!(value["error"]["retryable"], false);
    assert!(
        value["error"]["action"]
            .as_str()
            .unwrap()
            .contains("remove")
    );
    assert_eq!(
        value["error"]["details"]["selection"],
        "delivery_ids_must_be_unique"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read webhook journal after refusal"),
        journal_before
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_mixed_batch_limits_credential_to_configured_repository() {
    const IN_SCOPE_DELIVERY_ID: &str = "cli-mixed-in-scope-41";
    const OUT_OF_SCOPE_DELIVERY_B: &str = "cli-mixed-out-of-scope-42";
    const TOKEN: &str = "synthetic-mixed-process-token-012345";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let deliveries = [
        (
            IN_SCOPE_DELIVERY_ID,
            json!({
                "action": "opened",
                "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
                "issue": {"number": ISSUE_NUMBER, "state": "open"}
            }),
        ),
        (
            OUT_OF_SCOPE_DELIVERY_B,
            json!({
                "action": "opened",
                "repository": {"full_name": "other-owner/other-repository"},
                "issue": {"number": ISSUE_NUMBER + 1, "state": "open"}
            }),
        ),
    ];
    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (delivery_id, payload) in deliveries {
        let body = serde_json::to_vec(&payload).expect("serialize synthetic webhook payload");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);

    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-24T22:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        }),
        format!("token {TOKEN}"),
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic token to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(IN_SCOPE_DELIVERY_ID)
        .arg(OUT_OF_SCOPE_DELIVERY_B)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("run the actual reconciliation CLI with a private credential pipe");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    fixture_finished
        .send(())
        .expect("signal that the reconciliation CLI has exited");
    let requests = fixture.join().expect("authenticated API fixture completed");
    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("confirm allowed batch consumed the token to EOF");

    assert!(
        !timed_out,
        "actual reconciliation CLI exceeded its deadline"
    );
    assert_eq!(requests.len(), 1, "only one API request is allowed");
    let (request, authorized) = &requests[0];
    assert!(*authorized, "fixture rejected the synthetic host token");
    assert_eq!(
        remaining, b"",
        "allowed batch did not consume the credential pipe"
    );
    assert!(
        request
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(!request.contains("other-owner/other-repository"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], IN_SCOPE_DELIVERY_ID);
    assert_eq!(results[0]["verdict"], "current_state_read");
    assert_eq!(results[1]["delivery_id"], OUT_OF_SCOPE_DELIVERY_B);
    assert_eq!(results[1]["error"]["code"], "resource_out_of_scope");
    assert_eq!(
        results[1]["error"]["details"]["configured_repository"],
        "forge-admin/agent-forge-core"
    );
    assert_eq!(
        results[1]["error"]["details"]["event_repository"],
        "other-owner/other-repository"
    );
    assert!(
        results[1]["error"]["details"].get("resource").is_none(),
        "the out-of-scope resource identity must not be returned"
    );
    assert!(!output.status.success());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_pull_request_current_state_from_private_http_fixture() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": 23,
            "state": "open",
            "merged": false,
            "draft": true,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize stale synthetic pull request webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event("cli-pr-23", "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let current_merge = "3333333333333333333333333333333333333333";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": 23,
            "state": "closed",
            "merged": true,
            "draft": false,
            "updated_at": "2026-10-23T12:00:00Z",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23",
            "head": {"sha": current_head},
            "base": {"sha": current_base},
            "merge_commit_sha": current_merge
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-pr-23")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("pull request API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n"),
        "unexpected request path: {requests:?}"
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], 23);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["merged"], true);
    assert_eq!(value["canonical_state"]["draft"], false);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        value["review_notification"],
        Value::Null,
        "a synchronize event without review fields must not invent a review notification"
    );
    assert_eq!(value["canonical_state"]["merge_commit_sha"], current_merge);
    assert_eq!(
        value["canonical_state"]["html_url"],
        "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23"
    );
    assert_ne!(
        value["canonical_state"]["head_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_ne!(
        value["canonical_state"]["base_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_missing_or_malformed_current_pull_request_draft() {
    const NUMBER: u64 = 26;
    const DELIVERY_IDS: [&str; 2] = ["cli-pr-missing-draft-26", "cli-pr-string-draft-26"];
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize pull request webhook with a non-draft snapshot");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for delivery_id in DELIVERY_IDS {
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted pull request journal");
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);

    let current_without_draft = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "updated_at": "2026-10-28T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {"sha": "2222222222222222222222222222222222222222"}
    });
    let mut current_with_malformed_draft = current_without_draft.clone();
    current_with_malformed_draft["draft"] = json!("false");
    let fixture = start_cli_http_fixture_responses(vec![
        (200, current_without_draft),
        (200, current_with_malformed_draft),
    ]);

    for delivery_id in DELIVERY_IDS {
        let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
            .env_clear()
            .arg(&journal_path)
            .arg(delivery_id)
            .stdin(Stdio::null())
            .output()
            .expect("run the actual reconciliation CLI process");
        assert!(
            !output.status.success(),
            "missing or string-valued current draft must fail closed"
        );
        let value: Value = serde_json::from_slice(&output.stdout)
            .expect("malformed current PR snapshot refusal is structured JSON");
        assert_reconcile_schema_version(&value);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["delivery_id"], delivery_id);
        assert_eq!(value["upstream_read_attempts"], 1);
        assert_eq!(value["error"]["code"], "invalid_current_state");
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(value["error"]["details"]["field"], "draft");
        assert_eq!(
            value["error"]["action"],
            "inspect the named field in the Forgejo API snapshot and retry only after the source is corrected"
        );
        assert!(
            value.get("canonical_state").is_none(),
            "the webhook draft value must not fill an absent or malformed API field"
        );
    }

    let requests = fixture
        .join()
        .expect("two pull request API reads completed");
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
        )));
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "the anonymous local CLI must not send an authorization header"
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted pull request journal"),
        journal_before,
        "inconclusive reads must not rewrite accepted webhook deliveries"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_projects_current_pull_request_review_teams() {
    const DELIVERY_ID: &str = "cli-pr-review-teams-23";
    const NUMBER: u64 = 23;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize stale PR webhook without reviewer authority fields");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-26T11:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base},
            "requested_reviewers": [
                {"id": 7, "login": "reviewer-z", "email": "private@example.invalid"}
            ],
            "requested_reviewers_teams": [
                {
                    "id": 29,
                    "name": "team-z",
                    "description": "private team detail",
                    "organization": {"username": "private-org"}
                },
                {"id": 4, "name": "team-a", "units": ["repo.code"]}
            ]
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("pull-request reviewer API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous CLI must not send Authorization"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        value["canonical_state"]["requested_reviewers"],
        json!([{"id": 7, "login": "reviewer-z"}])
    );
    assert_eq!(
        value["canonical_state"]["requested_reviewers_teams"],
        json!([
            {"id": 4, "name": "team-a"},
            {"id": 29, "name": "team-z"}
        ])
    );
    let serialized = String::from_utf8(output.stdout).expect("CLI output is UTF-8");
    assert!(!serialized.contains("private@example.invalid"));
    assert!(!serialized.contains("private team detail"));
    assert!(!serialized.contains("private-org"));
    assert!(!serialized.contains("repo.code"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_duplicate_pull_request_review_team_ids() {
    const DELIVERY_ID: &str = "cli-pr-review-teams-duplicate-23";
    const NUMBER: u64 = 23;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize PR webhook for malformed current reviewer-team response");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-26T11:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {"sha": "2222222222222222222222222222222222222222"},
            "requested_reviewers_teams": [
                {"id": 4, "name": "team-a"},
                {"id": 4, "name": "team-b"}
            ]
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("malformed reviewer-team API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "read-only reconciliation must preserve the durable webhook journal"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("refusal returns structured JSON instead of a partial canonical snapshot");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["field"],
        "requested_reviewers_teams"
    );
    assert!(!value["error"]["action"].as_str().unwrap().is_empty());
    assert!(value.get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_reviewer_teams_on_pr_comment_batch_parent() {
    const PULL_DELIVERY_ID: &str = "cli-pr-team-batch-parent-23";
    const COMMENT_DELIVERY_ID: &str = "cli-pr-team-batch-comment-915";
    const NUMBER: u64 = 23;
    const COMMENT_ID: u64 = 915;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let pull_body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize signed PR delivery");
    let comment_body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "open"},
        "comment": {"id": COMMENT_ID, "body": "untrusted PR comment webhook text"},
        "is_pull": true,
        "pull_request": {"number": NUMBER, "state": "open"}
    }))
    .expect("serialize signed PR-conversation comment delivery");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (event, delivery_id, body, expected_sequence) in [
        ("pull_request", PULL_DELIVERY_ID, pull_body, 1),
        ("issue_comment", COMMENT_DELIVERY_ID, comment_body, 2),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, event, &body));
        assert_eq!(status, 202, "receiver response for {event}: {accepted}");
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted PR/comment journal");
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let pr_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}");
    let pull_readback = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "updated_at": "2026-10-26T11:00:00Z",
        "html_url": pr_url,
        "head": {"sha": current_head},
        "base": {"sha": current_base},
        "requested_reviewers": [
            {"id": 7, "login": "reviewer-z", "email": "private@example.invalid"}
        ],
        "requested_reviewers_teams": [
            {"id": 29, "name": "team-z", "description": "private team detail"},
            {"id": 4, "name": "team-a", "organization": {"username": "private-org"}}
        ]
    });
    let fixture = start_cli_http_fixture_responses(vec![
        (200, pull_readback),
        (
            200,
            json!({
                "id": COMMENT_ID,
                "body": "current API comment body is untrusted",
                "updated_at": "2026-10-26T11:01:00Z",
                "html_url": format!("{pr_url}#issuecomment-{COMMENT_ID}"),
                "issue_url": "",
                "pull_request_url": pr_url
            }),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args([PULL_DELIVERY_ID, COMMENT_DELIVERY_ID])
        .stdin(Stdio::null())
        .output()
        .expect("run the actual batch reconciliation CLI");
    let requests = fixture
        .join()
        .expect("PR/comment batch API fixture completed");

    assert!(
        output.status.success(),
        "batch reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.len(),
        2,
        "one PR read and one comment read are expected"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable webhook journal"),
        journal_before,
        "read-only batch reconciliation must preserve the journal"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("batch CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["schema_version"], 23);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], PULL_DELIVERY_ID);
    assert_eq!(results[1]["delivery_id"], COMMENT_DELIVERY_ID);
    let expected_reviewers = json!([{"id": 7, "login": "reviewer-z"}]);
    let expected_teams = json!([
        {"id": 4, "name": "team-a"},
        {"id": 29, "name": "team-z"}
    ]);
    for result in results {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["canonical_state"]["head_sha"], current_head);
        assert_eq!(result["canonical_state"]["base_sha"], current_base);
        assert_eq!(
            result["canonical_state"]["requested_reviewers"],
            expected_reviewers
        );
        assert_eq!(
            result["canonical_state"]["requested_reviewers_teams"],
            expected_teams
        );
    }
    assert_eq!(
        results[1]["canonical_state"]["comment"]["body_trust"],
        "untrusted_user_content"
    );
    let serialized = String::from_utf8(output.stdout).expect("CLI output is UTF-8");
    assert!(!serialized.contains("private@example.invalid"));
    assert!(!serialized.contains("private team detail"));
    assert!(!serialized.contains("private-org"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_preserves_configured_same_number_pull_request_in_both_batch_orders()
{
    const NUMBER: u64 = 23;
    const OTHER_REPOSITORY: &str = "agent-forge-sibling";
    const FIRST_DELIVERY_ID: &str = "cli-pr-repo-cache-first-23";
    const SECOND_DELIVERY_ID: &str = "cli-pr-repo-cache-second-23";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let first_event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize first signed same-number PR event");
    let second_event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{OTHER_REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "cccccccccccccccccccccccccccccccccccccccc"},
            "base": {"sha": "dddddddddddddddddddddddddddddddddddddddd"}
        }
    }))
    .expect("serialize second signed same-number PR event");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, body)) in [
        (FIRST_DELIVERY_ID, first_event.as_slice()),
        (SECOND_DELIVERY_ID, second_event.as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", body));
        assert_eq!(status, 202, "receiver response for delivery {delivery_id}");
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot both accepted PR deliveries");

    let first_head = "1111111111111111111111111111111111111111";
    let first_base = "2222222222222222222222222222222222222222";
    let first_readback = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "updated_at": "2026-10-26T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": first_head},
        "base": {"sha": first_base},
        "requested_reviewers": [{"id": 7, "login": "first-repo-reviewer"}],
        "requested_reviewers_teams": [{"id": 11, "name": "first-repo-team"}]
    });
    let fixture = start_cli_http_fixture_responses(vec![
        (200, first_readback.clone()),
        (200, first_readback),
    ]);
    let configured_first = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args([FIRST_DELIVERY_ID, SECOND_DELIVERY_ID])
        .stdin(Stdio::null())
        .output()
        .expect("run batch with the configured PR before the out-of-scope PR");
    let foreign_first = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args([SECOND_DELIVERY_ID, FIRST_DELIVERY_ID])
        .stdin(Stdio::null())
        .output()
        .expect("run batch with the out-of-scope PR before the configured PR");
    let requests = fixture
        .join()
        .expect("two ordered in-scope PR reads completed");

    assert_eq!(configured_first.status.code(), Some(1));
    assert_eq!(foreign_first.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        2,
        "each invocation reads only the configured PR, regardless of event order"
    );
    for request in &requests {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
        )));
        assert!(!request.contains(OTHER_REPOSITORY));
    }
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted PR journal"),
        journal_before,
        "read-only batch reconciliation must preserve the inbox"
    );

    let value: Value = serde_json::from_slice(&configured_first.stdout)
        .expect("configured-first batch returns JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], FIRST_DELIVERY_ID);
    assert_eq!(results[0]["resource"]["kind"], "pull_request");
    assert_eq!(results[0]["resource"]["owner"], OWNER);
    assert_eq!(results[0]["resource"]["repository"], REPOSITORY);
    assert_eq!(results[0]["verdict"], "current_state_read");
    assert_eq!(results[0]["canonical_state"]["head_sha"], first_head);
    assert_eq!(results[0]["canonical_state"]["base_sha"], first_base);
    assert_eq!(
        results[0]["canonical_state"]["requested_reviewers"],
        json!([{"id": 7, "login": "first-repo-reviewer"}])
    );
    assert_eq!(
        results[0]["canonical_state"]["requested_reviewers_teams"],
        json!([{"id": 11, "name": "first-repo-team"}])
    );
    assert_eq!(results[1]["delivery_id"], SECOND_DELIVERY_ID);
    assert_eq!(results[1]["error"]["code"], "resource_out_of_scope");
    assert_eq!(results[1]["error"]["retryable"], false);
    let action = results[1]["error"]["action"]
        .as_str()
        .expect("out-of-scope error includes a concrete routing action");
    assert!(action.contains("matching configured repository adapter"));
    assert!(action.contains("do not retry"));
    assert_eq!(
        results[1]["error"]["details"]["configured_repository"],
        format!("{OWNER}/{REPOSITORY}")
    );
    assert_eq!(
        results[1]["error"]["details"]["event_repository"],
        format!("{OWNER}/{OTHER_REPOSITORY}")
    );
    assert!(results[1].get("canonical_state").is_none());
    assert!(results[1]["error"]["details"].get("resource").is_none());

    let reverse_value: Value = serde_json::from_slice(&foreign_first.stdout)
        .expect("out-of-scope-first batch returns JSON");
    assert_reconcile_schema_version(&reverse_value);
    assert_eq!(reverse_value["verdict"], "inconclusive");
    assert_eq!(reverse_value["upstream_read_attempts"], 1);
    let reverse_results = reverse_value["results"]
        .as_array()
        .expect("reverse-order batch results array");
    assert_eq!(reverse_results.len(), 2);
    assert_eq!(reverse_results[0]["delivery_id"], SECOND_DELIVERY_ID);
    assert_eq!(reverse_results[0]["error"]["code"], "resource_out_of_scope");
    assert_eq!(reverse_results[0]["error"]["retryable"], false);
    assert!(reverse_results[0].get("canonical_state").is_none());
    assert_eq!(
        reverse_results[0]["error"]["details"]["configured_repository"],
        format!("{OWNER}/{REPOSITORY}")
    );
    assert_eq!(
        reverse_results[0]["error"]["details"]["event_repository"],
        format!("{OWNER}/{OTHER_REPOSITORY}")
    );
    assert!(
        reverse_results[0]["error"]["details"]
            .get("resource")
            .is_none()
    );
    let reverse_action = reverse_results[0]["error"]["action"]
        .as_str()
        .expect("reverse-order refusal includes a routing action");
    assert!(reverse_action.contains("matching configured repository adapter"));
    assert!(reverse_action.contains("do not retry"));
    assert_eq!(reverse_results[1]["delivery_id"], FIRST_DELIVERY_ID);
    assert_eq!(reverse_results[1]["verdict"], "current_state_read");
    assert_eq!(
        reverse_results[1]["canonical_state"]["head_sha"],
        first_head
    );
    assert_eq!(
        reverse_results[1]["canonical_state"]["base_sha"],
        first_base
    );
    assert_eq!(
        reverse_results[1]["canonical_state"]["requested_reviewers"],
        json!([{"id": 7, "login": "first-repo-reviewer"}])
    );
    assert_eq!(
        reverse_results[1]["canonical_state"]["requested_reviewers_teams"],
        json!([{"id": 11, "name": "first-repo-team"}])
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_foreign_release_before_credential_or_api() {
    const DELIVERY_ID: &str = "foreign-release-before-credential-api-001";
    const TOKEN: &str = "synthetic-foreign-release-scope-token-012345";
    const FOREIGN_REPOSITORY: &str = "outside-org/unconfigured-release-repository";
    const EVENT_PRIVATE_MARKER: &str = "foreign-release-event-private-marker";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let foreign_event = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": FOREIGN_REPOSITORY},
        "release": {
            "id": 617,
            "tag_name": "foreign-release-tag",
            "body": EVENT_PRIVATE_MARKER
        }
    }))
    .expect("serialize signed foreign repository Release event");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(DELIVERY_ID, "release", &foreign_event),
    );
    assert_eq!(
        status, 202,
        "shared authenticated inbox should durably accept a valid foreign event: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted foreign Release");
    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({"id": 617}),
        format!("token {TOKEN}"),
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic credential to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("run reconciliation CLI against foreign Release delivery");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    fixture_finished
        .send(())
        .expect("signal CLI completion to authenticated fixture");
    let requests = fixture
        .join()
        .expect("foreign Release API fixture completed");
    let mut credential_remaining = Vec::new();
    credential_reader
        .read_to_end(&mut credential_remaining)
        .expect("read credential bytes left untouched by foreign Release");

    assert!(
        !timed_out,
        "foreign Release reconciliation exceeded its deadline"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(requests.is_empty(), "foreign Release made an API request");
    assert_eq!(credential_remaining, TOKEN.as_bytes());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_text.contains(TOKEN));
    assert!(!output_text.contains(EVENT_PRIVATE_MARKER));
    assert_eq!(
        fs::read(&journal_path).expect("re-read shared foreign Release inbox"),
        journal_before,
        "reconciliation refusal must not rewrite the accepted event"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign Release returns structured out-of-scope result");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["error"]["code"], "resource_out_of_scope");
    assert_eq!(value["error"]["retryable"], false);
    assert!(
        value["error"]["action"]
            .as_str()
            .is_some_and(|action| action.contains("matching configured repository adapter"))
    );
    assert_eq!(
        value["error"]["details"]["configured_repository"],
        format!("{OWNER}/{REPOSITORY}")
    );
    assert_eq!(
        value["error"]["details"]["event_repository"],
        FOREIGN_REPOSITORY
    );
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["status"],
        "verified"
    );
    assert_eq!(
        value["journal_source"]["receiver_attestation"]["verification_scope"],
        "receiver_signature_only"
    );
}

#[test]
#[ignore = "run in an isolated local fixture; this reconcile-only profile reserves 127.0.0.1:3130"]
fn actual_reconcile_cli_process_org_profile_refuses_foreign_pr_before_credential_or_api() {
    const FOREIGN_DELIVERY_ID: &str = "org-profile-foreign-pr-001";
    const TOKEN: &str = "synthetic-org-scope-token-0123456789";
    const FOREIGN_REPOSITORY: &str = "outside-org/unconfigured-repository";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let foreign_event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": FOREIGN_REPOSITORY},
        "pull_request": {
            "number": 17,
            "state": "open",
            "merged": false,
            "head": {"sha": "cccccccccccccccccccccccccccccccccccccccc"},
            "base": {
                "sha": "dddddddddddddddddddddddddddddddddddddddd",
                "repo": {"full_name": FOREIGN_REPOSITORY}
            }
        }
    }))
    .expect("serialize foreign organization PR event");
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(FOREIGN_DELIVERY_ID, "pull_request", &foreign_event),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot signed foreign PR event");

    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({"number": 17}),
        format!("token {TOKEN}"),
    );
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic credential to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .args(["--reconcile-target-profile", "org-team-review-e2e"])
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .arg(&journal_path)
        .arg(FOREIGN_DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("run the actual CLI against the organization-only profile");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    fixture_finished
        .send(())
        .expect("signal the loopback API fixture after CLI exit");
    let requests = fixture
        .join()
        .expect("organization-profile API fixture completed");
    let mut credential_remaining = Vec::new();
    credential_reader
        .read_to_end(&mut credential_remaining)
        .expect("read credential bytes left untouched by out-of-scope event");

    assert!(!timed_out, "organization-profile CLI exceeded its deadline");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        requests.is_empty(),
        "out-of-scope event made an API request"
    );
    assert_eq!(credential_remaining, TOKEN.as_bytes());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
    assert_eq!(
        fs::read(&journal_path).expect("re-read foreign PR journal"),
        journal_before
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign organization PR returns structured reconciliation result");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], FOREIGN_DELIVERY_ID);
    assert_eq!(value["error"]["code"], "resource_out_of_scope");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert!(value.get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_uses_credential_only_after_refusing_foreign_first_pr() {
    const NUMBER: u64 = 23;
    const OTHER_REPOSITORY: &str = "agent-forge-sibling";
    const FOREIGN_DELIVERY_ID: &str = "cli-pr-foreign-first-23";
    const CONFIGURED_DELIVERY_ID: &str = "cli-pr-configured-second-23";
    const TOKEN: &str = "synthetic-foreign-first-token-012345";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let foreign_event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{OTHER_REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "cccccccccccccccccccccccccccccccccccccccc"},
            "base": {"sha": "dddddddddddddddddddddddddddddddddddddddd"}
        }
    }))
    .expect("serialize foreign-first signed PR event");
    let configured_event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize configured second signed PR event");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, body)) in [
        (FOREIGN_DELIVERY_ID, foreign_event.as_slice()),
        (CONFIGURED_DELIVERY_ID, configured_event.as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot foreign-first PR batch");

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-26T13:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base},
            "requested_reviewers": [{"id": 7, "login": "configured-reviewer"}],
            "requested_reviewers_teams": [{"id": 11, "name": "configured-team"}]
        }),
        format!("token {TOKEN}"),
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic token to private pipe");
    drop(credential_writer);

    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args([FOREIGN_DELIVERY_ID, CONFIGURED_DELIVERY_ID])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.process_group(0);
    let child = command
        .spawn()
        .expect("run CLI with foreign PR first and host credential pipe");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(5));
    fixture_finished
        .send(())
        .expect("signal CLI completion to authenticated fixture");
    let requests = fixture.join().expect("configured PR API fixture completed");
    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("confirm the configured PR read consumed the credential");

    assert!(!timed_out, "foreign-first batch exceeded its deadline");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        1,
        "only the configured PR may reach the API"
    );
    let (request, authorized) = &requests[0];
    assert!(
        *authorized,
        "configured PR request uses the host credential"
    );
    assert!(request.starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(!request.contains(OTHER_REPOSITORY));
    assert_eq!(remaining, b"");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted PR journal"),
        journal_before
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign-first credential batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], FOREIGN_DELIVERY_ID);
    assert_eq!(results[0]["error"]["code"], "resource_out_of_scope");
    assert_eq!(results[0]["error"]["retryable"], false);
    assert!(results[0].get("canonical_state").is_none());
    assert_eq!(results[1]["delivery_id"], CONFIGURED_DELIVERY_ID);
    assert_eq!(results[1]["verdict"], "current_state_read");
    assert_eq!(results[1]["canonical_state"]["head_sha"], current_head);
    assert_eq!(results[1]["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        results[1]["canonical_state"]["requested_reviewers"],
        json!([{"id": 7, "login": "configured-reviewer"}])
    );
    assert_eq!(
        results[1]["canonical_state"]["requested_reviewers_teams"],
        json!([{"id": 11, "name": "configured-team"}])
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_preserves_foreign_first_pr_refusal_after_configured_401() {
    const NUMBER: u64 = 23;
    const OTHER_REPOSITORY: &str = "agent-forge-sibling";
    const FOREIGN_DELIVERY_ID: &str = "cli-pr-foreign-first-401-23";
    const CONFIGURED_DELIVERY_ID: &str = "cli-pr-configured-second-401-23";
    const TOKEN: &str = "synthetic-foreign-first-401-token-012345";
    const BODY_MARKER: &str = "synthetic-foreign-first-401-body-must-not-leak";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let foreign_event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{OTHER_REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "cccccccccccccccccccccccccccccccccccccccc"},
            "base": {"sha": "dddddddddddddddddddddddddddddddddddddddd"}
        }
    }))
    .expect("serialize foreign-first signed PR event");
    let configured_event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize configured second signed PR event");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, body)) in [
        (FOREIGN_DELIVERY_ID, foreign_event.as_slice()),
        (CONFIGURED_DELIVERY_ID, configured_event.as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot foreign-first PR batch");

    let (fixture_finished, fixture) = start_scripted_cli_http_fixture_until_finished(vec![(
        401,
        json!({"message": BODY_MARKER}),
        Vec::new(),
    )]);
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let mut credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic token to private pipe");
    drop(credential_writer);

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .args([FOREIGN_DELIVERY_ID, CONFIGURED_DELIVERY_ID])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run CLI with foreign PR first and an unauthorized configured read");
    fixture_finished
        .send(())
        .expect("signal CLI completion to scripted fixture");
    let requests = fixture.join().expect("configured PR 401 fixture completed");
    let mut remaining = Vec::new();
    credential_reader
        .read_to_end(&mut remaining)
        .expect("confirm configured PR read consumed the credential");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        1,
        "only the configured PR may reach the API"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(!requests[0].contains(OTHER_REPOSITORY));
    assert!(requests[0].lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("token {TOKEN}")
        })
    }));
    assert_eq!(remaining, b"");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(TOKEN));
    assert!(!stderr.contains(TOKEN));
    assert!(!stdout.contains(BODY_MARKER));
    assert!(!stderr.contains(BODY_MARKER));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted PR journal"),
        journal_before
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign-first 401 batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], FOREIGN_DELIVERY_ID);
    assert_eq!(results[0]["error"]["code"], "resource_out_of_scope");
    assert_eq!(results[0]["error"]["retryable"], false);
    assert!(results[0].get("canonical_state").is_none());
    assert_eq!(results[1]["delivery_id"], CONFIGURED_DELIVERY_ID);
    assert_eq!(results[1]["error"]["code"], "upstream_unauthorized");
    assert_eq!(results[1]["error"]["retryable"], false);
    assert!(results[1].get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_fork_pull_request_from_configured_target_api() {
    const NUMBER: u64 = 43;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-fork-pr-43";
    let event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {
            "full_name": format!("{OWNER}/{REPOSITORY}"),
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        },
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {
                "ref": "feature/agent-change",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            },
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }
    }))
    .expect("serialize signed fork pull request notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &event));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable fork PR delivery");
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-25T16:30:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {
                "sha": current_head,
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            },
            "base": {
                "ref": "main",
                "sha": current_base,
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciler for the fork PR delivery");
    let requests = fixture.join().expect("fork PR API fixture completed");

    assert!(
        output.status.success(),
        "fork PR reconciliation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("fork PR reconciliation returns JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["canonical_state"]["state"], "open");
    assert_eq!(value["canonical_state"]["merged"], false);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_ne!(
        value["canonical_state"]["head_sha"], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "reconciliation must return the API's current fork head, not the stale webhook head"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read fork PR delivery journal"),
        journal_before,
        "read-only reconciliation must preserve the durable webhook"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_fork_pull_request_with_host_credential_fd() {
    const NUMBER: u64 = 45;
    const DELIVERY_ID: &str = "cli-private-fork-pr-45";
    const TOKEN: &str = "synthetic-private-fork-pr-token-012345";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {
            "full_name": format!("{OWNER}/{REPOSITORY}"),
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        },
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {
                "ref": "feature/private-fork-change",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            },
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }
    }))
    .expect("serialize private target pull request with a fork head");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &event));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot private fork PR delivery");

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-25T21:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {
                "sha": current_head,
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            },
            "base": {
                "ref": "main",
                "sha": current_base,
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }),
        format!("token {TOKEN}"),
    );

    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write synthetic Forgejo credential to private pipe");
    drop(credential_writer);

    let child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn reconciler with host-owned credential pipe");
    drop(credential_reader);
    let output = child.wait_with_output().expect("wait for fork PR readback");
    fixture_finished
        .send(())
        .expect("signal that the reconciliation CLI has exited");
    let requests = fixture
        .join()
        .expect("authenticated fork PR fixture completed");

    assert_eq!(requests.len(), 1, "only one API request is allowed");
    let (request, authorized) = &requests[0];
    assert!(
        *authorized,
        "private target API fixture rejected the host token"
    );
    assert!(request.starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        output.status.success(),
        "fork PR read failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(TOKEN), "host token leaked to stdout");
    assert!(!stderr.contains(TOKEN), "host token leaked to stderr");

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("authenticated fork PR readback returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_ne!(
        value["canonical_state"]["head_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_ne!(
        value["canonical_state"]["base_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read private fork PR inbox"),
        journal_before,
        "credentialed read-only reconciliation must leave the inbox unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_fork_pull_request_with_invalid_host_credential() {
    const NUMBER: u64 = 46;
    const DELIVERY_ID: &str = "cli-private-fork-pr-invalid-token-46";
    const TOKEN: &str = "synthetic-private-fork-pr-invalid-token-012345";
    const EXPECTED_TOKEN: &str = "synthetic-private-fork-pr-valid-token-67890";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {
            "full_name": format!("{OWNER}/{REPOSITORY}"),
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        },
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {
                "ref": "feature/private-fork-change",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            },
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }
    }))
    .expect("serialize private target pull request with a fork head");

    let (mut receiver, address, signing_key) =
        start_ready_attested_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &event));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot private fork PR delivery");

    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({"message": "synthetic fixture rejection"}),
        format!("token {EXPECTED_TOKEN}"),
    );
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    let credential_reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let mut credential_writer = unsafe { File::from_raw_fd(descriptors[1]) };
    credential_writer
        .write_all(TOKEN.as_bytes())
        .expect("write invalid synthetic host credential to private pipe");
    drop(credential_writer);

    let child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg("--host-credential-fd")
        .arg(credential_reader.as_raw_fd().to_string())
        .args(trusted_attestation_key_args(&signing_key))
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn reconciler with invalid host credential pipe");
    drop(credential_reader);
    let output = child
        .wait_with_output()
        .expect("wait for invalid-credential fork PR readback");
    fixture_finished
        .send(())
        .expect("signal that the reconciliation CLI has exited");
    let requests = fixture
        .join()
        .expect("invalid-credential fork PR fixture completed");

    assert_eq!(requests.len(), 1, "only one API request is allowed");
    let (request, authorized) = &requests[0];
    assert!(
        !authorized,
        "fixture unexpectedly authorized the invalid token"
    );
    assert!(request.starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("token {TOKEN}")
        })
    }));
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(TOKEN),
        "invalid credential leaked to stdout"
    );
    assert!(
        !stderr.contains(TOKEN),
        "invalid credential leaked to stderr"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("unauthorized fork PR readback returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_unauthorized");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read private fork PR inbox"),
        journal_before,
        "unauthorized read-only reconciliation must leave the inbox unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_fork_pull_request_inconclusive_without_host_credential() {
    const NUMBER: u64 = 47;
    const DELIVERY_ID: &str = "cli-private-fork-pr-no-token-47";
    const EXPECTED_TOKEN: &str = "synthetic-private-fork-pr-required-token-13579";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {
            "full_name": format!("{OWNER}/{REPOSITORY}"),
            "name": REPOSITORY,
            "owner": {"login": OWNER}
        },
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {
                "ref": "feature/private-fork-change",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {
                    "full_name": "contributor/agent-forge-core",
                    "name": "agent-forge-core",
                    "owner": {"login": "contributor"}
                }
            },
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }
    }))
    .expect("serialize private target pull request with a fork head");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &event));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot private fork PR delivery");

    let (fixture_finished, fixture) = start_authenticated_cli_http_fixture_until_finished(
        json!({"message": "synthetic fixture requires host authorization"}),
        format!("token {EXPECTED_TOKEN}"),
    );
    let child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn reconciler without a host credential descriptor");
    let output = child
        .wait_with_output()
        .expect("wait for unauthenticated fork PR readback");
    fixture_finished
        .send(())
        .expect("signal that the reconciliation CLI has exited");
    let requests = fixture
        .join()
        .expect("unauthenticated fork PR fixture completed");

    assert_eq!(requests.len(), 1, "only one API request is allowed");
    let (request, authorized) = &requests[0];
    assert!(
        !authorized,
        "fixture unexpectedly authorized an anonymous request"
    );
    assert!(request.starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(!request.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(name, _)| name.eq_ignore_ascii_case("authorization"))
    }));
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(EXPECTED_TOKEN));
    assert!(!stderr.contains(EXPECTED_TOKEN));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("unauthenticated fork PR readback returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_unauthorized");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read private fork PR inbox"),
        journal_before,
        "unauthenticated read-only reconciliation must leave the inbox unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_contradictory_current_base_repository_ids() {
    const NUMBER: u64 = 45;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-pr-contradictory-current-base-repository-ids-45";
    let event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {
                "ref": "feature/agent-change",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {"full_name": "contributor/agent-forge-core"}
            },
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {"full_name": format!("{OWNER}/{REPOSITORY}")}
            }
        }
    }))
    .expect("serialize signed fork PR notification with configured base");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &event));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable fork PR delivery");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-25T17:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {
                "sha": "1111111111111111111111111111111111111111",
                "repo": {"full_name": "contributor/agent-forge-core"}
            },
            "base": {
                "ref": "main",
                "sha": "2222222222222222222222222222222222222222",
                "repo_id": 18,
                "repo": {
                    "id": 17,
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciler against contradictory numeric repository ids");
    let requests = fixture
        .join()
        .expect("contradictory current-base API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("contradictory current-base IDs refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read fork PR delivery journal"),
        journal_before,
        "contradictory current base IDs must not rewrite the durable webhook"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_foreign_current_base_repository_after_fork_delivery() {
    const NUMBER: u64 = 44;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-fork-pr-foreign-current-base-44";
    let event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {
                "ref": "feature/agent-change",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {"full_name": "contributor/agent-forge-core"}
            },
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {"full_name": format!("{OWNER}/{REPOSITORY}")}
            }
        }
    }))
    .expect("serialize signed fork PR notification with target base");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &event));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable fork PR delivery");
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-25T17:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {
                "sha": "1111111111111111111111111111111111111111",
                "repo": {"full_name": "contributor/agent-forge-core"}
            },
            "base": {
                "ref": "main",
                "sha": "2222222222222222222222222222222222222222",
                "repo": {
                    "full_name": "other-owner/other-repository",
                    "name": "other-repository",
                    "owner": {"login": "other-owner"}
                }
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciler against foreign current base fixture");
    let requests = fixture
        .join()
        .expect("foreign current-base API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign current base refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read fork PR delivery journal"),
        journal_before,
        "a foreign current base must not rewrite the durable webhook"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_current_base_repository_without_identity() {
    const NUMBER: u64 = 43;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-pr-current-base-without-identity-43";
    let event = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {
                "ref": "feature/agent-change",
                "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "repo": {"full_name": "contributor/agent-forge-core"}
            },
            "base": {
                "ref": "main",
                "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "repo": {"full_name": format!("{OWNER}/{REPOSITORY}")}
            }
        }
    }))
    .expect("serialize signed fork PR notification with configured base");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &event));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable fork PR delivery");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-25T17:30:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {
                "sha": "1111111111111111111111111111111111111111",
                "repo": {"full_name": "contributor/agent-forge-core"}
            },
            "base": {
                "ref": "main",
                "sha": "2222222222222222222222222222222222222222",
                "repo": {}
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciler against empty base repository metadata");
    let requests = fixture
        .join()
        .expect("empty current-base API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("missing base repository identity refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "base.repo");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted fork PR delivery"),
        journal_before,
        "invalid current base metadata must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_routes_approved_review_to_pull_request_api() {
    const NUMBER: u64 = 26;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-approved-review-26";
    let body = serde_json::to_vec(&json!({
        "action": "reviewed",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        },
        "review": {"type": "pull_request_review_approved", "content": "approved"}
    }))
    .expect("serialize valid approved-review webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "pull_request_approved", &body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted review journal");

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "closed",
            "merged": true,
            "merge_commit_sha": "3333333333333333333333333333333333333333",
            "updated_at": "2026-10-25T14:30:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for the review delivery");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("review reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["merged"], true);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_ne!(value["canonical_state"]["state"], "open");
    assert_ne!(
        value["canonical_state"]["head_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted review journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_routes_rejected_review_to_pull_request_api() {
    const NUMBER: u64 = 28;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-rejected-review-28";
    let body = serde_json::to_vec(&json!({
        "action": "reviewed",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        },
        "review": {"type": "pull_request_review_rejected", "content": "changes requested"}
    }))
    .expect("serialize valid rejected-review webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "pull_request_rejected", &body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted review journal");

    let current_head = "3333333333333333333333333333333333333333";
    let current_base = "4444444444444444444444444444444444444444";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-25T16:30:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for the rejected-review delivery");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("rejected-review reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["canonical_state"]["state"], "open");
    assert_eq!(value["canonical_state"]["merged"], false);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted review journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_issue_and_pull_request_number_collision_separate() {
    const NUMBER: u64 = 27;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_delivery = "cli-same-number-issue-27";
    let pull_delivery = "cli-same-number-pull-27";
    let issue_body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"}
    }))
    .expect("serialize signed issue event");
    let pull_body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize signed pull request event");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, event, body) in [
        (issue_delivery, "issues", issue_body.as_slice()),
        (pull_delivery, "pull_request", pull_body.as_slice()),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, event, body));
        assert_eq!(status, 202, "event {event}");
        assert_eq!(accepted["accepted"], true, "event {event}");
        assert_eq!(accepted["duplicate"], false, "event {event}");
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted issue and PR events");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                200,
                json!({
                    "number": NUMBER,
                    "state": "open",
                    "updated_at": "2026-10-25T15:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{NUMBER}")
                }),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": NUMBER,
                    "state": "closed",
                    "merged": true,
                    "merge_commit_sha": "3333333333333333333333333333333333333333",
                    "updated_at": "2026-10-25T15:01:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
                    "head": {"sha": "1111111111111111111111111111111111111111"},
                    "base": {"sha": "2222222222222222222222222222222222222222"}
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(50),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(issue_delivery)
        .arg(pull_delivery)
        .stdin(Stdio::null())
        .output()
        .expect("run batch reconciliation CLI for same-number resources");
    let requests = fixture.join().expect("issue and PR API fixture completed");

    assert!(
        output.status.success(),
        "batch reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("same-number batch reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], issue_delivery);
    assert_eq!(results[0]["resource"]["kind"], "issue");
    assert_eq!(results[0]["canonical_state"]["state"], "open");
    assert!(results[0]["canonical_state"]["head_sha"].is_null());
    assert_eq!(results[1]["delivery_id"], pull_delivery);
    assert_eq!(results[1]["resource"]["kind"], "pull_request");
    assert_eq!(results[1]["canonical_state"]["state"], "closed");
    assert_eq!(results[1]["canonical_state"]["merged"], true);
    assert_eq!(
        results[1]["canonical_state"]["head_sha"],
        "1111111111111111111111111111111111111111"
    );
    assert_eq!(
        results[1]["canonical_state"]["merge_commit_sha"],
        "3333333333333333333333333333333333333333"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read journal after batch reconciliation"),
        journal_before,
        "reconciliation must not rewrite the accepted event records"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_branch_404_ambiguous_and_inconclusive() {
    const DELIVERY_ID: &str = "cli-branch-404-ambiguous";
    const BRANCH: &str = "feature/release/agent-proof";
    const NOT_FOUND_BODY: &str = "synthetic-branch-404-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "before": "1111111111111111111111111111111111111111",
        "after": "0000000000000000000000000000000000000000",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "commits": [],
        "total_commits": 0
    }))
    .expect("serialize signed branch-deletion webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "push", &body));
    assert_eq!(
        status, 202,
        "valid branch push should be journaled: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted push journal");

    let fixture = start_cli_http_fixture(404, json!({"message":NOT_FOUND_BODY}), 1);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual branch reconciliation CLI");
    let requests = fixture.join().expect("branch API 404 fixture completed");

    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after branch 404"),
        journal_before,
        "a hidden or deleted branch must not rewrite the durable inbox"
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the local reconciliation CLI must not send an authorization header"
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(NOT_FOUND_BODY));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("404 refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["details"]["resource"]["kind"], "branch");
    assert_eq!(value["error"]["details"]["resource"]["branch"], BRANCH);
    assert!(value.get("canonical_state").is_none());
    let action = value["error"]["action"]
        .as_str()
        .expect("404 has a remediation action");
    assert!(
        action.contains("branch name"),
        "unexpected action: {action}"
    );
    assert!(
        action.contains("does not distinguish"),
        "the action must preserve 404 ambiguity: {action}"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_negative_caches_missing_branch_across_deliveries() {
    const BRANCH: &str = "feature/release/agent-proof";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = |before: &str| {
        serde_json::to_vec(&json!({
            "ref": format!("refs/heads/{BRANCH}"),
            "before": before,
            "after": "0000000000000000000000000000000000000000",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "commits": [],
            "total_commits": 0
        }))
        .expect("serialize signed branch deletion webhook")
    };
    let first_body = body("1111111111111111111111111111111111111111");
    let second_body = body("2222222222222222222222222222222222222222");
    let deliveries = [
        ("cli-missing-branch-a", first_body.as_slice()),
        ("cli-missing-branch-b", second_body.as_slice()),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (expected_sequence, (delivery_id, body)) in deliveries.iter().enumerate() {
        let (status, accepted) = send(address, &request_event(delivery_id, "push", body));
        assert_eq!(status, 202, "delivery {delivery_id}");
        assert_eq!(accepted["accepted"], true, "delivery {delivery_id}");
        assert_eq!(accepted["duplicate"], false, "delivery {delivery_id}");
        assert_eq!(accepted["sequence"], expected_sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted branch batch");

    let fixture = start_scripted_cli_http_fixture(
        vec![(
            404,
            json!({"message":"synthetic-missing-branch-body-must-not-leak"}),
            Vec::new(),
        )],
        Duration::from_millis(250),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _)| *delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run repeated missing-branch reconciliation batch");
    let requests = fixture
        .join()
        .expect("missing-branch API fixture completed");

    assert_eq!(
        fs::read(&journal_path).expect("re-read branch journal after 404 batch"),
        journal_before,
        "reconciliation must not rewrite the accepted webhook inbox"
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        1,
        "one missing branch is fetched once per batch"
    );
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the local CLI must not send an authorization header"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("synthetic-missing-branch-body-must-not-leak"));

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("batch 404 refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for (result, (delivery_id, _)) in results.iter().zip(deliveries) {
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "resource_not_found");
        assert_eq!(result["error"]["retryable"], false);
        assert_eq!(result["error"]["details"]["resource"]["kind"], "branch");
        assert_eq!(result["error"]["details"]["resource"]["branch"], BRANCH);
        assert!(result.get("canonical_state").is_none());
        assert!(
            result["error"]["action"]
                .as_str()
                .is_some_and(|action| action.contains("does not distinguish"))
        );
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_missing_merge_sha_inconclusive_in_mixed_batch() {
    const NUMBER: u64 = 28;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_delivery = "cli-batch-issue-28";
    let pull_delivery = "cli-batch-pull-missing-merge-sha-28";
    let issue_body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"}
    }))
    .expect("serialize signed issue event");
    let pull_body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize signed pull request event");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (delivery_id, event, body) in [
        (issue_delivery, "issues", issue_body.as_slice()),
        (pull_delivery, "pull_request", pull_body.as_slice()),
    ] {
        let (status, accepted) = send(address, &request_event(delivery_id, event, body));
        assert_eq!(status, 202, "event {event}");
        assert_eq!(accepted["accepted"], true, "event {event}");
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted issue and PR events");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                200,
                json!({
                    "number": NUMBER,
                    "state": "closed",
                    "updated_at": "2026-10-25T16:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{NUMBER}")
                }),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": NUMBER,
                    "state": "closed",
                    "merged": true,
                    "updated_at": "2026-10-25T16:01:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
                    "head": {"sha": "1111111111111111111111111111111111111111"},
                    "base": {"sha": "2222222222222222222222222222222222222222"}
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(50),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(issue_delivery)
        .arg(pull_delivery)
        .stdin(Stdio::null())
        .output()
        .expect("run mixed batch with a malformed merged PR snapshot");
    let requests = fixture.join().expect("issue and PR API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed batch returns structured JSON for both deliveries");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], issue_delivery);
    assert_eq!(results[0]["verdict"], "current_state_read");
    assert_eq!(results[0]["canonical_state"]["state"], "closed");
    assert_eq!(results[1]["delivery_id"], pull_delivery);
    assert_eq!(results[1]["verdict"], "inconclusive");
    assert_eq!(results[1]["error"]["code"], "invalid_current_state");
    assert_eq!(results[1]["error"]["details"]["field"], "merge_commit_sha");
    assert_eq!(
        results[1]["error"]["details"]["resource"]["kind"],
        "pull_request"
    );
    assert!(results[1].get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable batch journal"),
        journal_before,
        "reconciliation must not rewrite accepted events"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_routes_pull_request_review_comment_to_pull_request_api() {
    const NUMBER: u64 = 29;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-pr-review-comment-29";
    let body = serde_json::to_vec(&json!({
        "action": "reviewed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        },
        "review": {"type": "pull_request_review_comment", "content": "inline review comment"},
        "comment": {"id": 929, "body": "Please adjust this line."}
    }))
    .expect("serialize valid pull-request review-comment webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event(delivery_id, "pull_request_comment", &body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted review-comment journal");

    let current_head = "5555555555555555555555555555555555555555";
    let current_base = "6666666666666666666666666666666666666666";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "closed",
            "merged": true,
            "merge_commit_sha": "3333333333333333333333333333333333333333",
            "updated_at": "2026-10-25T18:30:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": current_head},
            "base": {"sha": current_base}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for the review-comment delivery");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("review-comment reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["merged"], true);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted review-comment journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_routes_issue_comment_pr_conversation_to_pull_request_api() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": 23, "state": "open"},
        "comment": {"id": 915, "body": "Please check the current PR state."},
        "is_pull": true,
        "pull_request": {
            "number": 23,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize Forgejo v17 pull request conversation payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (wrong_header_status, wrong_header_rejection) = send(
        address,
        &request_event(
            "cli-pr-conversation-wrong-header",
            "pull_request_comment",
            &body,
        ),
    );
    assert_eq!(wrong_header_status, 400);
    assert_eq!(
        wrong_header_rejection["error"]["code"],
        "event_payload_invalid"
    );
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .unwrap()
            .is_empty()
    );

    let (status, accepted) = send(
        address,
        &request_event("cli-pr-conversation-23", "issue_comment", &body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let fixture = start_cli_http_fixture_responses(vec![
        (
            200,
            json!({
                "number": 23,
                "state": "closed",
                "merged": true,
                "draft": false,
                "merge_commit_sha": "3333333333333333333333333333333333333333",
                "updated_at": "2026-10-25T12:00:00Z",
                "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23",
                "head": {"sha": current_head},
                "base": {"sha": current_base}
            }),
        ),
        (
            200,
            json!({
                "id": 915,
                "body": "canonical current PR conversation comment",
                "updated_at": "2026-10-25T12:01:00Z",
                "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23#issuecomment-915",
                "issue_url": "",
                "pull_request_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23"
            }),
        ),
        (
            200,
            json!({
                "number": 23,
                "state": "closed",
                "merged": true,
                "draft": false,
                "merge_commit_sha": "3333333333333333333333333333333333333333",
                "updated_at": "2026-10-25T12:02:00Z",
                "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23",
                "head": {"sha": current_head},
                "base": {"sha": current_base}
            }),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-pr-conversation-23")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for an issue_comment delivery");
    let requests = fixture.join().expect("pull request API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n"),
        "a PR conversation comment must reconcile through the pull request endpoint: {requests:?}"
    );
    assert!(requests[1].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/issues/comments/915 HTTP/1.1\r\n"
    ));
    assert!(
        requests[2]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n")
    );
    for request in &requests {
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "anonymous current-state read must not send Authorization"
        );
    }

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], 23);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["merged"], true);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_eq!(
        value["canonical_state"]["comment"]["body"],
        "canonical current PR conversation comment"
    );
    assert_eq!(
        value["canonical_state"]["comment"]["body_trust"],
        "untrusted_user_content"
    );
    assert_eq!(
        value["canonical_state"]["html_url"],
        "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23"
    );
    assert_ne!(
        value["canonical_state"]["head_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_ne!(
        value["canonical_state"]["base_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_foreign_pr_base_before_reading_comment() {
    const NUMBER: u64 = 23;
    const COMMENT_ID: u64 = 915;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-pr-comment-foreign-base-23";
    let body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "open"},
        "comment": {"id": COMMENT_ID, "body": "untrusted webhook text"},
        "is_pull": true,
        "pull_request": {"number": NUMBER, "state": "open"}
    }))
    .expect("serialize signed pull request conversation comment");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted comment journal");

    let pr_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-26T12:00:00Z",
            "html_url": pr_url,
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {
                "sha": "2222222222222222222222222222222222222222",
                "repo": {
                    "full_name": "other-owner/other-repository",
                    "name": "other-repository",
                    "owner": {"login": "other-owner"}
                }
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual CLI against a foreign PR base fixture");
    let requests = fixture.join().expect("foreign PR base fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("foreign PR base refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable comment journal"),
        journal_before
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_invalid_pr_head_before_reading_comment() {
    const NUMBER: u64 = 24;
    const COMMENT_ID: u64 = 916;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-pr-comment-invalid-head-24";
    let body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "open"},
        "comment": {"id": COMMENT_ID, "body": "untrusted webhook comment"},
        "is_pull": true,
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize signed pull request conversation comment");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted comment journal");

    let fixture = start_cli_http_fixture_responses(vec![(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "draft": false,
            "updated_at": "2026-10-27T12:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": "not-a-valid-commit-id"},
            "base": {
                "sha": "2222222222222222222222222222222222222222",
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }),
    )]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual CLI against a malformed current PR head fixture");
    let requests = fixture.join().expect("PR parent fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("malformed PR parent refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["details"]["field"], "head.sha");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert!(value.get("canonical_state").is_none());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("untrusted webhook comment"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable comment journal"),
        journal_before
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_missing_pr_draft_before_reading_comment() {
    const NUMBER: u64 = 24;
    const COMMENT_ID: u64 = 916;
    const DELIVERY_ID: &str = "cli-pr-comment-missing-draft-24";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "open"},
        "comment": {"id": COMMENT_ID, "body": "untrusted webhook comment"},
        "is_pull": true,
        "pull_request": {"number": NUMBER}
    }))
    .expect("serialize signed pull request conversation comment");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted comment journal");

    let fixture = start_cli_http_fixture_responses(vec![(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-27T12:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {
                "sha": "2222222222222222222222222222222222222222",
                "repo": {
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER}
                }
            }
        }),
    )]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual CLI against a PR parent missing draft");
    let requests = fixture.join().expect("PR parent fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("missing draft refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["details"]["field"], "draft");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert!(value.get("canonical_state").is_none());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("untrusted webhook comment"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable comment journal"),
        journal_before
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_missing_draft_on_fresh_deleted_comment_parent() {
    const NUMBER: u64 = 24;
    const COMMENT_ID: u64 = 916;
    const DELIVERY_ID: &str = "cli-pr-comment-fresh-parent-missing-draft-24";
    const COMMENT_404: &str = "deleted-comment-404-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"},
        "pull_request": {"number": NUMBER},
        "comment": {"id": COMMENT_ID, "body": "untrusted deleted PR comment"},
        "is_pull": true
    }))
    .expect("serialize signed Forgejo PR-comment deletion payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before =
        fs::read(&journal_path).expect("snapshot accepted comment-deletion journal");

    let valid_parent = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "merge_commit_sha": null,
        "updated_at": "2026-10-27T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {
            "sha": "2222222222222222222222222222222222222222",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let malformed_fresh_parent = json!({
        "number": NUMBER,
        "state": "closed",
        "merged": true,
        "merge_commit_sha": "5555555555555555555555555555555555555555",
        "updated_at": "2026-10-28T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "3333333333333333333333333333333333333333"},
        "base": {
            "sha": "4444444444444444444444444444444444444444",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let fixture = start_cli_http_fixture_responses(vec![
        (200, valid_parent),
        (404, json!({"message": COMMENT_404})),
        (200, malformed_fresh_parent),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual CLI against a malformed fresh PR parent");
    let requests = fixture
        .join()
        .expect("fresh-parent PR-comment fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
    )));
    assert!(requests[2].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        request
            .lines()
            .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid fresh PR parent refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["details"]["field"], "draft");
    assert_eq!(value["upstream_read_attempts"], 3);
    assert!(value.get("canonical_state").is_none());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(COMMENT_404));
    assert!(!transcript.contains("untrusted deleted PR comment"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable comment-deletion journal"),
        journal_before
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_foreign_base_on_fresh_deleted_comment_parent() {
    const NUMBER: u64 = 28;
    const COMMENT_ID: u64 = 929;
    const DELIVERY_ID: &str = "cli-pr-comment-foreign-fresh-base-28";
    const COMMENT_404: &str = "foreign-fresh-base-comment-404-body-must-not-leak";
    const FOREIGN_BASE_SENTINEL: &str = "foreign-pr-comment-base-private-sentinel";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"},
        "pull_request": {"number": NUMBER},
        "comment": {"id": COMMENT_ID, "body": "untrusted deleted PR comment"},
        "is_pull": true
    }))
    .expect("serialize signed Forgejo PR-comment deletion payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before =
        fs::read(&journal_path).expect("snapshot accepted comment-deletion journal");

    let initial_parent = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "merge_commit_sha": null,
        "updated_at": "2026-10-27T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {
            "sha": "2222222222222222222222222222222222222222",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let foreign_fresh_parent = json!({
        "number": NUMBER,
        "state": "closed",
        "merged": true,
        "draft": false,
        "merge_commit_sha": "5555555555555555555555555555555555555555",
        "updated_at": "2026-10-28T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "3333333333333333333333333333333333333333"},
        "base": {
            "sha": "4444444444444444444444444444444444444444",
            "repo": {
                "full_name": format!("{OWNER}/{FOREIGN_BASE_SENTINEL}"),
                "name": FOREIGN_BASE_SENTINEL,
                "owner": {"login": OWNER}
            }
        }
    });
    let fixture = start_cli_http_fixture_responses(vec![
        (200, initial_parent),
        (404, json!({"message": COMMENT_404})),
        (200, foreign_fresh_parent),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual CLI against a foreign base on the fresh PR parent");
    let requests = fixture
        .join()
        .expect("foreign fresh-parent base fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 3);
    for (index, request) in requests.iter().enumerate() {
        let expected_path = if index == 1 {
            format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
            )
        } else {
            format!("GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n")
        };
        assert!(
            request.starts_with(&expected_path),
            "request {index}: {request}"
        );
        assert!(
            request
                .lines()
                .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
        );
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("foreign fresh-parent base refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["upstream_read_attempts"], 3);
    assert!(value.get("canonical_state").is_none());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(COMMENT_404));
    assert!(!transcript.contains(FOREIGN_BASE_SENTINEL));
    assert!(!transcript.contains("untrusted deleted PR comment"));
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable comment-deletion journal"),
        journal_before
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_retries_fresh_pr_parent_before_projecting_deleted_comment_absent() {
    const NUMBER: u64 = 25;
    const COMMENT_ID: u64 = 917;
    const DELIVERY_ID: &str = "cli-pr-comment-fresh-parent-retry-503-25";
    const COMMENT_404: &str = "deleted-comment-404-body-must-not-leak";
    const PARENT_503: &str = "fresh-parent-503-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"},
        "pull_request": {"number": NUMBER},
        "comment": {"id": COMMENT_ID, "body": "untrusted deleted PR comment"},
        "is_pull": true
    }))
    .expect("serialize signed Forgejo PR-comment deletion payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before =
        fs::read(&journal_path).expect("snapshot accepted comment-deletion journal");

    let initial_parent = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "merge_commit_sha": null,
        "updated_at": "2026-10-27T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {
            "sha": "2222222222222222222222222222222222222222",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let fresh_parent = json!({
        "number": NUMBER,
        "state": "closed",
        "merged": true,
        "draft": false,
        "merge_commit_sha": "5555555555555555555555555555555555555555",
        "updated_at": "2026-10-28T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "3333333333333333333333333333333333333333"},
        "base": {
            "sha": "4444444444444444444444444444444444444444",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let fixture = start_scripted_cli_http_fixture(
        vec![
            (200, initial_parent, Vec::new()),
            (404, json!({"message": COMMENT_404}), Vec::new()),
            (
                503,
                json!({"message": PARENT_503}),
                vec![("Retry-After".to_owned(), "0".to_owned())],
            ),
            (200, fresh_parent, Vec::new()),
        ],
        Duration::from_millis(300),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the CLI through a transient fresh-parent failure");
    let requests = fixture
        .join()
        .expect("fresh-parent retry fixture completed");

    assert!(
        output.status.success(),
        "successful fresh-parent retry should allow reconciliation: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.len(),
        4,
        "one 503 must be retried before projection"
    );
    for (index, request) in requests.iter().enumerate() {
        let expected_path = if index == 1 {
            format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
            )
        } else {
            format!("GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n")
        };
        assert!(
            request.starts_with(&expected_path),
            "request {index}: {request}"
        );
        assert!(
            request
                .lines()
                .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable comment-deletion journal"),
        journal_before
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(COMMENT_404));
    assert!(!transcript.contains(PARENT_503));
    assert!(!transcript.contains("untrusted deleted PR comment"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("successful retry result is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 4);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["merged"], true);
    assert_eq!(
        value["canonical_state"]["merge_commit_sha"],
        "5555555555555555555555555555555555555555"
    );
    assert_eq!(value["canonical_state"]["comment"]["state"], "absent");
    assert!(value.get("error").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_deleted_pr_comment_inconclusive_after_fresh_parent_retries_exhaust()
 {
    const NUMBER: u64 = 26;
    const COMMENT_ID: u64 = 918;
    const DELIVERY_ID: &str = "cli-pr-comment-fresh-parent-exhausted-503-26";
    const COMMENT_404: &str = "deleted-comment-404-body-must-not-leak";
    const PARENT_503: &str = "fresh-parent-exhausted-503-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"},
        "pull_request": {"number": NUMBER},
        "comment": {"id": COMMENT_ID, "body": "untrusted deleted PR comment"},
        "is_pull": true
    }))
    .expect("serialize signed Forgejo PR-comment deletion payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before =
        fs::read(&journal_path).expect("snapshot accepted comment-deletion journal");

    let initial_parent = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "merge_commit_sha": null,
        "updated_at": "2026-10-27T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {
            "sha": "2222222222222222222222222222222222222222",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let retry_after_zero = vec![("Retry-After".to_owned(), "0".to_owned())];
    let fixture = start_scripted_cli_http_fixture(
        vec![
            (200, initial_parent, Vec::new()),
            (404, json!({"message": COMMENT_404}), Vec::new()),
            (
                503,
                json!({"message": PARENT_503}),
                retry_after_zero.clone(),
            ),
            (
                503,
                json!({"message": PARENT_503}),
                retry_after_zero.clone(),
            ),
            (503, json!({"message": PARENT_503}), retry_after_zero),
        ],
        Duration::from_millis(300),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the CLI until the fresh-parent retry budget is exhausted");
    let requests = fixture
        .join()
        .expect("exhausted fresh-parent retry fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        5,
        "the fresh parent gets exactly three attempts"
    );
    for (index, request) in requests.iter().enumerate() {
        let expected_path = if index == 1 {
            format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
            )
        } else {
            format!("GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n")
        };
        assert!(
            request.starts_with(&expected_path),
            "request {index}: {request}"
        );
        assert!(
            request
                .lines()
                .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
        );
    }
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable comment-deletion journal"),
        journal_before
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(COMMENT_404));
    assert!(!transcript.contains(PARENT_503));
    assert!(!transcript.contains("untrusted deleted PR comment"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("retry exhaustion result is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 5);
    assert_eq!(value["error"]["code"], "upstream_http_status");
    assert_eq!(value["error"]["retryable"], true);
    assert_eq!(value["error"]["details"]["http_status"], 503);
    assert!(value.get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_replays_same_deleted_pr_comment_after_upstream_retry_exhaustion() {
    const NUMBER: u64 = 27;
    const COMMENT_ID: u64 = 919;
    const DELIVERY_ID: &str = "cli-pr-comment-recover-after-503-27";
    const COMMENT_404: &str = "recovery-comment-404-body-must-not-leak";
    const PARENT_503: &str = "recovery-parent-503-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"},
        "pull_request": {"number": NUMBER},
        "comment": {"id": COMMENT_ID, "body": "untrusted deleted PR comment"},
        "is_pull": true
    }))
    .expect("serialize signed Forgejo PR-comment deletion payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before =
        fs::read(&journal_path).expect("snapshot accepted comment-deletion journal");

    let initial_parent = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "merge_commit_sha": null,
        "updated_at": "2026-10-27T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {
            "sha": "2222222222222222222222222222222222222222",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let retry_after_zero = vec![("Retry-After".to_owned(), "0".to_owned())];
    let outage_fixture = start_scripted_cli_http_fixture(
        vec![
            (200, initial_parent.clone(), Vec::new()),
            (404, json!({"message": COMMENT_404}), Vec::new()),
            (
                503,
                json!({"message": PARENT_503}),
                retry_after_zero.clone(),
            ),
            (
                503,
                json!({"message": PARENT_503}),
                retry_after_zero.clone(),
            ),
            (503, json!({"message": PARENT_503}), retry_after_zero),
        ],
        Duration::from_millis(300),
    );
    let failed_output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the CLI until the parent retry budget is exhausted");
    let outage_requests = outage_fixture
        .join()
        .expect("parent outage fixture completed");

    assert_eq!(failed_output.status.code(), Some(1));
    assert_eq!(outage_requests.len(), 5);
    for (index, request) in outage_requests.iter().enumerate() {
        let expected_path = if index == 1 {
            format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
            )
        } else {
            format!("GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n")
        };
        assert!(
            request.starts_with(&expected_path),
            "outage request {index}: {request}"
        );
        assert!(
            request
                .lines()
                .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
        );
    }
    let failed_value: Value = serde_json::from_slice(&failed_output.stdout)
        .expect("exhausted retry result is structured JSON");
    assert_reconcile_schema_version(&failed_value);
    assert_eq!(failed_value["verdict"], "inconclusive");
    assert_eq!(failed_value["delivery_id"], DELIVERY_ID);
    assert_eq!(failed_value["upstream_read_attempts"], 5);
    assert_eq!(failed_value["error"]["code"], "upstream_http_status");
    assert_eq!(failed_value["error"]["details"]["http_status"], 503);
    assert!(failed_value.get("canonical_state").is_none());
    let failed_transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&failed_output.stdout),
        String::from_utf8_lossy(&failed_output.stderr)
    );
    assert!(!failed_transcript.contains(COMMENT_404));
    assert!(!failed_transcript.contains(PARENT_503));
    assert!(!failed_transcript.contains("untrusted deleted PR comment"));
    assert_eq!(
        fs::read(&journal_path).expect("check journal after failed reconciliation"),
        journal_before
    );
    let first_replay = EventJournal::open(&journal_path)
        .replay()
        .expect("replay persisted webhook journal");
    assert_eq!(first_replay.len(), 1);
    assert_eq!(
        first_replay[0].idempotency_key,
        format!("forgejo-delivery-v1:{DELIVERY_ID}")
    );

    let recovered_parent = json!({
        "number": NUMBER,
        "state": "closed",
        "merged": true,
        "draft": false,
        "merge_commit_sha": "5555555555555555555555555555555555555555",
        "updated_at": "2026-10-28T12:00:00Z",
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
        "head": {"sha": "3333333333333333333333333333333333333333"},
        "base": {
            "sha": "4444444444444444444444444444444444444444",
            "repo": {
                "full_name": format!("{OWNER}/{REPOSITORY}"),
                "name": REPOSITORY,
                "owner": {"login": OWNER}
            }
        }
    });
    let recovery_fixture = start_scripted_cli_http_fixture(
        vec![
            (200, initial_parent, Vec::new()),
            (404, json!({"message": COMMENT_404}), Vec::new()),
            (200, recovered_parent, Vec::new()),
        ],
        Duration::from_millis(300),
    );
    let recovered_output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("replay the same durable delivery after the API recovers");
    let recovery_requests = recovery_fixture
        .join()
        .expect("recovered parent fixture completed");

    assert!(
        recovered_output.status.success(),
        "the same delivery should recover after the API is healthy: stdout={} stderr={}",
        String::from_utf8_lossy(&recovered_output.stdout),
        String::from_utf8_lossy(&recovered_output.stderr)
    );
    assert_eq!(recovery_requests.len(), 3);
    for (index, request) in recovery_requests.iter().enumerate() {
        let expected_path = if index == 1 {
            format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
            )
        } else {
            format!("GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n")
        };
        assert!(
            request.starts_with(&expected_path),
            "recovery request {index}: {request}"
        );
        assert!(
            request
                .lines()
                .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
        );
    }
    let recovered_value: Value = serde_json::from_slice(&recovered_output.stdout)
        .expect("recovered reconciliation result is structured JSON");
    assert_reconcile_schema_version(&recovered_value);
    assert_eq!(recovered_value["verdict"], "current_state_read");
    assert_eq!(recovered_value["delivery_id"], DELIVERY_ID);
    assert_eq!(recovered_value["upstream_read_attempts"], 3);
    assert_eq!(recovered_value["canonical_state"]["state"], "closed");
    assert_eq!(recovered_value["canonical_state"]["merged"], true);
    assert_eq!(
        recovered_value["canonical_state"]["merge_commit_sha"],
        "5555555555555555555555555555555555555555"
    );
    assert_eq!(
        recovered_value["canonical_state"]["comment"]["state"],
        "absent"
    );
    assert!(recovered_value.get("error").is_none());
    let recovered_transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&recovered_output.stdout),
        String::from_utf8_lossy(&recovered_output.stderr)
    );
    assert!(!recovered_transcript.contains(COMMENT_404));
    assert!(!recovered_transcript.contains("untrusted deleted PR comment"));
    assert_eq!(
        fs::read(&journal_path).expect("check journal after recovery"),
        journal_before,
        "reconciliation retry must not append another webhook delivery"
    );
    let final_replay = EventJournal::open(&journal_path)
        .replay()
        .expect("verify journal after recovery");
    assert_eq!(final_replay.len(), 1);
    assert_eq!(
        final_replay[0].idempotency_key,
        format!("forgejo-delivery-v1:{DELIVERY_ID}")
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reconciles_same_id_corrected_pr_comment_after_receiver_refusal() {
    const NUMBER: u64 = 31;
    const COMMENT_ID: u64 = 931;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "corrected-pr-comment-reconcile-31";
    let mut payload = json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER},
        "comment": {"id": COMMENT_ID, "body": "untrusted webhook text"},
        "is_pull": true
    });

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (rejected_status, rejected) = send(
        address,
        &request_event(
            delivery_id,
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize incomplete pull-comment payload"),
        ),
    );
    assert_eq!(
        rejected_status, 400,
        "missing PR parent must be rejected: {rejected}"
    );
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    assert!(
        EventJournal::open(&journal_path)
            .replay()
            .expect("replay journal after missing-parent refusal")
            .is_empty(),
        "the rejected delivery ID must remain available for corrected redelivery"
    );

    payload["pull_request"] = json!({
        "number": NUMBER,
        "state": "open",
        "merged": false,
        "draft": true,
        "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
        "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
    });
    let (accepted_status, accepted) = send(
        address,
        &request_event(
            delivery_id,
            "issue_comment",
            &serde_json::to_vec(&payload).expect("serialize corrected pull-comment payload"),
        ),
    );
    assert_eq!(accepted_status, 202, "corrected same-ID event: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());

    let events = EventJournal::open(&journal_path)
        .replay()
        .expect("replay corrected pull-comment event");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "forgejo.webhook.v1.issue_comment");
    assert_eq!(events[0].payload["delivery_id"], delivery_id);
    assert_eq!(events[0].payload["payload"]["is_pull"], true);
    assert_eq!(
        events[0].payload["payload"]["pull_request"]["number"],
        NUMBER
    );
    let journal_before = fs::read(&journal_path).expect("snapshot corrected-event journal");

    let current_head = "1111111111111111111111111111111111111111";
    let current_base = "2222222222222222222222222222222222222222";
    let pull_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}");
    let fixture = start_cli_http_fixture_responses(vec![
        (
            200,
            json!({
                "number": NUMBER,
                "state": "closed",
                "merged": true,
                "draft": false,
                "merge_commit_sha": "3333333333333333333333333333333333333333",
                "updated_at": "2026-10-28T10:00:00Z",
                "html_url": pull_url,
                "head": {"sha": current_head},
                "base": {"sha": current_base}
            }),
        ),
        (
            200,
            json!({
                "id": COMMENT_ID,
                "body": "canonical current PR conversation comment",
                "updated_at": "2026-10-28T10:01:00Z",
                "html_url": format!("{pull_url}#issuecomment-{COMMENT_ID}"),
                "issue_url": "",
                "pull_request_url": pull_url
            }),
        ),
        (
            200,
            json!({
                "number": NUMBER,
                "state": "closed",
                "merged": true,
                "draft": false,
                "merge_commit_sha": "3333333333333333333333333333333333333333",
                "updated_at": "2026-10-28T10:02:00Z",
                "html_url": pull_url,
                "head": {"sha": current_head},
                "base": {"sha": current_base}
            }),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run actual reconciliation CLI for the corrected PR comment");
    let requests = fixture
        .join()
        .expect("PR and comment API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation CLI failed with status {:?}; stdout={}; stderr={}; requests={requests:?}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
    )));
    assert!(requests[2].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "pull_request");
    assert_eq!(value["resource"]["number"], NUMBER);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["merged"], true);
    assert_eq!(value["canonical_state"]["draft"], false);
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(value["canonical_state"]["base_sha"], current_base);
    assert_ne!(
        value["canonical_state"]["draft"],
        payload["pull_request"]["draft"]
    );
    assert_ne!(
        value["canonical_state"]["head_sha"],
        payload["pull_request"]["head"]["sha"]
    );
    assert_ne!(
        value["canonical_state"]["base_sha"],
        payload["pull_request"]["base"]["sha"]
    );
    assert_eq!(
        value["canonical_state"]["comment"]["body"],
        "canonical current PR conversation comment"
    );
    assert_eq!(
        value["canonical_state"]["comment"]["body_trust"],
        "untrusted_user_content"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted corrected-event journal"),
        journal_before,
        "reconciliation must not rewrite the accepted webhook"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_routes_issue_comment_to_issue_api() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": 24, "state": "open"},
        "comment": {"id": 916, "body": "Please check the current issue state."},
        "is_pull": false
    }))
    .expect("serialize Forgejo v17 issue-comment payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event("cli-issue-comment-24", "issue_comment", &body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_events = EventJournal::open(&journal_path).replay().unwrap();
    assert_eq!(journal_events.len(), 1);
    assert_eq!(journal_events[0].payload["payload"]["is_pull"], false);

    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
        (
            200,
            json!({
                "number": 24,
                "state": "closed",
                "updated_at": "2026-10-25T12:30:00Z",
                "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/24"
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": 916,
                "body": "canonical comment body",
                "updated_at": "2026-10-25T12:31:00Z",
                "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/24#issuecomment-916",
                "issue_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/24"
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "number": 24,
                "state": "closed",
                "updated_at": "2026-10-25T12:32:00Z",
                "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/24"
            }),
            Vec::new(),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-issue-comment-24")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for an issue_comment delivery");
    let _ = finish_fixture.send(());
    let requests = fixture.join().expect("issue API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/24 HTTP/1.1\r\n"),
        "the parent issue must be readable before comment reconciliation: {requests:?}"
    );
    assert!(requests[1].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/issues/comments/916 HTTP/1.1\r\n"
    ));
    assert!(
        requests[2]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/24 HTTP/1.1\r\n")
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("/pulls/24"))
    );
    assert!(
        requests.iter().all(|request| !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["resource"]["kind"], "issue_comment");
    assert_eq!(value["resource"]["issue_number"], 24);
    assert_eq!(value["resource"]["comment_id"], 916);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(
        value["canonical_state"]["updated_at"],
        "2026-10-25T12:32:00Z"
    );
    assert_eq!(
        value["canonical_state"]["html_url"],
        "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/24"
    );
    assert_eq!(value["canonical_state"]["comment"]["state"], "present");
    assert_eq!(
        value["canonical_state"]["comment"]["body"],
        "canonical comment body"
    );
    assert_eq!(
        value["canonical_state"]["comment"]["body_trust"],
        "untrusted_user_content"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_uses_present_comment_readback_after_deleted_webhook() {
    const DELIVERY_ID: &str = "cli-comment-delete-still-present-916";
    const ISSUE_NUMBER: u64 = 24;
    const COMMENT_ID: u64 = 916;
    const UNTRUSTED_BODY: &str = "stale deletion webhook body must not become canonical";
    const CURRENT_BODY: &str = "current API body after deletion notification";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "comment": {"id": COMMENT_ID, "body": UNTRUSTED_BODY},
        "is_pull": false
    }))
    .expect("serialize signed Forgejo issue-comment deletion webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(
        status, 202,
        "signed deletion webhook is accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted deletion journal");

    let (finish_fixture, fixture) = start_scripted_cli_http_fixture_until_finished(vec![
        (
            200,
            json!({
                "number": ISSUE_NUMBER,
                "state": "closed",
                "updated_at": "2026-10-26T09:00:00Z",
                "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "id": COMMENT_ID,
                "body": CURRENT_BODY,
                "updated_at": "2026-10-26T09:01:00Z",
                "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}#issuecomment-{COMMENT_ID}"),
                "issue_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
            }),
            Vec::new(),
        ),
        (
            200,
            json!({
                "number": ISSUE_NUMBER,
                "state": "closed",
                "updated_at": "2026-10-26T09:02:00Z",
                "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
            }),
            Vec::new(),
        ),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for the deletion delivery");
    let _ = finish_fixture.send(());
    let requests = fixture
        .join()
        .expect("present-comment readback fixture completed");

    assert!(
        output.status.success(),
        "current API state should be reconciled: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
    )));
    assert!(requests[2].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted deletion journal"),
        journal_before,
        "current-state reconciliation must not rewrite the signed delivery"
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(UNTRUSTED_BODY));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("current state is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["upstream_read_attempts"], 3);
    assert_eq!(value["resource"]["kind"], "issue_comment");
    assert_eq!(value["resource"]["issue_number"], ISSUE_NUMBER);
    assert_eq!(value["resource"]["comment_id"], COMMENT_ID);
    assert_eq!(value["canonical_state"]["state"], "closed");
    assert_eq!(value["canonical_state"]["comment"]["state"], "present");
    assert_eq!(value["canonical_state"]["comment"]["body"], CURRENT_BODY);
    assert_eq!(
        value["canonical_state"]["comment"]["body_trust"],
        "untrusted_user_content"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_deleted_comment_inconclusive_when_parent_issue_is_unreadable()
{
    const DELIVERY_ID: &str = "cli-comment-deleted-parent-404-24";
    const ISSUE_NUMBER: u64 = 24;
    const COMMENT_ID: u64 = 916;
    const NOT_FOUND_BODY: &str = "private-parent-issue-404-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"},
        "comment": {"id": COMMENT_ID, "body": "untrusted deleted comment"},
        "is_pull": false
    }))
    .expect("serialize signed Forgejo v17 issue-comment deletion payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(
        status, 202,
        "signed deletion delivery should be journaled: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before =
        fs::read(&journal_path).expect("snapshot accepted comment-deletion journal");

    // A missing parent is ambiguous (deleted or private); do not query the
    // comment route and do not interpret its own 404 as canonical absence.
    let fixture = start_cli_http_fixture(404, json!({"message":NOT_FOUND_BODY}), 1);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for an inaccessible comment parent");
    let requests = fixture
        .join()
        .expect("parent-issue API 404 fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        1,
        "comment API must not be read after parent 404"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/24 HTTP/1.1\r\n")
    );
    assert!(!requests[0].contains("/issues/comments/916"));
    assert!(
        requests[0]
            .lines()
            .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read comment-deletion journal"),
        journal_before,
        "an inconclusive parent read must not mutate the durable inbox"
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(NOT_FOUND_BODY));
    assert!(!transcript.contains("untrusted deleted comment"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("parent read refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(
        value["error"]["details"]["resource"]["kind"],
        "issue_comment"
    );
    assert_eq!(
        value["error"]["details"]["resource"]["issue_number"],
        ISSUE_NUMBER
    );
    assert_eq!(
        value["error"]["details"]["resource"]["comment_id"],
        COMMENT_ID
    );
    assert!(value.get("canonical_state").is_none());
    let action = value["error"]["action"]
        .as_str()
        .expect("parent 404 has an actionable ambiguity explanation");
    assert!(action.contains("parent issue"));
    assert!(action.contains("only after the parent issue is readable"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_deleted_pr_comment_inconclusive_when_parent_access_is_revoked_between_reads()
 {
    const DELIVERY_ID: &str = "cli-pr-comment-parent-revoked-between-reads-24";
    const PR_NUMBER: u64 = 24;
    const COMMENT_ID: u64 = 916;
    const COMMENT_404: &str = "private-comment-404-body-must-not-leak";
    const PARENT_404: &str = "revoked-parent-404-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "deleted",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": PR_NUMBER, "state": "closed"},
        "pull_request": {"number": PR_NUMBER},
        "comment": {"id": COMMENT_ID, "body": "untrusted deletion webhook body"},
        "is_pull": true
    }))
    .expect("serialize signed Forgejo PR-comment deletion payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "issue_comment", &body));
    assert_eq!(
        status, 202,
        "signed PR-comment delivery should be journaled: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted PR-comment journal");

    let parent = json!({
        "number": PR_NUMBER,
        "state": "closed",
        "merged": false,
        "merge_commit_sha": null,
        "updated_at": "2026-10-25T12:30:00Z",
        "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/24",
        "head": {"sha": "1111111111111111111111111111111111111111"},
        "base": {"sha": "2222222222222222222222222222222222222222"}
    });
    let fixture = start_scripted_cli_http_fixture(
        vec![
            (200, parent, Vec::new()),
            (404, json!({"message": COMMENT_404}), Vec::new()),
            (404, json!({"message": PARENT_404}), Vec::new()),
        ],
        Duration::from_millis(300),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI across a parent-visibility race");
    let requests = fixture
        .join()
        .expect("parent-visibility race fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        3,
        "the parent must be re-read after comment 404"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/24 HTTP/1.1\r\n")
    );
    assert!(requests[1].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/issues/comments/916 HTTP/1.1\r\n"
    ));
    assert!(
        requests[2]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/24 HTTP/1.1\r\n")
    );
    assert!(requests.iter().all(|request| {
        request
            .lines()
            .all(|line| !line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read PR-comment journal after refusal"),
        journal_before,
        "the permission-race refusal must not mutate the durable webhook journal"
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(COMMENT_404));
    assert!(!transcript.contains(PARENT_404));
    assert!(!transcript.contains("untrusted deletion webhook body"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("permission-race refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["upstream_read_attempts"], 3);
    assert_eq!(
        value["error"]["details"]["resource"]["kind"],
        "pull_request_comment"
    );
    assert_eq!(
        value["error"]["details"]["resource"]["pull_request_number"],
        PR_NUMBER
    );
    assert_eq!(
        value["error"]["details"]["resource"]["comment_id"],
        COMMENT_ID
    );
    assert!(value.get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_current_branch_after_stale_push_webhook() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-push-feature-agent-proof";
    let webhook_after = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let body = serde_json::to_vec(&json!({
        "ref": "refs/heads/feature/release/agent-proof",
        "before": "1111111111111111111111111111111111111111",
        "after": webhook_after,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "commits": [],
        "total_commits": 1
    }))
    .expect("serialize Forgejo push notification for a nested branch");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "push", &body));
    assert_eq!(status, 202, "push delivery should be journaled: {accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted push journal");

    let current_head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": "feature/release/agent-proof",
            "commit": {
                "id": current_head,
                "timestamp": "2026-10-25T15:30:00Z"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for a push delivery");
    let requests = fixture.join().expect("branch API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("push reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "branch");
    assert_eq!(value["resource"]["branch"], "feature/release/agent-proof");
    assert_eq!(value["canonical_state"]["state"], "present");
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(
        value["canonical_state"]["updated_at"],
        "2026-10-25T15:30:00Z"
    );
    assert_ne!(value["canonical_state"]["head_sha"], webhook_after);
    assert_eq!(
        value["canonical_state"]["html_url"],
        "http://127.0.0.1:3130/forge-admin/agent-forge-core/src/branch/feature%2Frelease%2Fagent-proof"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted push journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_current_branch_after_native_delete_notification() {
    const DELIVERY_ID: &str = "cli-native-branch-delete-recreated";
    const BRANCH: &str = "release/agent-proof";
    const CURRENT_HEAD: &str = "dddddddddddddddddddddddddddddddddddddddd";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "ref_type": "branch",
        "pusher_type": "user",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize Forgejo branch-delete notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "delete", &body));
    assert_eq!(
        status, 202,
        "branch-delete delivery should be journaled: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted delete journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": BRANCH,
            "commit": {
                "id": CURRENT_HEAD,
                "timestamp": "2026-10-25T18:30:00Z"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for a branch-delete delivery");
    let requests = fixture.join().expect("branch API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("branch-delete reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["resource"]["kind"], "branch");
    assert_eq!(value["resource"]["branch"], BRANCH);
    assert_eq!(value["canonical_state"]["state"], "present");
    assert_eq!(value["canonical_state"]["head_sha"], CURRENT_HEAD);
    assert_eq!(
        value["canonical_state"]["updated_at"],
        "2026-10-25T18:30:00Z"
    );
    assert_eq!(
        value["canonical_state"]["html_url"],
        "http://127.0.0.1:3130/forge-admin/agent-forge-core/src/branch/release%2Fagent-proof"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted delete journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_confirms_native_delete_404_after_repository_and_fresh_reads() {
    const DELIVERY_ID: &str = "cli-native-branch-delete-404";
    const BRANCH: &str = "release/agent-proof";
    const PRIVATE_MARKER: &str = "private-branch-and-repository-error-marker";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "ref_type": "branch",
        "pusher_type": "user",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize Forgejo branch-delete notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "delete", &body));
    assert_eq!(
        status, 202,
        "branch-delete delivery should be journaled: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted delete journal");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (404, json!({"message":PRIVATE_MARKER}), vec![]),
            (
                200,
                json!({
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER},
                    "description": PRIVATE_MARKER
                }),
                vec![],
            ),
            (404, json!({"message":PRIVATE_MARKER}), vec![]),
        ],
        Duration::from_millis(50),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for a branch-delete 404");
    let requests = fixture.join().expect("branch API 404 fixture completed");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(requests[1].starts_with("GET /api/v1/repos/forge-admin/agent-forge-core HTTP/1.1\r\n"));
    assert!(requests[2].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
    ));
    for request in &requests {
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "the local reconciliation CLI must not send an authorization header"
        );
    }
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(PRIVATE_MARKER));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("confirmed branch deletion is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 3);
    assert_eq!(value["resource"]["kind"], "branch");
    assert_eq!(value["resource"]["branch"], BRANCH);
    assert_eq!(value["canonical_state"]["state"], "absent");
    assert_eq!(value["canonical_state"]["head_sha"], Value::Null);
    assert_eq!(
        value["canonical_state"]["html_url"],
        "http://127.0.0.1:3130/forge-admin/agent-forge-core/src/branch/release%2Fagent-proof"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted delete journal"),
        journal_before,
        "reconciliation must not rewrite the durable inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_branch_absence_when_repository_identity_mismatches() {
    const DELIVERY_ID: &str = "cli-native-branch-delete-repository-mismatch";
    const BRANCH: &str = "release/agent-proof";
    const PRIVATE_MARKER: &str = "private-repository-identity-marker";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "ref_type": "branch",
        "pusher_type": "user",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize Forgejo branch-delete notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "delete", &body));
    assert_eq!(
        status, 202,
        "branch-delete delivery should be journaled: {accepted}"
    );
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted delete journal");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (404, json!({"message":PRIVATE_MARKER}), vec![]),
            (
                200,
                json!({
                    "full_name": format!("{OWNER}/other-repository"),
                    "name": "other-repository",
                    "owner": {"login": OWNER},
                    "description": PRIVATE_MARKER
                }),
                vec![],
            ),
        ],
        Duration::from_millis(50),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI against mismatched repository identity");
    let requests = fixture
        .join()
        .expect("repository mismatch fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        2,
        "do not read the branch after identity mismatch"
    );
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(requests[1].starts_with("GET /api/v1/repos/forge-admin/agent-forge-core HTTP/1.1\r\n"));
    for request in &requests {
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "the local reconciliation CLI must not send an authorization header"
        );
    }
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(PRIVATE_MARKER));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("identity mismatch refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "upstream_invalid_response");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["upstream_read_attempts"], 2);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted delete journal"),
        journal_before,
        "a failed repository identity check must not rewrite the durable inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_branch_absence_when_repository_visibility_is_denied_or_hidden()
 {
    const DELIVERY_ID: &str = "cli-native-branch-delete-repository-not-readable";
    const BRANCH: &str = "release/agent-proof";
    const PRIVATE_MARKER: &str = "private-repository-visibility-marker";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "ref_type": "branch",
        "pusher_type": "user",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize Forgejo branch-delete notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "delete", &body));
    assert_eq!(
        status, 202,
        "branch-delete delivery should be journaled: {accepted}"
    );
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted delete journal");

    for (repository_status, expected_error) in [
        (401, "upstream_unauthorized"),
        (403, "upstream_forbidden"),
        (404, "resource_not_found"),
    ] {
        let fixture = start_scripted_cli_http_fixture(
            vec![
                (404, json!({"message":PRIVATE_MARKER}), vec![]),
                (repository_status, json!({"message":PRIVATE_MARKER}), vec![]),
            ],
            Duration::from_millis(50),
        );
        let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
            .env_clear()
            .arg(&journal_path)
            .arg(DELIVERY_ID)
            .stdin(Stdio::null())
            .output()
            .expect("run reconciliation CLI with unreadable repository metadata");
        let requests = fixture
            .join()
            .expect("repository visibility fixture completed");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(requests.len(), 2, "do not issue a third branch read");
        assert!(requests[0].starts_with(
            "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
        ));
        assert!(
            requests[1].starts_with("GET /api/v1/repos/forge-admin/agent-forge-core HTTP/1.1\r\n")
        );
        assert!(requests.iter().all(|request| {
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        }));
        let transcript = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!transcript.contains(PRIVATE_MARKER));
        let value: Value = serde_json::from_slice(&output.stdout)
            .expect("repository visibility refusal is structured JSON");
        assert_reconcile_schema_version(&value);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["error"]["code"], expected_error);
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(value["upstream_read_attempts"], 2);
        assert!(value.get("canonical_state").is_none());
        assert_eq!(
            fs::read(&journal_path).expect("re-read accepted delete journal"),
            journal_before,
            "repository visibility failure must not rewrite the durable inbox"
        );
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_create_inconclusive_and_delete_absent_in_batch() {
    const CREATE_DELIVERY_ID: &str = "cli-branch-create-then-absent-001";
    const DELETE_DELIVERY_ID: &str = "cli-branch-delete-then-absent-002";
    const BRANCH: &str = "release/agent-proof";
    const PRIVATE_MARKER: &str = "private-branch-batch-marker";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let create_body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "ref_type": "branch",
        "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize Forgejo branch-create notification");
    let delete_body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "ref_type": "branch",
        "pusher_type": "user",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize Forgejo branch-delete notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (create_status, create_acceptance) = send(
        address,
        &request_event(CREATE_DELIVERY_ID, "create", &create_body),
    );
    assert_eq!(
        create_status, 202,
        "branch create should be journaled: {create_acceptance}"
    );
    let (delete_status, delete_acceptance) = send(
        address,
        &request_event(DELETE_DELIVERY_ID, "delete", &delete_body),
    );
    assert_eq!(
        delete_status, 202,
        "branch delete should be journaled: {delete_acceptance}"
    );
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted branch batch");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (404, json!({"message":PRIVATE_MARKER}), vec![]),
            (
                200,
                json!({
                    "full_name": format!("{OWNER}/{REPOSITORY}"),
                    "name": REPOSITORY,
                    "owner": {"login": OWNER},
                    "description": PRIVATE_MARKER
                }),
                vec![],
            ),
            (404, json!({"message":PRIVATE_MARKER}), vec![]),
        ],
        Duration::from_millis(50),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args([CREATE_DELIVERY_ID, DELETE_DELIVERY_ID])
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation CLI on the absent create/delete batch");
    let requests = fixture.join().expect("branch batch API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(requests[1].starts_with("GET /api/v1/repos/forge-admin/agent-forge-core HTTP/1.1\r\n"));
    assert!(requests[2].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(PRIVATE_MARKER));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("mixed branch batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 3);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], CREATE_DELIVERY_ID);
    assert_eq!(results[0]["verdict"], "inconclusive");
    assert_eq!(results[0]["error"]["code"], "resource_not_found");
    assert!(results[0].get("canonical_state").is_none());
    assert_eq!(results[1]["delivery_id"], DELETE_DELIVERY_ID);
    assert_eq!(results[1]["verdict"], "current_state_read");
    assert_eq!(results[1]["resource"]["kind"], "branch");
    assert_eq!(results[1]["resource"]["branch"], BRANCH);
    assert_eq!(results[1]["canonical_state"]["state"], "absent");
    assert_eq!(results[1]["canonical_state"]["head_sha"], Value::Null);
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted branch batch"),
        journal_before,
        "reconciliation must not mutate the append-only inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_new_branch_with_literal_escape_from_zero_before_push() {
    const DELIVERY_ID: &str = "cli-new-branch-literal-escape";
    const BRANCH: &str = "feature/release/%2Fagent-proof";
    const CURRENT_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": format!("refs/heads/{BRANCH}"),
        "before": "0000000000000000000000000000000000000000",
        "after": CURRENT_HEAD,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "commits": [],
        "total_commits": 1
    }))
    .expect("serialize Forgejo branch-creation push webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "push", &body));
    assert_eq!(
        status, 202,
        "new branch push should be journaled: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before =
        fs::read(&journal_path).expect("snapshot accepted branch-creation journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": BRANCH,
            "commit": {
                "id": CURRENT_HEAD,
                "timestamp": "2026-10-25T16:00:00Z"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for a branch-creation delivery");
    let requests = fixture.join().expect("branch API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2F%252Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("branch-creation reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["resource"]["kind"], "branch");
    assert_eq!(value["resource"]["branch"], BRANCH);
    assert_eq!(value["canonical_state"]["state"], "present");
    assert_eq!(value["canonical_state"]["head_sha"], CURRENT_HEAD);
    assert_eq!(
        value["canonical_state"]["html_url"],
        "http://127.0.0.1:3130/forge-admin/agent-forge-core/src/branch/feature%2Frelease%2F%252Fagent-proof"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted branch-creation journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_zero_current_branch_commit() {
    const DELIVERY_ID: &str = "cli-zero-current-branch-commit";
    const BRANCH: &str = "feature/release/zero-current-head";
    const ZERO_OID: &str = "0000000000000000000000000000000000000000";
    const PUSH_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": format!("refs/heads/{BRANCH}"),
        "before": ZERO_OID,
        "after": PUSH_HEAD,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "commits": [],
        "total_commits": 1
    }))
    .expect("serialize valid branch-creation push with the Git zero sentinel");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "push", &body));
    assert_eq!(
        status, 202,
        "valid push delivery should be accepted: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted push journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": BRANCH,
            "commit": {
                "id": ZERO_OID,
                "timestamp": "2026-10-26T12:00:00Z"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual Forgejo reconciliation CLI");
    let requests = fixture.join().expect("branch API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/branches/feature%2Frelease%2Fzero-current-head HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid current branch snapshot is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "commit.id");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted push journal"),
        journal_before,
        "a malformed current API snapshot must not rewrite the accepted event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_sha256_current_branch_commit() {
    const DELIVERY_ID: &str = "cli-sha256-current-branch-commit";
    const BRANCH: &str = "feature/release/sha256-current-head";
    const PUSH_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
    let current_head = "a".repeat(64);

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": format!("refs/heads/{BRANCH}"),
        "before": "0000000000000000000000000000000000000000",
        "after": PUSH_HEAD,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "commits": [],
        "total_commits": 1
    }))
    .expect("serialize a valid branch-creation push");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "push", &body));
    assert_eq!(
        status, 202,
        "valid branch push should be accepted: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted branch journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": BRANCH,
            "commit": {
                "id": current_head,
                "timestamp": "2026-10-26T12:00:00Z"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for the branch delivery");
    let requests = fixture.join().expect("branch API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/branches/feature%2Frelease%2Fsha256-current-head HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous branch reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("branch reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["canonical_state"]["state"], "present");
    assert_eq!(value["canonical_state"]["head_sha"], current_head);
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted branch journal"),
        journal_before,
        "reconciliation must leave the durable delivery unchanged"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reads_current_branch_after_native_create_notification() {
    const DELIVERY_ID: &str = "cli-native-branch-create-current-head";
    const BRANCH: &str = "release/agent-proof";
    const CREATED_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const CURRENT_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": BRANCH,
        "ref_type": "branch",
        "sha": CREATED_SHA,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "sender": {"login": "fixture-owner"}
    }))
    .expect("serialize Forgejo branch-create notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "create", &body));
    assert_eq!(
        status, 202,
        "branch-create delivery should be journaled: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted create journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": BRANCH,
            "commit": {
                "id": CURRENT_HEAD,
                "timestamp": "2026-10-25T19:30:00Z"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for a branch-create event");
    let requests = fixture.join().expect("branch API fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/release%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("branch-create reconciliation returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], DELIVERY_ID);
    assert_eq!(value["resource"]["kind"], "branch");
    assert_eq!(value["resource"]["branch"], BRANCH);
    assert_eq!(value["canonical_state"]["state"], "present");
    assert_eq!(value["canonical_state"]["head_sha"], CURRENT_HEAD);
    assert_ne!(value["canonical_state"]["head_sha"], CREATED_SHA);
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted create journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_mismatched_branch_snapshot_without_journal_mutation() {
    const DELIVERY_ID: &str = "cli-branch-snapshot-mismatch";
    const REQUESTED_BRANCH: &str = "feature/release/agent-proof";
    const RETURNED_BRANCH: &str = "feature/release/other-proof";
    const RESPONSE_MARKER: &str = "synthetic-mismatched-branch-body-must-not-leak";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "ref": format!("refs/heads/{REQUESTED_BRANCH}"),
        "before": "1111111111111111111111111111111111111111",
        "after": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "commits": [],
        "total_commits": 0
    }))
    .expect("serialize signed branch push webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "push", &body));
    assert_eq!(
        status, 202,
        "valid branch push should be journaled: {accepted}"
    );
    assert_eq!(accepted["reconcile_required"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted push journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": RETURNED_BRANCH,
            "commit": {
                "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "timestamp": "2026-10-25T15:30:00Z"
            },
            "message": RESPONSE_MARKER
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for the accepted push");
    let requests = fixture.join().expect("branch API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2Fagent-proof HTTP/1.1\r\n"
    ));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the local reconciliation CLI must not send an authorization header"
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(RESPONSE_MARKER));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("mismatch refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["resource"]["kind"], "branch");
    assert_eq!(
        value["error"]["details"]["resource"]["branch"],
        REQUESTED_BRANCH
    );
    assert!(value.get("canonical_state").is_none());
    assert!(
        value["error"]["action"]
            .as_str()
            .is_some_and(|action| action.contains("API resource identity")),
        "mismatched current state must have an actionable refusal: {value}"
    );
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted push journal"),
        journal_before,
        "a mismatched upstream snapshot must not mutate the accepted event"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_merged_pull_request_without_merge_commit_sha() {
    const NUMBER: u64 = 44;
    const DELIVERY_ID: &str = "cli-pr-missing-merge-commit-44";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize stale synthetic pull request webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "closed",
            "merged": true,
            "updated_at": "2026-10-25T19:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {"sha": "2222222222222222222222222222222222222222"}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert!(
        !output.status.success(),
        "merge state without a result commit is inconclusive"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("missing merge commit refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["details"]["field"], "merge_commit_sha");
    assert!(value.get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_merged_pull_request_with_zero_merge_commit_sha() {
    const NUMBER: u64 = 46;
    const DELIVERY_ID: &str = "cli-pr-zero-merge-commit-46";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize stale synthetic pull request webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("capture accepted webhook journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "closed",
            "merged": true,
            "merge_commit_sha": "0000000000000000000000000000000000000000",
            "updated_at": "2026-10-25T19:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {"sha": "2222222222222222222222222222222222222222"}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert!(
        !output.status.success(),
        "an all-zero object ID cannot identify a merged commit"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("zero merge commit refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["details"]["field"], "merge_commit_sha");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("reread accepted webhook journal"),
        journal_before,
        "a malformed upstream snapshot must not change the inbox"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_open_pull_request_reported_as_merged() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": 24,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize stale synthetic pull request webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(
        address,
        &request_event("cli-pr-open-but-merged-24", "pull_request", &body),
    );
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": 24,
            "state": "open",
            "merged": true,
            "merge_commit_sha": "3333333333333333333333333333333333333333",
            "updated_at": "2026-10-25T12:00:00Z",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/24",
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {"sha": "2222222222222222222222222222222222222222"}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-pr-open-but-merged-24")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert!(!output.status.success());
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/24 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("refusal is returned as structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_unmerged_pull_request_with_merge_commit_sha() {
    const NUMBER: u64 = 45;
    const DELIVERY_ID: &str = "cli-pr-unmerged-with-merge-45";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize synthetic unmerged pull request webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("read durable accepted inbox");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "merge_commit_sha": "3333333333333333333333333333333333333333",
            "updated_at": "2026-10-25T12:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {"sha": "2222222222222222222222222222222222222222"}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("inconsistent pull request snapshot returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "merge_commit_sha");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("reread inbox after inconclusive API snapshot"),
        journal_before,
        "an invalid current-state response must not mutate the durable webhook journal"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_success_http_with_invalid_timestamp() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-invalid-api-timestamp-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-25T11:00:00Z"
        }
    }))
    .expect("serialize valid stale webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-02-30T15:30:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("successful HTTP fixture completed");

    assert!(
        !output.status.success(),
        "HTTP 200 alone is not canonical success"
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid snapshot refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["field"], "updated_at");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_rejects_foreign_canonical_html_url() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-foreign-canonical-url-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-25T11:00:00Z"
        }
    }))
    .expect("serialize valid stale webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let foreign_url =
        format!("https://attacker.invalid/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-10-25T14:00:00Z",
            "html_url": foreign_url
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("successful HTTP fixture completed");

    assert!(
        !output.status.success(),
        "foreign canonical URL is not success"
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("scope mismatch is returned as structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["resource"]["kind"], "issue");
    assert_eq!(
        value["error"]["details"]["resource"]["number"],
        ISSUE_NUMBER
    );
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_rejects_same_origin_foreign_repository_html_url() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-same-origin-foreign-repository-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-25T11:00:00Z"
        }
    }))
    .expect("serialize valid stale webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let foreign_repository_url =
        format!("http://127.0.0.1:3130/other-owner/other-repository/issues/{ISSUE_NUMBER}");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-10-25T14:00:00Z",
            "html_url": foreign_repository_url
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("successful HTTP fixture completed");

    assert!(
        !output.status.success(),
        "same-origin response for a different repository is not canonical success"
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("same-origin repository mismatch is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["resource"]["kind"], "issue");
    assert_eq!(
        value["error"]["details"]["resource"]["number"],
        ISSUE_NUMBER
    );
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted webhook journal"),
        journal_before,
        "reconciliation must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_issue_repository_metadata_mismatch() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-foreign-issue-repository-meta-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize signed issue notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted issue delivery");

    // Keep the canonical URL on the configured issue route while contradicting
    // the independent RepositoryMeta returned by Forgejo's issue endpoint.
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-10-25T14:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}"),
            "repository": {
                "id": 99,
                "name": "other-repository",
                "owner": "other-owner",
                "full_name": "other-owner/other-repository"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("issue metadata fixture completed");

    assert!(!output.status.success());
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("repository identity refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "resource_mismatch");
    assert_eq!(value["error"]["retryable"], false);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted issue delivery"),
        journal_before,
        "reconciliation must not rewrite the durable webhook"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_refuses_partial_issue_repository_metadata() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-partial-issue-repository-meta-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize signed issue notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted issue delivery");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-10-25T14:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}"),
            "repository": {"name": REPOSITORY}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("partial issue metadata fixture completed");

    assert!(!output.status.success());
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("partial repository identity refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["details"]["field"], "repository");
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted issue delivery"),
        journal_before,
        "reconciliation must leave the durable delivery unchanged"
    );
}

#[test]
#[ignore = "run via scripts/run_webhook_reconcile_cli_e2e.py; fixture reserves the fixed loopback API endpoint"]
fn actual_reconcile_cli_process_reads_v17_issue_repository_metadata_owner_string() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-v17-issue-repository-meta-owner-string-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize signed issue notification");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(
        status, 202,
        "issue webhook was not durably accepted: {accepted}"
    );
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted issue delivery");

    let current_url = format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}");
    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-26T14:00:00Z",
            "html_url": current_url,
            "repository": {
                "id": 7,
                "name": REPOSITORY,
                "owner": OWNER,
                "full_name": format!("{OWNER}/{REPOSITORY}")
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("issue metadata fixture completed");

    assert!(
        output.status.success(),
        "reconciliation CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "anonymous reconciliation must not send an authorization header"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("reconciliation CLI returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["resource"]["kind"], "issue");
    assert_eq!(value["resource"]["owner"], OWNER);
    assert_eq!(value["resource"]["repository"], REPOSITORY);
    assert_eq!(value["resource"]["number"], ISSUE_NUMBER);
    assert_eq!(value["canonical_state"]["state"], "open");
    assert_eq!(value["canonical_state"]["html_url"], current_url);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted issue delivery"),
        journal_before,
        "readback is non-mutating"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reports_invalid_pull_request_base_sha_field() {
    const NUMBER: u64 = 25;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-invalid-pr-base-sha-25";
    let body = serde_json::to_vec(&json!({
        "action": "synchronized",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize valid pull request webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(delivery_id, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted pull request journal");

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": NUMBER,
            "state": "open",
            "merged": false,
            "updated_at": "2026-10-25T13:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{NUMBER}"),
            "head": {"sha": "1111111111111111111111111111111111111111"},
            "base": {"sha": "not-a-commit-sha"}
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI for the pull request delivery");
    let requests = fixture.join().expect("pull request API fixture completed");

    assert!(
        !output.status.success(),
        "a malformed base SHA cannot be canonical"
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("invalid pull request snapshot refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "invalid_current_state");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["action"],
        "inspect the named field in the Forgejo API snapshot and retry only after the source is corrected"
    );
    assert_eq!(value["error"]["details"]["field"], "base.sha");
    assert_eq!(
        value["error"]["details"]["resource"]["kind"],
        "pull_request"
    );
    assert_eq!(value["error"]["details"]["resource"]["number"], NUMBER);
    assert!(value.get("canonical_state").is_none());
    assert_eq!(
        fs::read(&journal_path).expect("re-read accepted pull request journal"),
        journal_before,
        "invalid snapshots must not rewrite the durable delivery"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_forbidden_upstream_inconclusive() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize synthetic webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for delivery_id in ["cli-forbidden-41-a", "cli-forbidden-41-b"] {
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_cli_http_fixture(
        403,
        json!({"message":"synthetic-403-body-must-not-leak"}),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-forbidden-41-a")
        .arg("cli-forbidden-41-b")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("403 fixture completed");
    assert!(
        !output.status.success(),
        "a forbidden read is not a success"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n"),
        "unexpected request path: {requests:?}"
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.contains("synthetic-403-body-must-not-leak"),
        "untrusted upstream error body must not be copied into CLI output"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("refusal is reported as structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "upstream_forbidden");
        assert_eq!(result["error"]["retryable"], false);
        assert!(result.get("canonical_state").is_none());
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_continues_mixed_batch_after_forbidden_resource() {
    const DENIED_NUMBER: u64 = ISSUE_NUMBER;
    const SURVIVING_NUMBER: u64 = 42;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_body = |number: u64, state: &str| {
        serde_json::to_vec(&json!({
            "action": "edited",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": number, "state": state}
        }))
        .expect("serialize signed mixed-batch issue webhook")
    };
    let denied_body = issue_body(DENIED_NUMBER, "closed");
    let surviving_body = issue_body(SURVIVING_NUMBER, "open");
    let deliveries = [
        ("cli-mixed-forbidden-41-a", denied_body.as_slice()),
        ("cli-mixed-surviving-42", surviving_body.as_slice()),
        ("cli-mixed-forbidden-41-b", denied_body.as_slice()),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (expected_sequence, (delivery_id, body)) in deliveries.iter().enumerate() {
        let (status, accepted) = send(address, &request(delivery_id, body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted mixed batch");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                403,
                json!({"message":"synthetic-batch-forbidden-body-must-not-leak"}),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": SURVIVING_NUMBER,
                    "state": "open",
                    "updated_at": "2026-10-25T21:00:00Z",
                    "html_url": format!(
                        "http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{SURVIVING_NUMBER}"
                    )
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(250),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _)| *delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run mixed batch after upstream forbidden response");
    let requests = fixture
        .join()
        .expect("403 plus successful issue fixture completed");
    assert_eq!(
        fs::read(&journal_path).expect("re-read inbox after mixed 403 batch"),
        journal_before,
        "partial authorization failure must not rewrite the durable inbox"
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        2,
        "the repeated forbidden issue is cached while the sibling issue is still read"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{DENIED_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{SURVIVING_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.contains("synthetic-batch-forbidden-body-must-not-leak"),
        "untrusted upstream 403 body must not be copied into batch output"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed forbidden batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 3);
    for index in [0, 2] {
        assert_eq!(results[index]["delivery_id"], deliveries[index].0);
        assert_eq!(results[index]["verdict"], "inconclusive");
        assert_eq!(results[index]["error"]["code"], "upstream_forbidden");
        assert_eq!(results[index]["error"]["retryable"], false);
        assert!(results[index].get("canonical_state").is_none());
    }
    assert_eq!(results[1]["delivery_id"], deliveries[1].0);
    assert_eq!(results[1]["verdict"], "current_state_read");
    assert_eq!(results[1]["resource"]["number"], SURVIVING_NUMBER);
    assert_eq!(results[1]["canonical_state"]["state"], "open");
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_continues_branch_batch_with_legacy_tag_delete() {
    const TAG_DELIVERY_ID: &str = "legacy-cli-mixed-tag-delete";
    const BRANCH_DELIVERY_ID: &str = "legacy-cli-mixed-branch-delete";
    const BRANCH: &str = "feature/agent-proof";
    const CURRENT_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";

    let scratch = ScratchDir::new();
    let journal_path = scratch.path().join("legacy-inbox.jsonl");
    let deliveries = [
        (
            TAG_DELIVERY_ID,
            "delete",
            json!({
                "ref": "refs/tags/v1.0.0",
                "ref_type": "tag",
                "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
            }),
        ),
        (
            BRANCH_DELIVERY_ID,
            "delete",
            json!({
                "ref": BRANCH,
                "ref_type": "branch",
                "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")}
            }),
        ),
    ];
    let source_hashes = write_synthetic_webhook_batch(&journal_path, &deliveries);
    let journal_before = fs::read(&journal_path).expect("snapshot synthetic legacy inbox");
    let persisted = EventJournal::open_webhook_inbox(&journal_path)
        .replay()
        .expect("synthetic legacy journal has a valid hash chain");
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[0].hash, source_hashes[0]);
    assert_eq!(persisted[1].previous_hash, source_hashes[0]);
    assert_eq!(persisted[1].hash, source_hashes[1]);

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "name": BRANCH,
            "commit": {
                "id": CURRENT_HEAD,
                "timestamp": "2026-10-25T18:00:00Z"
            }
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(TAG_DELIVERY_ID)
        .arg(BRANCH_DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run CLI on a synthetic pre-upgrade mixed batch");
    let requests = fixture.join().expect("branch fixture request completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1, "only the branch should be read upstream");
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/branches/feature%2Fagent-proof HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read legacy inbox after reconciliation"),
        journal_before,
        "reconciliation must not rewrite either legacy delivery"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("mixed legacy batch returns structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], TAG_DELIVERY_ID);
    assert_eq!(results[0]["verdict"], "inconclusive");
    assert_eq!(results[0]["error"]["code"], "unsupported_event");
    assert_eq!(results[0]["error"]["retryable"], false);
    assert_eq!(
        results[0]["error"]["details"]["field_path"],
        "payload.ref_type"
    );
    assert!(results[0].get("canonical_state").is_none());
    assert_eq!(results[1]["delivery_id"], BRANCH_DELIVERY_ID);
    assert_eq!(results[1]["verdict"], "current_state_read");
    assert_eq!(results[1]["resource"]["kind"], "branch");
    assert_eq!(results[1]["resource"]["branch"], BRANCH);
    assert_eq!(results[1]["canonical_state"]["state"], "present");
    assert_eq!(results[1]["canonical_state"]["head_sha"], CURRENT_HEAD);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reports_deleted_resource_in_single_delivery() {
    const DELIVERY_ID: &str = "cli-deleted-single-41";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize signed issue webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(DELIVERY_ID, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_cli_http_fixture(
        404,
        json!({"message":"synthetic-single-404-body-must-not-leak"}),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("single-resource HTTP fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("synthetic-single-404-body-must-not-leak"));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("single reconciliation failure is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["error"]["details"]["resource"]["kind"], "issue");
    assert_eq!(value["error"]["details"]["resource"]["owner"], OWNER);
    assert_eq!(
        value["error"]["details"]["resource"]["repository"],
        REPOSITORY
    );
    assert_eq!(
        value["error"]["details"]["resource"]["number"],
        ISSUE_NUMBER
    );
    let action = value["error"]["action"]
        .as_str()
        .expect("404 has a remediation action");
    assert!(action.contains("issue number"));
    assert!(action.contains("does not distinguish"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reports_deleted_pull_request_in_single_delivery() {
    const DELIVERY_ID: &str = "cli-deleted-pr-single-23";
    const PULL_REQUEST_NUMBER: u64 = 23;

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": PULL_REQUEST_NUMBER,
            "state": "closed",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize signed pull request webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request_event(DELIVERY_ID, "pull_request", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted PR journal");

    let fixture = start_cli_http_fixture(
        404,
        json!({"message":"synthetic-deleted-pull-request-body-must-not-leak"}),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("deleted pull request API fixture completed");
    assert_eq!(
        fs::read(&journal_path).expect("re-read PR journal after 404"),
        journal_before,
        "a deleted pull request must not mutate the durable inbox"
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.contains("synthetic-deleted-pull-request-body-must-not-leak"),
        "untrusted upstream error body must not be copied into CLI output"
    );

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("single pull request reconciliation refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["error"]["code"], "resource_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["resource"]["kind"],
        "pull_request"
    );
    assert_eq!(value["error"]["details"]["resource"]["owner"], OWNER);
    assert_eq!(
        value["error"]["details"]["resource"]["repository"],
        REPOSITORY
    );
    assert_eq!(
        value["error"]["details"]["resource"]["number"],
        PULL_REQUEST_NUMBER
    );
    assert!(value.get("canonical_state").is_none());
    let action = value["error"]["action"]
        .as_str()
        .expect("404 has a remediation action");
    assert!(action.contains("pull request number"));
    assert!(action.contains("does not distinguish"));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_keeps_deleted_pull_request_collision_separate_in_batch() {
    const NUMBER: u64 = ISSUE_NUMBER;
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": NUMBER, "state": "closed"}
    }))
    .expect("serialize signed same-number issue event");
    let pull_body = serde_json::to_vec(&json!({
        "action": "closed",
        "number": NUMBER,
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": NUMBER,
            "state": "closed",
            "merged": false,
            "head": {"sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "base": {"sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
        }
    }))
    .expect("serialize signed same-number pull request event");
    let deliveries = [
        (
            "cli-deleted-pr-collision-a-41",
            "pull_request",
            pull_body.as_slice(),
        ),
        (
            "cli-surviving-issue-collision-41",
            "issues",
            issue_body.as_slice(),
        ),
        (
            "cli-deleted-pr-collision-b-41",
            "pull_request",
            pull_body.as_slice(),
        ),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (expected_sequence, (delivery_id, event, body)) in deliveries.iter().enumerate() {
        let (status, accepted) = send(address, &request_event(delivery_id, event, body));
        assert_eq!(status, 202, "event {event}");
        assert_eq!(accepted["accepted"], true, "event {event}");
        assert_eq!(accepted["duplicate"], false, "event {event}");
        assert_eq!(accepted["sequence"], expected_sequence + 1, "event {event}");
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted collision batch");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                404,
                json!({"message":"synthetic-deleted-pr-collision-body-must-not-leak"}),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": NUMBER,
                    "state": "open",
                    "updated_at": "2026-10-25T20:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{NUMBER}")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(250),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _, _)| *delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run mixed same-number reconciliation batch");
    let requests = fixture
        .join()
        .expect("same-number PR and issue API fixture completed");
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after mixed 404 batch"),
        journal_before,
        "reconciliation must not rewrite the durable webhook inbox"
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        2,
        "the deleted PR is fetched once and must not poison the same-number issue cache entry"
    );
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.contains("synthetic-deleted-pr-collision-body-must-not-leak"),
        "untrusted upstream 404 body must not be copied into batch output"
    );

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("mixed batch result is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 3);
    for index in [0, 2] {
        assert_eq!(results[index]["delivery_id"], deliveries[index].0);
        assert_eq!(results[index]["verdict"], "inconclusive");
        assert_eq!(results[index]["error"]["code"], "resource_not_found");
        assert_eq!(results[index]["error"]["retryable"], false);
        assert_eq!(
            results[index]["error"]["details"]["resource"]["kind"],
            "pull_request"
        );
        assert_eq!(
            results[index]["error"]["details"]["resource"]["number"],
            NUMBER
        );
        assert!(results[index].get("canonical_state").is_none());
    }
    assert_eq!(results[1]["delivery_id"], deliveries[1].0);
    assert_eq!(results[1]["verdict"], "current_state_read");
    assert_eq!(results[1]["resource"]["kind"], "issue");
    assert_eq!(results[1]["resource"]["number"], NUMBER);
    assert_eq!(results[1]["canonical_state"]["state"], "open");
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_reports_deleted_resource_without_aborting_batch() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let issue_body = |number: u64, state: &str| {
        serde_json::to_vec(&json!({
            "action": "edited",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": number, "state": state}
        }))
        .expect("serialize signed issue webhook")
    };
    let deliveries = [
        ("cli-deleted-41-a", issue_body(ISSUE_NUMBER, "closed")),
        ("cli-deleted-41-b", issue_body(ISSUE_NUMBER, "opened")),
        ("cli-surviving-42", issue_body(42, "open")),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (expected_sequence, (delivery_id, body)) in deliveries.iter().enumerate() {
        let (status, accepted) = send(address, &request(delivery_id, body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], expected_sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                404,
                json!({"message":"synthetic-404-body-must-not-leak"}),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": 42,
                    "state": "open",
                    "updated_at": "2026-10-25T18:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/42")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(250),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _)| *delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("deleted-resource API fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        requests.len(),
        2,
        "404 is terminal and duplicate resources are cached"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        requests[1]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/42 HTTP/1.1\r\n")
    );
    for request in &requests {
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "the anonymous local CLI must not send an authorization header"
        );
    }

    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.contains("synthetic-404-body-must-not-leak"),
        "untrusted upstream error body must not be copied into CLI output"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("batch refusal is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 3);
    for (result, delivery_id) in results[..2]
        .iter()
        .zip(["cli-deleted-41-a", "cli-deleted-41-b"])
    {
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "resource_not_found");
        assert_eq!(result["error"]["retryable"], false);
        assert_eq!(result["error"]["details"]["resource"]["kind"], "issue");
        assert_eq!(result["error"]["details"]["resource"]["owner"], OWNER);
        assert_eq!(
            result["error"]["details"]["resource"]["repository"],
            REPOSITORY
        );
        assert_eq!(
            result["error"]["details"]["resource"]["number"],
            ISSUE_NUMBER
        );
        assert!(result.get("canonical_state").is_none());
    }
    assert_eq!(results[2]["delivery_id"], "cli-surviving-42");
    assert_eq!(results[2]["verdict"], "current_state_read");
    assert_eq!(results[2]["canonical_state"]["state"], "open");
    assert_eq!(results[2]["resource"]["number"], 42);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_commit_status_snapshot_cli_refuses_duplicate_case_variant_total_count_headers() {
    let commit_sha = "a".repeat(40);
    let fixture = start_cli_http_fixture_with_headers(
        200,
        json!({
            "state":"success",
            "sha":commit_sha,
            "total_count":1,
            "statuses":[{"status":"success","context":"ci/test"}],
            "repository":{"full_name":format!("{OWNER}/{REPOSITORY}")},
            "response_marker":"synthetic-duplicate-total-count-must-not-leak"
        }),
        1,
        vec![
            ("X-Total-Count".to_owned(), "1".to_owned()),
            ("x-total-count".to_owned(), "1".to_owned()),
        ],
        Some(Duration::from_millis(300)),
        None,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(["--commit-status-snapshot", &commit_sha])
        .stdin(Stdio::null())
        .output()
        .expect("run actual status snapshot CLI process");
    let (requests, _) = fixture.join().expect("duplicate-header fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "invalid headers cannot produce a snapshot"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Forgejo returned an invalid or mismatched status snapshot"),
        "refusal should tell the caller to avoid treating this as verification: {stderr}"
    );
    assert!(!stderr.contains("synthetic-duplicate-total-count-must-not-leak"));
    assert_eq!(requests.len(), 1, "invalid response is not retried");
    assert_eq!(
        requests[0].lines().next(),
        Some(
            format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/commits/{commit_sha}/status?limit=50&page=1 HTTP/1.1"
            )
            .as_str()
        )
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the unauthenticated local snapshot route must not send a credential"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_bounds_transient_upstream_retries() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize synthetic webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for delivery_id in ["cli-unavailable-41-a", "cli-unavailable-41-b"] {
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_cli_http_fixture(
        503,
        json!({"message":"synthetic-503-body-must-not-leak"}),
        3,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-unavailable-41-a")
        .arg("cli-unavailable-41-b")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture.join().expect("503 retry fixture completed");
    assert_eq!(
        requests.len(),
        3,
        "CLI uses only its three-attempt retry budget"
    );
    assert!(
        requests.iter().all(|request| request
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")),
        "unexpected request path: {requests:?}"
    );
    assert!(
        requests.iter().all(|request| !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))),
        "the anonymous local CLI must not send an authorization header"
    );
    assert!(
        !output.status.success(),
        "three transient upstream failures are not a success"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.contains("synthetic-503-body-must-not-leak"),
        "untrusted upstream error body must not be copied into CLI output"
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("unavailable state is structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 3);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "upstream_http_status");
        assert_eq!(result["error"]["retryable"], true);
        assert_eq!(result["error"]["details"]["http_status"], 503);
        assert!(result.get("canonical_state").is_none());
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_respects_retry_after_beyond_read_budget() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-retry-after-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize synthetic webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_cli_http_fixture_with_headers(
        429,
        json!({"message":"synthetic-429-body-must-not-leak"}),
        1,
        vec![("Retry-After".to_owned(), "3600".to_owned())],
        Some(Duration::from_millis(1_000)),
        None,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let (requests, _) = fixture.join().expect("Retry-After fixture completed");
    assert_eq!(
        requests.len(),
        1,
        "the CLI must not retry before Retry-After exceeds its 30-second read budget"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    assert!(!output.status.success());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("synthetic-429-body-must-not-leak"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("429 refusal is reported as structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_http_status");
    assert_eq!(value["error"]["retryable"], true);
    assert_eq!(value["error"]["details"]["http_status"], 429);
    assert_eq!(value["error"]["details"]["retry_after_ms"], 3_600_000);
    assert!(value.get("canonical_state").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_recovers_from_in_budget_retry_after() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-retry-after-recovers-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize the signed stale issue webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                429,
                json!({"message":"synthetic-rate-limit-body-must-not-leak"}),
                vec![("Retry-After".to_owned(), "0".to_owned())],
            ),
            (
                200,
                json!({
                    "number": ISSUE_NUMBER,
                    "state": "open",
                    "updated_at": "2026-10-25T22:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(300),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("transient response and canonical readback fixture completed");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(requests.len(), 2, "the bounded retry should finish on 200");
    assert!(requests.iter().all(|request| {
        request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
        ))
    }));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read webhook journal after retry"),
        journal_before,
        "API retry and reconciliation must not rewrite the durable inbox"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("synthetic-rate-limit-body-must-not-leak"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("successful reconciliation is JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 2);
    assert_eq!(value["canonical_state"]["state"], "open");
    assert_eq!(
        value["canonical_state"]["html_url"],
        format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
    );
    assert!(value.get("error").is_none());
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_waits_for_nonzero_in_budget_retry_after() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-retry-after-one-second-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize the signed stale issue webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted webhook journal");

    let fixture = start_scripted_cli_http_fixture_with_retry_timing(
        vec![
            (
                429,
                json!({"message":"synthetic-one-second-rate-limit-body-must-not-leak"}),
                vec![("Retry-After".to_owned(), "1".to_owned())],
            ),
            (
                200,
                json!({
                    "number": ISSUE_NUMBER,
                    "state": "open",
                    "updated_at": "2026-10-27T00:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(300),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI process");
    let (requests, retry_delay) = fixture
        .join()
        .expect("nonzero Retry-After and canonical readback fixture completed");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(
        retry_delay >= Duration::from_secs(1),
        "CLI retried before honoring the one-second Retry-After: {retry_delay:?}"
    );
    assert!(
        retry_delay < Duration::from_secs(10),
        "bounded retry took too long: {retry_delay:?}"
    );
    assert_eq!(requests.len(), 2, "the bounded retry should finish on 200");
    assert!(requests.iter().all(|request| {
        request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
        ))
    }));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read webhook journal after retry"),
        journal_before,
        "API retry and reconciliation must not rewrite the durable inbox"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("synthetic-one-second-rate-limit-body-must-not-leak"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("successful reconciliation is JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 2);
    assert_eq!(value["canonical_state"]["state"], "open");
    assert_eq!(
        value["canonical_state"]["html_url"],
        format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
    );
    assert!(value.get("error").is_none());
}

#[cfg(target_os = "linux")]
fn run_retry_after_interruption_and_resume(signal: libc::c_int) {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    let delivery_id = "cli-retry-after-cancel-resume-41";
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize the signed issue webhook");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot accepted inbox");
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);

    let (response_sent, stop_fixture, fixture) = start_retry_after_wait_cancellation_fixture();
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut interrupted = command
        .spawn()
        .expect("start reconciliation CLI before Retry-After wait");
    let process_group = ProcessGroupGuard(interrupted.id() as libc::pid_t);
    response_sent
        .recv_timeout(Duration::from_secs(5))
        .expect("fixture wrote the complete HTTP 429 response");
    thread::sleep(Duration::from_millis(200));
    assert!(
        interrupted
            .try_wait()
            .expect("poll CLI during Retry-After wait")
            .is_none(),
        "CLI should still be waiting for its five-second Retry-After"
    );
    assert_eq!(
        unsafe { libc::kill(-(interrupted.id() as libc::pid_t), signal) },
        0,
        "cancel CLI and any curl child as one process group"
    );
    let (interrupted_output, timed_out) =
        wait_for_process_output_with_timeout(interrupted, Duration::from_secs(3));
    assert!(!timed_out, "cancelled CLI process group stopped promptly");
    assert_eq!(interrupted_output.status.signal(), Some(signal));
    assert!(interrupted_output.stdout.is_empty());
    assert!(interrupted_output.stderr.is_empty());
    stop_fixture
        .send(())
        .expect("stop fixture after cancellation");
    let interrupted_requests = fixture.join().expect("cancellation fixture completed");
    assert_eq!(
        interrupted_requests.len(),
        1,
        "no retry request should arrive before cancellation"
    );
    assert!(interrupted_requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !interrupted_requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after cancellation"),
        journal_before,
        "cancellation during Retry-After must not mutate the accepted inbox"
    );

    let resume_fixture = start_cli_http_fixture_with_headers(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-27T00:00:00Z",
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        }),
        1,
        Vec::new(),
        Some(Duration::from_millis(300)),
        None,
    );
    let resumed = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("resume the same delivery from the unchanged inbox");
    let (resume_requests, _) = resume_fixture
        .join()
        .expect("resumed current-state fixture completed");
    assert!(resumed.status.success());
    assert_eq!(resume_requests.len(), 1);
    assert!(resume_requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(
        !resume_requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );
    let result: Value = serde_json::from_slice(&resumed.stdout).expect("resumed result is JSON");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "current_state_read");
    assert_eq!(result["delivery_id"], delivery_id);
    assert_eq!(result["upstream_read_attempts"], 1);
    assert_eq!(result["canonical_state"]["state"], "open");
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after successful resume"),
        journal_before,
        "successful resume also leaves the inbox byte-identical"
    );
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 1);
    drop(process_group);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
#[cfg(target_os = "linux")]
fn interrupted_reconcile_during_retry_after_resumes_from_unchanged_journal() {
    run_retry_after_interruption_and_resume(libc::SIGTERM);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
#[cfg(target_os = "linux")]
fn killed_reconcile_during_retry_after_resumes_from_unchanged_journal() {
    run_retry_after_interruption_and_resume(libc::SIGKILL);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_shares_recovered_issue_read_across_batch_deliveries() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let deliveries = [
        ("cli-retry-cache-41-first", "closed"),
        ("cli-retry-cache-41-second", "closed"),
    ];
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, state)) in deliveries.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "edited",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": ISSUE_NUMBER, "state": state}
        }))
        .expect("serialize duplicate-resource webhook delivery");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable batch before CLI");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                429,
                json!({"message":"synthetic-rate-limit-body-must-not-leak"}),
                vec![("Retry-After".to_owned(), "0".to_owned())],
            ),
            (
                200,
                json!({
                    "number": ISSUE_NUMBER,
                    "state": "open",
                    "updated_at": "2026-10-27T00:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(300),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _)| *delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run the actual batch reconciliation CLI process");
    let requests = fixture
        .join()
        .expect("recovered issue read fixture completed");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        requests.len(),
        2,
        "one 429 and its successful retry should be shared by both delivery results"
    );
    assert!(requests.iter().all(|request| {
        request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
        ))
    }));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read durable batch after recovered lookup"),
        journal_before,
        "successful retry and batch projection must not rewrite the inbox"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("synthetic-rate-limit-body-must-not-leak"));
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("successful batch result is JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 2);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), deliveries.len());
    for (result, (delivery_id, _)) in results.iter().zip(deliveries) {
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["resource"]["number"], ISSUE_NUMBER);
        assert_eq!(result["canonical_state"]["state"], "open");
        assert_eq!(
            result["canonical_state"]["html_url"],
            format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        );
        assert!(result.get("error").is_none());
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_shares_recovered_parent_issue_read_with_comment_batch() {
    const ISSUE_NUMBER: u64 = 41;
    const COMMENT_ID: u64 = 917;
    const ISSUE_DELIVERY_ID: &str = "cli-retry-cache-mixed-issue-41";
    const COMMENT_DELIVERY_ID: &str = "cli-retry-cache-mixed-comment-917";
    const RATE_LIMIT_MARKER: &str = "synthetic-mixed-retry-body-must-not-leak";
    const WEBHOOK_COMMENT: &str = "untrusted mixed comment webhook body";

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize signed issue delivery");
    let comment_body = serde_json::to_vec(&json!({
        "action": "created",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"},
        "comment": {"id": COMMENT_ID, "body": WEBHOOK_COMMENT},
        "is_pull": false
    }))
    .expect("serialize signed issue-comment delivery");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (sequence, (event, delivery_id, body)) in [
        ("issues", ISSUE_DELIVERY_ID, issue_body.as_slice()),
        (
            "issue_comment",
            COMMENT_DELIVERY_ID,
            comment_body.as_slice(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, accepted) = send(address, &request_event(delivery_id, event, body));
        assert_eq!(status, 202, "accepted {event} delivery: {accepted}");
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot mixed issue/comment inbox");

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                429,
                json!({"message": RATE_LIMIT_MARKER}),
                vec![("Retry-After".to_owned(), "0".to_owned())],
            ),
            (
                200,
                json!({
                    "number": ISSUE_NUMBER,
                    "state": "open",
                    "updated_at": "2026-10-27T09:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
                }),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "id": COMMENT_ID,
                    "body": "canonical mixed comment body",
                    "updated_at": "2026-10-27T09:01:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}#issuecomment-{COMMENT_ID}"),
                    "issue_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(300),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args([ISSUE_DELIVERY_ID, COMMENT_DELIVERY_ID])
        .stdin(Stdio::null())
        .output()
        .expect("run actual CLI for mixed issue/comment batch");
    let requests = fixture.join().expect("mixed retry fixture completed");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(requests.len(), 3, "429, recovered issue read, comment read");
    assert!(requests[0].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[1].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER} HTTP/1.1\r\n"
    )));
    assert!(requests[2].starts_with(&format!(
        "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/comments/{COMMENT_ID} HTTP/1.1\r\n"
    )));
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    assert_eq!(
        fs::read(&journal_path).expect("re-read mixed issue/comment inbox"),
        journal_before,
        "the retry and batch cache must not rewrite accepted deliveries"
    );
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains(RATE_LIMIT_MARKER));
    assert!(!transcript.contains(WEBHOOK_COMMENT));

    let value: Value = serde_json::from_slice(&output.stdout).expect("mixed batch returns JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 3);
    let results = value["results"].as_array().expect("batch result array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["delivery_id"], ISSUE_DELIVERY_ID);
    assert_eq!(results[0]["verdict"], "current_state_read");
    assert_eq!(results[0]["canonical_state"]["state"], "open");
    assert_eq!(results[1]["delivery_id"], COMMENT_DELIVERY_ID);
    assert_eq!(results[1]["verdict"], "current_state_read");
    assert_eq!(results[1]["resource"]["kind"], "issue_comment");
    assert_eq!(results[1]["resource"]["issue_number"], ISSUE_NUMBER);
    assert_eq!(results[1]["resource"]["comment_id"], COMMENT_ID);
    assert_eq!(results[1]["canonical_state"]["state"], "open");
    assert_eq!(results[1]["canonical_state"]["comment"]["state"], "present");
    assert_eq!(
        results[1]["canonical_state"]["comment"]["body"],
        "canonical mixed comment body"
    );
    assert_eq!(
        results[1]["canonical_state"]["comment"]["body_trust"],
        "untrusted_user_content"
    );
    assert!(results.iter().all(|result| result.get("error").is_none()));
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_resumes_mixed_batch_after_transient_failure() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let issue_body = |number: u64, state: &str| {
        serde_json::to_vec(&json!({
            "action": "edited",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": number, "state": state}
        }))
        .expect("serialize signed resume-batch issue")
    };
    let deliveries = [
        ("cli-resume-transient-41", issue_body(41, "closed")),
        ("cli-resume-sibling-42", issue_body(42, "closed")),
    ];
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, body)) in deliveries.iter().enumerate() {
        let (status, accepted) = send(address, &request(delivery_id, body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot durable batch before CLI");

    let first_fixture = start_scripted_cli_http_fixture(
        vec![
            (
                503,
                json!({"message":"synthetic-transient-batch-body-must-not-leak"}),
                Vec::new(),
            ),
            (
                503,
                json!({"message":"synthetic-transient-batch-body-must-not-leak"}),
                Vec::new(),
            ),
            (
                503,
                json!({"message":"synthetic-transient-batch-body-must-not-leak"}),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": 42,
                    "state": "open",
                    "updated_at": "2026-10-25T22:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/42")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(250),
    );
    let first = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _)| *delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run first reconciliation process with transient failure");
    let first_requests = first_fixture
        .join()
        .expect("first mixed transient fixture completed");
    assert_eq!(
        fs::read(&journal_path).expect("re-read inbox after first process"),
        journal_before,
        "transient failure and successful sibling must not rewrite the inbox"
    );

    assert_eq!(first.status.code(), Some(1));
    assert_eq!(first_requests.len(), 4);
    for request in &first_requests[..3] {
        assert!(
            request.starts_with(
                "GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n"
            )
        );
    }
    assert!(
        first_requests[3]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/42 HTTP/1.1\r\n")
    );
    assert!(first_requests.iter().all(|request| {
        !request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    }));
    let first_transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        !first_transcript.contains("synthetic-transient-batch-body-must-not-leak"),
        "untrusted transient bodies must not appear in CLI output"
    );
    let first_value: Value =
        serde_json::from_slice(&first.stdout).expect("first batch result is structured JSON");
    assert_reconcile_schema_version(&first_value);
    assert_eq!(first_value["verdict"], "inconclusive");
    assert_eq!(first_value["upstream_read_attempts"], 4);
    let first_results = first_value["results"]
        .as_array()
        .expect("first batch results array");
    assert_eq!(first_results.len(), 2);
    assert_eq!(first_results[0]["delivery_id"], deliveries[0].0);
    assert_eq!(first_results[0]["verdict"], "inconclusive");
    assert_eq!(first_results[0]["error"]["code"], "upstream_http_status");
    assert_eq!(first_results[0]["error"]["retryable"], true);
    assert_eq!(first_results[0]["error"]["details"]["http_status"], 503);
    assert!(first_results[0].get("canonical_state").is_none());
    assert_eq!(first_results[1]["delivery_id"], deliveries[1].0);
    assert_eq!(first_results[1]["verdict"], "current_state_read");
    assert_eq!(first_results[1]["canonical_state"]["state"], "open");

    let second_fixture = start_scripted_cli_http_fixture(
        vec![
            (
                200,
                json!({
                    "number": 41,
                    "state": "open",
                    "updated_at": "2026-10-25T22:01:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/41")
                }),
                Vec::new(),
            ),
            (
                200,
                json!({
                    "number": 42,
                    "state": "open",
                    "updated_at": "2026-10-25T22:02:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/42")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_millis(250),
    );
    let resumed = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _)| *delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("resume the same durable batch in a new CLI process");
    let second_requests = second_fixture
        .join()
        .expect("resumed successful fixture completed");
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after successful resume"),
        journal_before,
        "successful resume remains read-only"
    );

    assert!(resumed.status.success());
    assert_eq!(second_requests.len(), 2);
    for (request, number) in second_requests.iter().zip([41, 42]) {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{number} HTTP/1.1\r\n"
        )));
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        );
    }
    let resumed_value: Value =
        serde_json::from_slice(&resumed.stdout).expect("resumed result is structured JSON");
    assert_reconcile_schema_version(&resumed_value);
    assert_eq!(resumed_value["verdict"], "current_state_read");
    assert_eq!(resumed_value["upstream_read_attempts"], 2);
    let resumed_results = resumed_value["results"]
        .as_array()
        .expect("resumed batch results array");
    assert_eq!(resumed_results.len(), 2);
    for (result, (delivery_id, _)) in resumed_results.iter().zip(deliveries) {
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["canonical_state"]["state"], "open");
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_preserves_retry_after_for_replayed_batch_deliveries() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let deliveries = [
        ("batch-retry-after-41-original", 41),
        ("batch-retry-after-41-replay", 41),
        ("batch-retry-after-42", 42),
        ("batch-retry-after-43", 43),
    ];

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for (sequence, (delivery_id, issue_number)) in deliveries.iter().enumerate() {
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": issue_number, "state": "open"}
        }))
        .expect("serialize synthetic webhook payload");
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
        assert_eq!(accepted["sequence"], sequence + 1);
    }
    assert!(stop_receiver(&mut receiver).success());

    let fixture = start_scripted_cli_http_fixture(
        vec![
            (
                429,
                json!({"message":"synthetic-429-body-must-not-leak"}),
                vec![("Retry-After".to_owned(), "3600".to_owned())],
            ),
            (
                503,
                json!({"message":"synthetic-503-body-must-not-leak"}),
                vec![("Retry-After".to_owned(), "7200".to_owned())],
            ),
            (
                200,
                json!({
                    "number": 43,
                    "state": "open",
                    "updated_at": "2026-10-24T00:00:00Z",
                    "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/43")
                }),
                Vec::new(),
            ),
        ],
        Duration::from_secs(1),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .args(deliveries.iter().map(|(delivery_id, _)| delivery_id))
        .stdin(Stdio::null())
        .output()
        .expect("run the actual batch reconciliation CLI process");
    let requests = fixture.join().expect("batch Retry-After fixture completed");

    assert_eq!(
        requests.len(),
        3,
        "one read per resource; a replayed delivery must reuse the cached failure"
    );
    for (request, issue_number) in requests.iter().zip([41, 42, 43]) {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{issue_number} HTTP/1.1\r\n"
        )));
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "the anonymous local CLI must not send an authorization header"
        );
    }
    assert!(!output.status.success());
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!transcript.contains("synthetic-429-body-must-not-leak"));
    assert!(!transcript.contains("synthetic-503-body-must-not-leak"));

    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("mixed batch results are reported as structured JSON");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], 3);
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(results.len(), deliveries.len());
    for (result, (delivery_id, _)) in results.iter().zip(deliveries) {
        assert_eq!(result["delivery_id"], delivery_id);
    }
    for result in &results[..2] {
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "upstream_http_status");
        assert_eq!(result["error"]["retryable"], true);
        assert_eq!(result["error"]["details"]["http_status"], 429);
        assert_eq!(result["error"]["details"]["retry_after_ms"], 3_600_000);
        assert!(result.get("canonical_state").is_none());
    }
    assert_eq!(results[2]["verdict"], "inconclusive");
    assert_eq!(results[2]["error"]["code"], "upstream_http_status");
    assert_eq!(results[2]["error"]["retryable"], true);
    assert_eq!(results[2]["error"]["details"]["http_status"], 503);
    assert_eq!(results[2]["error"]["details"]["retry_after_ms"], 7_200_000);
    assert!(results[2].get("canonical_state").is_none());
    assert_eq!(results[3]["verdict"], "current_state_read");
    assert_eq!(results[3]["resource"]["number"], 43);
    assert_eq!(results[3]["canonical_state"]["state"], "open");
    assert_eq!(
        results[3]["canonical_state"]["html_url"],
        format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/43")
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_handles_invalid_and_http_date_retry_after() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-retry-after-invalid-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize synthetic webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());

    for (case, retry_after, expected_attempts, observe_after, http_date_delay) in [
        (
            "invalid",
            Some("tomorrow"),
            1,
            Some(Duration::from_millis(1_000)),
            None,
        ),
        (
            "beyond-budget HTTP date",
            Some("Thu, 31 Dec 2099 00:00:00 GMT"),
            1,
            Some(Duration::from_millis(1_000)),
            None,
        ),
        (
            "in-budget HTTP date",
            None,
            3,
            None,
            Some(Duration::from_secs(3)),
        ),
    ] {
        let retry_after_headers = retry_after
            .map(|value| vec![("Retry-After".to_owned(), value.to_owned())])
            .unwrap_or_default();
        let fixture = start_cli_http_fixture_with_headers(
            429,
            json!({"message":"synthetic-429-body-must-not-leak"}),
            expected_attempts,
            retry_after_headers,
            observe_after,
            http_date_delay,
        );
        let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
            .env_clear()
            .arg(&journal_path)
            .arg(delivery_id)
            .stdin(Stdio::null())
            .output()
            .expect("run the actual reconciliation CLI process");
        let (requests, response_to_next_request) =
            fixture.join().expect("Retry-After fixture completed");

        assert_eq!(
            requests.len(),
            expected_attempts,
            "unexpected number of requests for {case} Retry-After value {retry_after:?}"
        );
        assert!(
            requests[0].starts_with(
                "GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n"
            )
        );
        assert!(
            !requests[0]
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "the anonymous local CLI must not send an authorization header"
        );
        assert!(!output.status.success());
        let transcript = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!transcript.contains("synthetic-429-body-must-not-leak"));
        let value: Value = serde_json::from_slice(&output.stdout)
            .expect("429 refusal is reported as structured JSON");
        assert_reconcile_schema_version(&value);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["delivery_id"], delivery_id);
        assert_eq!(value["upstream_read_attempts"], expected_attempts);
        assert_eq!(value["error"]["code"], "upstream_http_status");
        assert_eq!(value["error"]["retryable"], true);
        assert_eq!(value["error"]["details"]["http_status"], 429);
        if case == "beyond-budget HTTP date" {
            assert!(
                value["error"]["details"]["retry_after_ms"]
                    .as_u64()
                    .is_some_and(|delay| delay > 30_000),
                "a valid future HTTP date must be surfaced when its delay exceeds the read budget"
            );
        } else {
            assert!(value["error"]["details"].get("retry_after_ms").is_none());
        }
        assert!(value.get("canonical_state").is_none());
        if http_date_delay.is_some() {
            assert!(
                response_to_next_request
                    .first()
                    .is_some_and(|delay| *delay >= Duration::from_secs(2)),
                "the first retry should honor the HTTP-date delay from the response"
            );
        }
    }
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_disables_retry_for_duplicate_retry_after_headers() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let delivery_id = "cli-retry-after-duplicate-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize synthetic issue webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (accepted_status, accepted) = send(address, &request(delivery_id, &body));
    assert_eq!(accepted_status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("read accepted webhook journal");

    let fixture = start_cli_http_fixture_with_headers(
        429,
        json!({"message":"synthetic-duplicate-retry-after-body-must-not-leak"}),
        1,
        vec![
            ("Retry-After".to_owned(), "0".to_owned()),
            ("retry-after".to_owned(), "0".to_owned()),
        ],
        Some(Duration::from_millis(300)),
        None,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run actual webhook reconciliation CLI process");
    let (requests, _) = fixture
        .join()
        .expect("duplicate Retry-After fixture completed");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("duplicate Retry-After is a structured inconclusive result");
    assert_reconcile_schema_version(&value);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], delivery_id);
    assert_eq!(value["upstream_read_attempts"], 1);
    assert_eq!(value["error"]["code"], "upstream_http_status");
    assert_eq!(value["error"]["details"]["http_status"], 429);
    assert_eq!(
        value["error"]["details"]["retry_after_disposition"],
        "invalid_or_ambiguous"
    );
    assert!(
        value["error"]["action"]
            .as_str()
            .expect("structured retry guidance")
            .contains("Retry-After is invalid or ambiguous")
    );
    assert_eq!(value["error"]["retryable"], true);
    assert!(value["error"]["details"].get("retry_after_ms").is_none());
    assert!(value.get("canonical_state").is_none());
    assert!(
        !output
            .stdout
            .windows(b"synthetic-duplicate-retry-after-body-must-not-leak".len())
            .any(|window| window == b"synthetic-duplicate-retry-after-body-must-not-leak"),
        "the untrusted 429 body must not appear in the result"
    );
    assert_eq!(requests.len(), 1, "ambiguous Retry-After disables retries");
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );
    assert_eq!(
        fs::read(&journal_path).expect("read journal after reconciliation"),
        journal_before,
        "API failure must not rewrite the accepted source journal"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
#[cfg(target_os = "linux")]
fn reconcile_cli_terminates_curl_child_when_parent_is_signalled() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize signed cancellation fixture event");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let (status, accepted) = send(address, &request("cancel-parent-only-41", &body));
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot journal before cancellation");

    let (fixture, request_receiver, release_fixture) = start_gated_cli_http_fixture();
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .arg("cancel-parent-only-41")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut cli = command.spawn().expect("start reconciliation CLI");
    let process_group = ProcessGroupGuard(cli.id() as libc::pid_t);
    let cli_request = request_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("CLI reached the gated Forgejo fixture");
    assert!(
        cli_request
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );

    let child_list = fs::read_to_string(format!("/proc/{}/task/{}/children", cli.id(), cli.id()))
        .expect("read CLI child list before interrupt");
    let curl_pid = child_list
        .split_whitespace()
        .next()
        .expect("CLI has a curl child")
        .parse::<libc::pid_t>()
        .expect("curl child PID is numeric");
    assert_eq!(
        unsafe { libc::kill(cli.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    let cli_status = cli.wait().expect("reap interrupted CLI");
    assert_eq!(cli_status.signal(), Some(libc::SIGTERM));

    let curl_stopped = wait_until_process_exited_or_zombie(curl_pid, Duration::from_secs(2));
    release_fixture
        .send(())
        .expect("release fixture after checking child lifecycle");
    assert!(fixture.join().is_ok(), "fixture thread completed");
    assert_eq!(
        fs::read(&journal_path).expect("re-read journal after cancellation"),
        journal_before,
        "cancellation leaves the signed inbox unchanged"
    );
    drop(process_group);
    assert!(
        curl_stopped,
        "curl child {curl_pid} must stop when only its CLI parent receives SIGTERM"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn interrupted_reconcile_cli_can_resume_from_the_unchanged_journal() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize synthetic webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for delivery_id in ["cli-resume-41-a", "cli-resume-41-b"] {
        let (status, accepted) = send(address, &request(delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["duplicate"], false);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("read journal before interruption");
    assert_eq!(EventJournal::open(&journal_path).replay().unwrap().len(), 2);

    let (fixture, request_receiver, release_fixture) = start_gated_cli_http_fixture();
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .arg("cli-resume-41-a")
        .arg("cli-resume-41-b")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut interrupted = command
        .spawn()
        .expect("start reconciliation CLI to interrupt");
    let cli_request = request_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("interrupted CLI reached the gated Forgejo fixture");
    assert!(
        cli_request
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n"),
        "unexpected interrupted request: {cli_request:?}"
    );
    assert!(
        !cli_request
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the anonymous local CLI must not send an authorization header"
    );

    let process_group = interrupted.id() as libc::pid_t;
    let result = unsafe { libc::kill(-process_group, libc::SIGTERM) };
    assert_eq!(
        result, 0,
        "interrupt CLI and its curl child as one process group"
    );
    release_fixture
        .send(())
        .expect("release the gated fixture after interruption");
    let exit_deadline = Instant::now() + Duration::from_secs(3);
    let interrupted_status = loop {
        if let Some(status) = interrupted
            .try_wait()
            .expect("poll interrupted reconciliation CLI")
        {
            break status;
        }
        if Instant::now() >= exit_deadline {
            let _ = unsafe { libc::kill(-process_group, libc::SIGKILL) };
            let _ = interrupted.wait();
            panic!("interrupted CLI process group did not stop within three seconds");
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        interrupted_status.signal(),
        Some(libc::SIGTERM),
        "CLI stopped by the requested interruption"
    );
    let interrupted_output = interrupted
        .wait_with_output()
        .expect("collect output after interrupted CLI exit");
    assert!(interrupted_output.stdout.is_empty());
    assert!(fixture.join().is_ok(), "gated fixture thread completed");
    assert_eq!(
        fs::read(&journal_path).expect("read journal after interruption"),
        journal_before,
        "interruption does not mutate the inbox journal"
    );

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41"
        }),
        1,
    );
    let resumed = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg("cli-resume-41-a")
        .arg("cli-resume-41-b")
        .stdin(Stdio::null())
        .output()
        .expect("resume reconciliation CLI from the same journal");
    let requests = fixture.join().expect("resumed API fixture completed");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(resumed.status.success());
    let value: Value = serde_json::from_slice(&resumed.stdout).expect("resumed output is JSON");
    assert_eq!(value["verdict"], "current_state_read");
    assert_eq!(value["upstream_read_attempts"], 1);
    let results = value["results"].as_array().expect("resumed batch results");
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["canonical_state"]["state"], "open");
    }
    assert_eq!(
        fs::read(&journal_path).expect("read journal after successful resume"),
        journal_before,
        "successful resume remains read-only"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_cancels_slow_reads_at_the_shared_budget() {
    const RESOURCE_COUNT: u64 = 9;
    const EXPECTED_DISTINCT_RESOURCES_BEFORE_EXPIRY: usize = 8;

    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let mut delivery_ids = Vec::new();
    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    for offset in 0..RESOURCE_COUNT {
        let number = ISSUE_NUMBER + offset;
        let delivery_id = format!("shared-budget-{number}");
        let body = serde_json::to_vec(&json!({
            "action": "closed",
            "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
            "issue": {"number": number, "state": "closed"}
        }))
        .expect("serialize distinct issue event");
        let (status, accepted) = send(address, &request(&delivery_id, &body));
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["sequence"], offset + 1);
        delivery_ids.push(delivery_id);
    }
    assert!(stop_receiver(&mut receiver).success());
    let journal_before = fs::read(&journal_path).expect("snapshot signed inbox before reads");

    let (stop_fixture, release_fixture) = mpsc::channel();
    let fixture = start_slow_cli_http_fixture(
        Duration::from_secs(3),
        EXPECTED_DISTINCT_RESOURCES_BEFORE_EXPIRY - 1,
        Duration::from_secs(30),
        release_fixture,
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command
        .env_clear()
        .arg(&journal_path)
        .args(&delivery_ids)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let started = Instant::now();
    let child = command
        .spawn()
        .expect("spawn bounded reconciliation process");
    let (output, timed_out) = wait_for_process_output_with_timeout(child, Duration::from_secs(35));
    let cli_elapsed = started.elapsed();
    stop_fixture
        .send(())
        .expect("release the slow fixture after CLI termination");
    let requests = fixture.join().expect("slow HTTP fixture completed");

    assert!(!timed_out, "CLI exceeded its bounded process deadline");
    assert!(
        !output.status.success(),
        "expired read budget is not a success"
    );
    assert!(
        cli_elapsed >= Duration::from_secs(29),
        "CLI returned before reaching the configured 30-second shared read budget: {cli_elapsed:?}"
    );
    assert!(
        cli_elapsed < Duration::from_secs(32),
        "CLI exceeded its 30-second shared read budget: {cli_elapsed:?}"
    );
    assert!(
        (EXPECTED_DISTINCT_RESOURCES_BEFORE_EXPIRY..=10).contains(&requests.len()),
        "the seven fast resources and bounded retries of the eighth produce 8-10 GET attempts; observed {requests:?}"
    );
    let read_deadline = started + Duration::from_secs(30);
    for (received_at, _) in &requests {
        assert!(
            *received_at < read_deadline,
            "HTTP GET reached the fixture after the shared read deadline: {:?}",
            received_at.duration_since(started)
        );
    }
    for (offset, (received_at, request)) in requests.iter().take(7).enumerate() {
        assert!(
            received_at.duration_since(started) < Duration::from_secs(30),
            "fast resource request arrived after the shared budget"
        );
        let expected_number = ISSUE_NUMBER + u64::try_from(offset).expect("small index");
        assert!(
            request.starts_with(&format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{expected_number} HTTP/1.1\r\n"
            )),
            "unexpected or reordered request at index {offset}: {request:?}"
        );
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "anonymous CLI must not send an authorization header"
        );
    }
    for (_, request) in requests.iter().skip(7) {
        assert!(
            request.starts_with(&format!(
                "GET /api/v1/repos/{OWNER}/{REPOSITORY}/issues/{} HTTP/1.1\r\n",
                ISSUE_NUMBER + 7
            )),
            "no ninth distinct resource GET is sent after the slow resource exhausts the shared deadline: {request:?}"
        );
        assert!(
            !request
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
            "anonymous CLI must not send an authorization header"
        );
    }

    let value: Value =
        serde_json::from_slice(&output.stdout).expect("expired read budget returns JSON");
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["upstream_read_attempts"], requests.len());
    let results = value["results"].as_array().expect("batch results array");
    assert_eq!(
        results.len(),
        usize::try_from(RESOURCE_COUNT).expect("small resource count")
    );
    for result in &results[..7] {
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["canonical_state"]["state"], "open");
    }
    for (result, delivery_id) in results[7..]
        .iter()
        .zip(delivery_ids[7..].iter().map(String::as_str))
    {
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["delivery_id"], delivery_id);
        assert_eq!(result["error"]["code"], "upstream_unavailable");
    }
    assert_eq!(
        fs::read(&journal_path).expect("read signed inbox after timeout"),
        journal_before,
        "timeouts and retries never rewrite delivery events"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_reconcile_cli_process_accepts_complete_request_before_client_half_close() {
    let scratch = ScratchDir::new();
    let secret = scratch.path().join("webhook.secret");
    let journal_path = scratch.path().join("inbox.jsonl");
    write_secret(&secret);

    let delivery_id = "keep-open-write-side-41";
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "closed"}
    }))
    .expect("serialize signed webhook payload");

    let (mut receiver, address) = start_ready_receiver(&secret, &journal_path);
    let mut wire_request = request(delivery_id, &body);
    let header_end = wire_request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("request has a header terminator");
    wire_request.splice(
        header_end + 2..header_end + 2,
        b"Connection: keep-alive\r\n".iter().copied(),
    );

    let mut client = TcpStream::connect(address).expect("connect without closing the write side");
    client
        .set_read_timeout(Some(Duration::from_secs(4)))
        .expect("set bounded response timeout");
    client
        .write_all(&wire_request)
        .expect("send complete signed request while keeping the write side open");
    // Deliberately do not call shutdown(Write): HTTP/1.1 clients normally finish
    // the framed body and continue waiting for the response on the same socket.
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .expect("receiver responds from Content-Length without waiting for EOF");
    let response = String::from_utf8(response).expect("receiver response is UTF-8");
    let (headers, response_body) = response
        .split_once("\r\n\r\n")
        .expect("receiver response has an HTTP header terminator");
    assert!(headers.starts_with("HTTP/1.1 202 "));
    assert!(
        headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("Connection: close")),
        "receiver closes after responding rather than waiting for client EOF"
    );
    let accepted: Value = serde_json::from_str(response_body).expect("acceptance response is JSON");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["delivery_id"], delivery_id);
    assert!(stop_receiver(&mut receiver).success());

    let inbox_before_reconcile = fs::read(&journal_path).expect("read accepted webhook inbox");
    let stored = EventJournal::open(&journal_path)
        .replay()
        .expect("replay accepted webhook inbox");
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].idempotency_key,
        format!("forgejo-delivery-v1:{delivery_id}")
    );

    let fixture = start_cli_http_fixture(
        200,
        json!({
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-22T17:00:00Z",
            "html_url": "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41"
        }),
        1,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal_path)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("run reconciliation against the private Forgejo API fixture");
    let requests = fixture.join().expect("API fixture completed");
    assert!(
        output.status.success(),
        "reconciliation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
    );

    let result: Value = serde_json::from_slice(&output.stdout).expect("CLI result is JSON");
    assert_reconcile_schema_version(&result);
    assert_eq!(result["verdict"], "current_state_read");
    assert_eq!(result["upstream_read_attempts"], 1);
    assert_eq!(result["delivery_id"], delivery_id);
    assert_eq!(result["resource"]["number"], ISSUE_NUMBER);
    assert_eq!(result["canonical_state"]["state"], "open");
    assert_eq!(
        fs::read(&journal_path).expect("read inbox after reconciliation"),
        inbox_before_reconcile,
        "reconciliation remains read-only"
    );
}
