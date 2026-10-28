#![cfg(unix)]

//! Process-level admission proof for the fixed local mission adapter.
//!
//! Run networked process checks only through
//! `scripts/run_forgejo_mission_delivery_process_e2e.py`. The verifier-only
//! process probe opens no sockets; its candidate test process is independently
//! confined by the verifier's nested Bubblewrap/systemd scope.

use agent_forge_core::{EventJournal, mission_admission::MissionHostAdmissionV1};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    os::fd::AsRawFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

const WEBHOOK_SECRET: &[u8] = b"synthetic-process-e2e-secret-only";
const DEFAULT_ATTESTATION_SEED: [u8; 32] = [42; 32];
const OWNER: &str = "forge-admin";
const REPOSITORY: &str = "agent-forge-core";
const ISSUE_NUMBER: u64 = 41;
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const ADVANCED_COMMIT: &str = "fedcba9876543210fedcba9876543210fedcba98";
const REQUIREMENTS: &[u8] = br#"{"schema_version":2,"id":"REQ-001","revision":1,"source_issue_number":41,"request":"Parse durations.","acceptance":["250ms is 250 milliseconds."],"ambiguities":[]}"#;
const REQUIREMENTS_CHANGED_SAME_REVISION: &[u8] = br#"{"schema_version":2,"id":"REQ-001","revision":1,"source_issue_number":41,"request":"Parse durations with strict units.","acceptance":["250ms is 250 milliseconds.","Unknown units are rejected."],"ambiguities":[]}"#;
const REQUIREMENTS_V3: &[u8] = br#"{"schema_version":3,"id":"REQ-001","revision":1,"source_issue_number":41,"source_pull_request_number":77,"request":"Parse durations with strict units.","acceptance":["250ms is 250 milliseconds."],"ambiguities":[],"decisions":[{"ambiguity":"Bare integer units","resolution":"Bare integers mean milliseconds.","rationale":"This preserves the existing CLI convention."}]}"#;
const SOURCE: &[u8] = b"pub enum ParseDurationError { Empty, InvalidNumber, Overflow }\npub fn parse_duration_ms(input: &str) -> Result<u64, ParseDurationError> {\n    let _ = input;\n    Ok(0)\n}\n";
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

struct Receiver(Child);

impl Receiver {
    fn stop(&mut self) -> ExitStatus {
        if self.0.try_wait().expect("check receiver status").is_none() {
            // SAFETY: this sends SIGTERM to the receiver child spawned below.
            let result = unsafe { libc::kill(self.0.id() as i32, libc::SIGTERM) };
            assert_eq!(result, 0, "request graceful receiver shutdown");
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.0.try_wait().expect("wait for receiver shutdown") {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.0.kill();
                return self.0.wait().expect("reap timed-out receiver");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

struct ForgejoFixture {
    issue_closed: Arc<AtomicBool>,
    issue_edited: Arc<AtomicBool>,
    issue_labels_changed: Arc<AtomicBool>,
    advance_branch_after_source: Arc<AtomicBool>,
    edit_issue_after_requirement_inventory: Arc<AtomicBool>,
    advance_branch_after_requirement_inventory: Arc<AtomicBool>,
    requirements: Arc<Mutex<Vec<u8>>>,
    paths: Arc<Mutex<Vec<String>>>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ForgejoFixture {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:3130")
            .expect("reserve the fixed Forgejo endpoint inside the private network namespace");
        listener
            .set_nonblocking(true)
            .expect("make the fixture accept loop stoppable");
        let issue_closed = Arc::new(AtomicBool::new(true));
        let issue_edited = Arc::new(AtomicBool::new(false));
        let issue_labels_changed = Arc::new(AtomicBool::new(false));
        let branch_commit = Arc::new(Mutex::new(COMMIT.to_owned()));
        let advance_branch_after_source = Arc::new(AtomicBool::new(false));
        let edit_issue_after_requirement_inventory = Arc::new(AtomicBool::new(false));
        let advance_branch_after_requirement_inventory = Arc::new(AtomicBool::new(false));
        let requirements = Arc::new(Mutex::new(REQUIREMENTS.to_vec()));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let worker_issue_closed = Arc::clone(&issue_closed);
        let worker_issue_edited = Arc::clone(&issue_edited);
        let worker_issue_labels_changed = Arc::clone(&issue_labels_changed);
        let worker_branch_commit = Arc::clone(&branch_commit);
        let worker_advance_branch_after_source = Arc::clone(&advance_branch_after_source);
        let worker_edit_issue_after_requirement_inventory =
            Arc::clone(&edit_issue_after_requirement_inventory);
        let worker_advance_branch_after_requirement_inventory =
            Arc::clone(&advance_branch_after_requirement_inventory);
        let worker_requirements = Arc::clone(&requirements);
        let worker_paths = Arc::clone(&paths);
        let (stop_sender, stop_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(45);
            loop {
                match stop_receiver.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                assert!(
                    Instant::now() < deadline,
                    "Forgejo fixture exceeded its deadline"
                );
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .expect("set a bounded fixture request timeout");
                        let request = read_request_headers(&mut stream);
                        let path = request_path(&request);
                        let requirement_bytes = worker_requirements.lock().unwrap().clone();
                        let current_branch_commit = worker_branch_commit.lock().unwrap().clone();
                        let body = fixture_body(
                            &path,
                            worker_issue_closed.load(Ordering::SeqCst),
                            worker_issue_edited.load(Ordering::SeqCst),
                            worker_issue_labels_changed.load(Ordering::SeqCst),
                            &current_branch_commit,
                            &requirement_bytes,
                        );
                        let context_source_path = format!(
                            "/{OWNER}/{REPOSITORY}/raw/commit/{current_branch_commit}/src/lib.rs"
                        );
                        if path == context_source_path
                            && worker_advance_branch_after_source.swap(false, Ordering::SeqCst)
                        {
                            *worker_branch_commit.lock().unwrap() = ADVANCED_COMMIT.to_owned();
                        }
                        let requirement_tree_prefix = format!(
                            "/api/v1/repos/{OWNER}/{REPOSITORY}/git/trees/{current_branch_commit}?"
                        );
                        if path.starts_with(&requirement_tree_prefix)
                            && worker_edit_issue_after_requirement_inventory
                                .swap(false, Ordering::SeqCst)
                        {
                            // Mutate the API's current issue snapshot while the
                            // admission is validating protected requirement paths.
                            worker_issue_edited.store(true, Ordering::SeqCst);
                        }
                        if path.starts_with(&requirement_tree_prefix)
                            && worker_advance_branch_after_requirement_inventory
                                .swap(false, Ordering::SeqCst)
                        {
                            // Move the protected branch after admission has read
                            // its head but while it inventories that exact commit.
                            *worker_branch_commit.lock().unwrap() = ADVANCED_COMMIT.to_owned();
                        }
                        worker_paths.lock().unwrap().push(path.clone());
                        let review_count_header = if path
                            == format!(
                                "/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77/reviews?page=1&limit=50"
                            ) {
                            "X-Total-Count: 1\r\n"
                        } else {
                            ""
                        };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
                            body.len(),
                            review_count_header
                        );
                        stream
                            .write_all(response.as_bytes())
                            .expect("write fixture response headers");
                        stream
                            .write_all(&body)
                            .expect("write fixture response body");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept fixed Forgejo fixture request: {error}"),
                }
            }
        });
        Self {
            issue_closed,
            issue_edited,
            issue_labels_changed,
            advance_branch_after_source,
            edit_issue_after_requirement_inventory,
            advance_branch_after_requirement_inventory,
            requirements,
            paths,
            stop: Some(stop_sender),
            worker: Some(worker),
        }
    }

    fn set_issue_closed(&self, closed: bool) {
        self.issue_closed.store(closed, Ordering::SeqCst);
    }

    fn set_issue_edited(&self, edited: bool) {
        self.issue_edited.store(edited, Ordering::SeqCst);
    }

    fn set_issue_labels_changed(&self, changed: bool) {
        self.issue_labels_changed.store(changed, Ordering::SeqCst);
    }

    fn advance_branch_after_context_source(&self) {
        self.advance_branch_after_source
            .store(true, Ordering::SeqCst);
    }

    fn edit_issue_after_requirement_inventory(&self) {
        self.edit_issue_after_requirement_inventory
            .store(true, Ordering::SeqCst);
    }

    fn advance_branch_after_requirement_inventory(&self) {
        self.advance_branch_after_requirement_inventory
            .store(true, Ordering::SeqCst);
    }

    fn set_requirements(&self, requirements: &[u8]) {
        *self.requirements.lock().unwrap() = requirements.to_vec();
    }

    fn paths(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker.join().expect("Forgejo fixture worker exits cleanly");
        }
    }
}

impl Drop for ForgejoFixture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn fixture_body(
    path: &str,
    issue_closed: bool,
    issue_edited: bool,
    issue_labels_changed: bool,
    commit: &str,
    requirements: &[u8],
) -> Vec<u8> {
    let issue_path = format!("/api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}");
    let branch_path = format!("/api/v1/repos/{OWNER}/{REPOSITORY}/branches/main");
    let tree_prefix = format!("/api/v1/repos/{OWNER}/{REPOSITORY}/git/trees/{commit}?");
    let requirements_path =
        format!("/{OWNER}/{REPOSITORY}/raw/commit/{commit}/requirements/REQ-001.json");
    let source_path = format!("/{OWNER}/{REPOSITORY}/raw/commit/{commit}/src/lib.rs");
    let source_pull_request_path = format!("/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77");
    let source_reviews_path =
        format!("/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/77/reviews?page=1&limit=50");
    if path == issue_path {
        serde_json::to_vec(&json!({
            "number": ISSUE_NUMBER,
            "state": if issue_closed { "closed" } else { "open" },
            "updated_at": if issue_edited { "2026-10-23T18:00:00Z" } else { "2026-10-22T17:00:00Z" },
            "title": if issue_edited { "Edited title from Forgejo API" } else { "Current title from Forgejo API" },
            "body": if issue_edited { "Edited body from Forgejo API; changed after the first claim." } else { "Current body from Forgejo API; this is optional untrusted context." },
            "labels": if issue_edited || issue_labels_changed { json!([{"id": 12, "name": "edited/current"}]) } else { json!([{"id": 7, "name": "area/triage"}]) },
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}")
        }))
        .unwrap()
    } else if path == branch_path {
        serde_json::to_vec(&json!({
            "name": "main",
            "protected": true,
            "commit": {"id": commit}
        }))
        .unwrap()
    } else if path == "/api/v1/settings/api" {
        br#"{"default_git_trees_per_page":1000}"#.to_vec()
    } else if path.starts_with(&tree_prefix) {
        serde_json::to_vec(&json!({
            "page": 1,
            "total_count": 2,
            "truncated": false,
            "tree": [
                {"path":"requirements/REQ-001.json","type":"blob","mode":"100644"},
                {"path":"src/lib.rs","type":"blob","mode":"100644"}
            ]
        }))
        .unwrap()
    } else if path == requirements_path {
        requirements.to_vec()
    } else if path == source_path {
        SOURCE.to_vec()
    } else if path == source_pull_request_path {
        serde_json::to_vec(&json!({
            "number": 77,
            "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/77"),
            "state": "closed",
            "merged": true,
            "draft": false,
            "mergeable": true,
            "base": {
                "ref": "main",
                "sha": commit,
                "repo": {"full_name": format!("{OWNER}/{REPOSITORY}")}
            },
            "head": {"sha": "fedcba9876543210fedcba9876543210fedcba98"},
            "merge_commit_sha": commit
        }))
        .unwrap()
    } else if path == source_reviews_path {
        serde_json::to_vec(&json!([{
            "id": 7001,
            "user": {"id": 9001},
            "state": "APPROVED",
            "commit_id": "fedcba9876543210fedcba9876543210fedcba98",
            "pull_request_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/77"),
            "stale": false,
            "official": true,
            "dismissed": false
        }]))
        .unwrap()
    } else {
        serde_json::to_vec(&json!({"error":"unexpected fixture path","path":path})).unwrap()
    }
}

fn read_request_headers(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = stream.read(&mut buffer).expect("read fixture request");
        assert_ne!(count, 0, "fixture request ended before its headers");
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        assert!(
            bytes.len() <= 16 * 1024,
            "fixture request headers exceed bound"
        );
    }
    String::from_utf8(bytes).expect("fixture request is UTF-8")
}

fn request_path(request: &str) -> String {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("fixture request line contains a path")
        .to_owned()
}

fn write_synthetic_secret(path: &Path) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create synthetic receiver secret with mode 0600");
    file.write_all(WEBHOOK_SECRET)
        .expect("write synthetic receiver secret");
    file.sync_all().expect("flush synthetic receiver secret");
    let metadata = fs::symlink_metadata(path).expect("inspect synthetic secret metadata");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
}

fn write_synthetic_attestation_seed(path: &Path, seed: &[u8; 32]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create synthetic receiver attestation seed with mode 0600");
    file.write_all(seed)
        .expect("write synthetic receiver attestation seed");
    file.sync_all()
        .expect("flush synthetic receiver attestation seed");
    let metadata = fs::symlink_metadata(path).expect("inspect synthetic attestation metadata");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
}

fn signed_webhook_request(delivery_id: &str, body: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(WEBHOOK_SECRET).expect("valid synthetic HMAC key");
    mac.update(body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let headers = format!(
        "POST /forgejo/webhook HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Content-Length: {}\r\n\
         Content-Type: application/json\r\n\
         X-Forgejo-Delivery: {delivery_id}\r\n\
         X-Forgejo-Event: issues\r\n\
         X-Forgejo-Signature: {signature}\r\n",
        body.len()
    );
    let mut request = format!("{headers}\r\n").into_bytes();
    request.extend_from_slice(body);
    request
}

fn send_webhook(address: SocketAddr, delivery_id: &str, body: &[u8]) -> (u16, Value) {
    let mut stream = TcpStream::connect(address).expect("connect to actual webhook receiver");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set bounded webhook response timeout");
    stream
        .write_all(&signed_webhook_request(delivery_id, body))
        .expect("send HMAC-signed synthetic webhook");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish webhook request body");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read webhook receiver response");
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .expect("receiver response contains header terminator");
    let status = headers
        .split_whitespace()
        .nth(1)
        .expect("receiver response status exists")
        .parse::<u16>()
        .expect("receiver response status is numeric");
    (
        status,
        serde_json::from_str(body).expect("receiver response body is JSON"),
    )
}

fn start_receiver(secret: &Path, journal: &Path) -> (Receiver, SocketAddr) {
    let seed_path = journal.with_extension("attestation.seed");
    write_synthetic_attestation_seed(&seed_path, &DEFAULT_ATTESTATION_SEED);
    start_receiver_inner(secret, journal, Some(&seed_path))
}

fn start_receiver_with_attestation(
    secret: &Path,
    journal: &Path,
    seed_file: &Path,
) -> (Receiver, SocketAddr) {
    start_receiver_inner(secret, journal, Some(seed_file))
}

fn start_receiver_inner(
    secret: &Path,
    journal: &Path,
    seed_file: Option<&Path>,
) -> (Receiver, SocketAddr) {
    for _ in 0..5 {
        let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = reservation
            .local_addr()
            .expect("read reserved loopback address");
        drop(reservation);
        let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"));
        command
            .env_clear()
            .arg(address.to_string())
            .arg(secret)
            .arg(journal);
        if let Some(seed_file) = seed_file {
            command.arg(seed_file);
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("launch actual forgejo-webhook-local binary");
        let mut receiver = Receiver(child);
        if receiver_is_ready(&mut receiver, address) {
            return (receiver, address);
        }
        drop(receiver);
    }
    panic!("actual webhook receiver did not pass its readiness probe after five attempts");
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
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100))
            && stream
                .set_read_timeout(Some(Duration::from_millis(300)))
                .is_ok()
                && stream
                    .write_all(b"GET /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n")
                    .is_ok()
                && stream.shutdown(Shutdown::Write).is_ok()
        {
            let mut response = String::new();
            if stream.read_to_string(&mut response).is_ok()
                && response.starts_with("HTTP/1.1 405 ")
                && receiver.0.try_wait().expect("check receiver after probe").is_none()
            {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn mission_cli(
    delivery_id: &str,
    requirement_sha256: &str,
    host_admission_file: &Path,
) -> (Option<i32>, Value) {
    let public_key = hex::encode(
        SigningKey::from_bytes(&DEFAULT_ATTESTATION_SEED)
            .verifying_key()
            .as_bytes(),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-mission-local"))
        .env_clear()
        .args(BINDING)
        .args([
            "--delivery-id",
            delivery_id,
            "--requirement-sha256",
            requirement_sha256,
            "--trusted-webhook-attestation-public-key-hex",
            &public_key,
            "--host-admission-file",
        ])
        .arg(host_admission_file)
        .stdin(Stdio::null())
        .output()
        .expect("launch actual forgejo-mission-local binary");
    let stdout = String::from_utf8(output.stdout).expect("mission CLI stdout is UTF-8");
    let value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "mission CLI must return one JSON document (exit {:?}, stderr {:?}): {error}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code(), value)
}

fn assert_private_runtime(runtime: &Path) {
    let metadata = fs::symlink_metadata(runtime).expect("inspect mounted scratch runtime only");
    assert!(
        metadata.file_type().is_dir(),
        "mounted runtime is a directory"
    );
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.mode() & 0o777, 0o700);
}

fn assert_issue_request_counts(paths: &[String], expected: usize) {
    let prefix = format!("/api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}");
    assert_eq!(
        paths.iter().filter(|path| path.as_str() == prefix).count(),
        expected,
        "all issue reads must use the fixed fixture endpoint: {paths:?}"
    );
}

const MISSION_012_REQUIREMENTS: &[u8] =
    include_bytes!("fixtures/mission-012/requirements/MISSION-012-CLAMP-001.json");
const MISSION_012_POLICY_V1: &[u8] =
    include_bytes!("fixtures/mission-012/policy/MISSION-012-CLAMP-001-v1.json");
const MISSION_012_POLICY_V2: &[u8] =
    include_bytes!("fixtures/mission-012/policy/MISSION-012-CLAMP-001-v2.json");
const MISSION_012_POLICY_V3: &[u8] =
    include_bytes!("fixtures/mission-012/policy/MISSION-012-CLAMP-001-v3.json");
const MISSION_012_POLICY_V4: &[u8] =
    include_bytes!("fixtures/mission-012/policy/MISSION-012-CLAMP-001-v4.json");
const MISSION_012_ACTIVE_POLICY: &[u8] = include_bytes!("fixtures/mission-012/policy/active.json");
const MISSION_012_BROKEN_SOURCE: &[u8] = b"pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        minimum // BUG: values above the range should clamp to maximum.\n    } else {\n        value\n    }\n}\n";
const MISSION_012_FIXED_SOURCE: &str = "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        maximum\n    } else {\n        value\n    }\n}\n";
const MISSION_012_ACCEPTANCE: &[u8] = b"use mission012_clamp::clamp_to_range;\n\n#[test]\nfn values_outside_the_range_clamp_to_the_nearest_boundary() {\n    assert_eq!(clamp_to_range(-5, -2, 10), -2);\n    assert_eq!(clamp_to_range(15, -2, 10), 10);\n}\n\n#[test]\nfn values_inside_the_range_are_unchanged() {\n    assert_eq!(clamp_to_range(4, -2, 10), 4);\n}\n";
const MISSION_012_CARGO_TOML: &[u8] =
    b"[package]\nname = \"mission012-clamp\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const MISSION_012_CARGO_LOCK: &[u8] = b"# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"mission012-clamp\"\nversion = \"0.1.0\"\n";

struct Mission012ForgejoFixture {
    commit: String,
    paths: Arc<Mutex<Vec<String>>>,
    authorization: Arc<Mutex<Vec<Option<String>>>>,
    issue_content: Arc<Mutex<Option<Value>>>,
    requirements: Arc<Mutex<Vec<u8>>>,
    required_token: Option<String>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Mission012ForgejoFixture {
    fn start(commit: String) -> Self {
        Self::start_with_token(commit, None)
    }

    fn start_with_token(commit: String, required_token: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:3132")
            .expect("reserve the fixed MISSION-012 Forgejo endpoint in the private net namespace");
        listener
            .set_nonblocking(true)
            .expect("make the MISSION-012 fixture accept loop stoppable");
        let paths = Arc::new(Mutex::new(Vec::new()));
        let authorization = Arc::new(Mutex::new(Vec::new()));
        let issue_content = Arc::new(Mutex::new(None));
        let requirements = Arc::new(Mutex::new(MISSION_012_REQUIREMENTS.to_vec()));
        let worker_paths = Arc::clone(&paths);
        let worker_authorization = Arc::clone(&authorization);
        let worker_issue_content = Arc::clone(&issue_content);
        let worker_requirements = Arc::clone(&requirements);
        let worker_commit = commit.clone();
        let worker_required_token = required_token.clone();
        let (stop_sender, stop_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                match stop_receiver.try_recv() {
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                assert!(
                    Instant::now() < deadline,
                    "MISSION-012 Forgejo fixture exceeded its deadline"
                );
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .expect("set a bounded MISSION-012 fixture request timeout");
                        let request = read_request_headers(&mut stream);
                        let path = request_path(&request);
                        let supplied_authorization = request
                            .lines()
                            .find(|line| {
                                line.split_once(':').is_some_and(|(name, _)| {
                                    name.eq_ignore_ascii_case("authorization")
                                })
                            })
                            .and_then(|line| {
                                line.split_once(':')
                                    .map(|(_, value)| value.trim().to_owned())
                            });
                        worker_paths.lock().unwrap().push(path.clone());
                        worker_authorization
                            .lock()
                            .unwrap()
                            .push(supplied_authorization.clone());
                        let authorized = worker_required_token.as_ref().is_none_or(|token| {
                            let expected = format!("token {token}");
                            supplied_authorization.as_deref() == Some(expected.as_str())
                        });
                        let (status, content_type, body) = if authorized {
                            mission_012_fixture_response(
                                &path,
                                &worker_commit,
                                worker_issue_content.lock().unwrap().clone(),
                                worker_requirements.lock().unwrap().clone(),
                            )
                        } else {
                            (
                                "401 Unauthorized".to_owned(),
                                "application/json",
                                br#"{"message":"unauthorized"}"#.to_vec(),
                            )
                        };
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        stream
                            .write_all(response.as_bytes())
                            .expect("write MISSION-012 fixture response headers");
                        stream
                            .write_all(&body)
                            .expect("write MISSION-012 fixture response body");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept MISSION-012 fixture request: {error}"),
                }
            }
        });
        Self {
            commit,
            paths,
            authorization,
            issue_content,
            requirements,
            required_token,
            stop: Some(stop_sender),
            worker: Some(worker),
        }
    }

    fn paths(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }

    fn authorization(&self) -> Vec<Option<String>> {
        self.authorization.lock().unwrap().clone()
    }

    fn set_issue_content(&self, content: Value) {
        *self.issue_content.lock().unwrap() = Some(content);
    }

    fn set_requirements(&self, requirements: &[u8]) {
        *self.requirements.lock().unwrap() = requirements.to_vec();
    }

    fn required_token(&self) -> Option<&str> {
        self.required_token.as_deref()
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("MISSION-012 fixture worker exits cleanly");
        }
    }
}

impl Drop for Mission012ForgejoFixture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn mission_012_fixture_response(
    path: &str,
    commit: &str,
    issue_content: Option<Value>,
    requirements: Vec<u8>,
) -> (String, &'static str, Vec<u8>) {
    const PREFIX: &str = "/api/v1/repos/forge-admin/mission-012-clamp";
    const REQUIREMENT_PATH: &str = "/forge-admin/mission-012-clamp/raw/commit/";
    let json_response = |value: Value| {
        (
            "200 OK".to_owned(),
            "application/json",
            serde_json::to_vec(&value).expect("serialize MISSION-012 fixture JSON"),
        )
    };
    if path == format!("{PREFIX}/issues/1") {
        let mut issue = json!({
            "number":1,
            "state":"open",
            "updated_at":"2026-10-23T00:00:00Z",
            "html_url":"http://127.0.0.1:3132/forge-admin/mission-012-clamp/issues/1"
        });
        if let Some(content) = issue_content {
            issue["title"] = content["title"].clone();
            issue["body"] = content["body"].clone();
            issue["labels"] = content["labels"].clone();
            if let Some(milestone) = content.get("milestone") {
                issue["milestone"] = milestone.clone();
            }
        }
        json_response(issue)
    } else if path == format!("{PREFIX}/branches/main") {
        json_response(json!({
            "name":"main",
            "protected":true,
            "commit":{"id":commit}
        }))
    } else if path == "/api/v1/settings/api" {
        (
            "200 OK".to_owned(),
            "application/json",
            br#"{"default_git_trees_per_page":1000}"#.to_vec(),
        )
    } else if path == format!("{PREFIX}/git/trees/{commit}?recursive=true&page=1&per_page=1000") {
        json_response(json!({
            "page":1,
            "total_count":10,
            "truncated":false,
            "sha":commit,
            "tree":[
                {"path":"Cargo.lock","type":"blob","mode":"100644"},
                {"path":"Cargo.toml","type":"blob","mode":"100644"},
                {"path":"policy/MISSION-012-CLAMP-001-v1.json","type":"blob","mode":"100644"},
                {"path":"policy/MISSION-012-CLAMP-001-v2.json","type":"blob","mode":"100644"},
                {"path":"policy/MISSION-012-CLAMP-001-v3.json","type":"blob","mode":"100644"},
                {"path":"policy/MISSION-012-CLAMP-001-v4.json","type":"blob","mode":"100644"},
                {"path":"policy/active.json","type":"blob","mode":"100644"},
                {"path":"requirements/MISSION-012-CLAMP-001.json","type":"blob","mode":"100644"},
                {"path":"src/lib.rs","type":"blob","mode":"100644"},
                {"path":"tests/acceptance.rs","type":"blob","mode":"100644"}
            ]
        }))
    } else if path == format!("{REQUIREMENT_PATH}{commit}/requirements/MISSION-012-CLAMP-001.json")
    {
        ("200 OK".to_owned(), "application/json", requirements)
    } else if path == format!("{REQUIREMENT_PATH}{commit}/src/lib.rs") {
        (
            "200 OK".to_owned(),
            "text/plain; charset=utf-8",
            MISSION_012_BROKEN_SOURCE.to_vec(),
        )
    } else {
        (
            "404 Not Found".to_owned(),
            "application/json",
            serde_json::to_vec(&json!({"error":"unexpected fixture path","path":path}))
                .expect("serialize unexpected-route response"),
        )
    }
}

struct Mission012TempDirectory(PathBuf);

impl Mission012TempDirectory {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mission012-composed-e2e-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create private MISSION-012 process test directory");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("make MISSION-012 process test directory private");
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Mission012TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_mission_012_tree(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("create mission source directory");
    fs::create_dir_all(root.join("tests")).expect("create mission tests directory");
    fs::create_dir_all(root.join("requirements")).expect("create mission requirements directory");
    fs::create_dir_all(root.join("policy")).expect("create mission policy directory");
    fs::write(root.join("Cargo.toml"), MISSION_012_CARGO_TOML).expect("write mission manifest");
    fs::write(root.join("Cargo.lock"), MISSION_012_CARGO_LOCK).expect("write locked dependencies");
    fs::write(root.join("src/lib.rs"), MISSION_012_BROKEN_SOURCE).expect("write broken clamp");
    fs::write(root.join("tests/acceptance.rs"), MISSION_012_ACCEPTANCE)
        .expect("write protected clamp acceptance tests");
    fs::write(
        root.join("requirements/MISSION-012-CLAMP-001.json"),
        MISSION_012_REQUIREMENTS,
    )
    .expect("write exact versioned mission requirements");
    fs::write(
        root.join("policy/MISSION-012-CLAMP-001-v1.json"),
        MISSION_012_POLICY_V1,
    )
    .expect("write historical policy v1");
    fs::write(
        root.join("policy/MISSION-012-CLAMP-001-v2.json"),
        MISSION_012_POLICY_V2,
    )
    .expect("write historical policy v2");
    fs::write(
        root.join("policy/MISSION-012-CLAMP-001-v3.json"),
        MISSION_012_POLICY_V3,
    )
    .expect("write historical policy v3");
    fs::write(
        root.join("policy/MISSION-012-CLAMP-001-v4.json"),
        MISSION_012_POLICY_V4,
    )
    .expect("write pinned verification policy");
    fs::write(root.join("policy/active.json"), MISSION_012_ACTIVE_POLICY)
        .expect("write active policy pointer");
}

fn copy_mission_012_tree(source: &Path, destination: &Path) {
    for relative in [
        "Cargo.toml",
        "Cargo.lock",
        "src/lib.rs",
        "tests/acceptance.rs",
        "requirements/MISSION-012-CLAMP-001.json",
        "policy/MISSION-012-CLAMP-001-v1.json",
        "policy/MISSION-012-CLAMP-001-v2.json",
        "policy/MISSION-012-CLAMP-001-v3.json",
        "policy/MISSION-012-CLAMP-001-v4.json",
        "policy/active.json",
    ] {
        let source_file = source.join(relative);
        let destination_file = destination.join(relative);
        fs::create_dir_all(destination_file.parent().unwrap())
            .expect("create copied mission tree directory");
        fs::copy(source_file, destination_file).expect("copy mission tree file");
    }
}

fn mission_git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "MISSION-012 E2E")
        .env("GIT_AUTHOR_EMAIL", "mission012@example.invalid")
        .env("GIT_COMMITTER_NAME", "MISSION-012 E2E")
        .env("GIT_COMMITTER_EMAIL", "mission012@example.invalid")
        .arg("-C")
        .arg(repository)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run host Git for synthetic mission fixture");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git output is UTF-8")
        .trim()
        .to_owned()
}

fn run_mission_012_cli(
    delivery_id: &str,
    requirement_sha256: &str,
    admission_path: &Path,
) -> (ExitStatus, Value) {
    let public_key = hex::encode(
        SigningKey::from_bytes(&DEFAULT_ATTESTATION_SEED)
            .verifying_key()
            .as_bytes(),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-mission-local"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--profile",
            "mission-012-clamp",
            "--issue",
            "1",
            "--requirement",
            "MISSION-012-CLAMP-001",
            "--revision",
            "1",
            "--allocation-id",
            "mission012-e2e-allocation",
            "--workspace-id",
            "mission-012-clamp-issue-1",
            "--delivery-id",
            delivery_id,
            "--requirement-sha256",
            requirement_sha256,
            "--trusted-webhook-attestation-public-key-hex",
            &public_key,
            "--host-admission-file",
        ])
        .arg(admission_path)
        .stdin(Stdio::null())
        .output()
        .expect("launch actual mission admission CLI");
    let stdout = String::from_utf8(output.stdout).expect("mission CLI stdout is UTF-8");
    let value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "mission CLI must return one JSON response (exit {}, stderr {:?}): {error}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, value)
}

fn run_mission_012_direct_cli_with_credential(
    credential_path: Option<&Path>,
    admission_path: &Path,
) -> (ExitStatus, Value, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-mission-local"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--profile",
            "mission-012-clamp",
            "--issue",
            "1",
            "--requirement",
            "MISSION-012-CLAMP-001",
            "--revision",
            "1",
            "--allocation-id",
            "mission012-host-credential-check",
            "--workspace-id",
            "mission-012-clamp-issue-1",
            "--host-admission-file",
        ])
        .arg(admission_path);
    if let Some(path) = credential_path {
        command.arg("--host-credential-file").arg(path);
    }
    let output = command
        .stdin(Stdio::null())
        .output()
        .expect("launch actual MISSION-012 admission CLI with host credential file");
    let stdout = String::from_utf8(output.stdout).expect("mission CLI stdout is UTF-8");
    let value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "mission CLI must return one JSON response (exit {}, stderr {:?}): {error}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (
        output.status,
        value,
        String::from_utf8(output.stderr).expect("mission CLI stderr is UTF-8"),
    )
}

fn run_mission_012_host(config_path: &Path, requests: &str) -> (ExitStatus, Vec<Value>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mission-012-mcp-host"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .arg("--config")
        .arg(config_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch actual MISSION-012 MCP host process");
    child
        .stdin
        .take()
        .expect("MCP host stdin is piped")
        .write_all(requests.as_bytes())
        .expect("send synthetic stdio MCP calls");
    let output = child
        .wait_with_output()
        .expect("wait for actual MISSION-012 MCP host process");
    let stdout = String::from_utf8(output.stdout).expect("MCP host stdout is UTF-8");
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("MCP response is JSON"))
        .collect();
    (
        output.status,
        responses,
        String::from_utf8(output.stderr).expect("MCP host stderr is UTF-8"),
    )
}

fn write_private_json(path: &Path, value: &Value) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create private process E2E configuration");
    serde_json::to_writer(&mut file, value).expect("write private process E2E configuration");
    file.write_all(b"\n").expect("terminate private JSON file");
    file.sync_all().expect("sync private JSON file");
}

fn mission_rpc_line(value: Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string(&value).expect("serialize process-test MCP request")
    )
}

fn mission_response_for(responses: &[Value], id: u64) -> &Value {
    responses
        .iter()
        .find(|response| response.get("id") == Some(&json!(id)))
        .unwrap_or_else(|| panic!("MCP response {id} is missing: {responses:?}"))
}

fn mission_tool_text(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool call returns JSON text");
    serde_json::from_str(text).expect("tool result is valid JSON")
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_uses_private_host_credential_only_for_fixed_forgejo_reads() {
    const COMMIT: &str = "f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0";
    const TOKEN: &str = "synthetic-host-read-token-0123456789";

    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let scratch = Mission012TempDirectory::new();
    let credential_path = scratch.path().join("host-api-token");
    let mut credential = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&credential_path)
        .expect("create a private synthetic Forgejo read token");
    credential
        .write_all(format!("{TOKEN}\n").as_bytes())
        .expect("write synthetic read token");
    credential.sync_all().expect("sync synthetic read token");
    assert_eq!(
        fs::symlink_metadata(&credential_path).unwrap().mode() & 0o777,
        0o600
    );

    let mut forgejo =
        Mission012ForgejoFixture::start_with_token(COMMIT.to_owned(), Some(TOKEN.to_owned()));
    let admission_path = runtime.join("host-credential-admission.json");
    let (status, admitted, stderr) =
        run_mission_012_direct_cli_with_credential(Some(&credential_path), &admission_path);
    assert!(
        status.success(),
        "host-only credential admission: {admitted} {stderr}"
    );
    assert_eq!(admitted["verdict"], "claimed");
    assert_eq!(admitted["profile_id"], "mission-012-clamp");
    assert_eq!(admitted["base_commit"], COMMIT);
    assert_eq!(forgejo.required_token(), Some(TOKEN));
    let auth_value = format!("token {TOKEN}");
    assert!(!forgejo.authorization().is_empty());
    assert!(
        forgejo
            .authorization()
            .iter()
            .all(|header| header.as_deref() == Some(auth_value.as_str()))
    );
    assert!(forgejo.paths().iter().all(|path| {
        path.starts_with("/api/v1/repos/forge-admin/mission-012-clamp/")
            || path == "/api/v1/settings/api"
            || path.starts_with("/forge-admin/mission-012-clamp/raw/commit/")
    }));
    let admission_bytes = fs::read(&admission_path).expect("read persisted host handoff");
    let journal_path = runtime.join("mission-execution.jsonl");
    let journal_before_refusal = fs::read(&journal_path).expect("read durable admission journal");
    let serialized_response = serde_json::to_string(&admitted).unwrap();
    assert!(!serialized_response.contains(TOKEN));
    assert!(!String::from_utf8_lossy(&admission_bytes).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&journal_before_refusal).contains(TOKEN));
    assert!(!stderr.contains(TOKEN));

    let (anonymous_status, anonymous, anonymous_stderr) =
        run_mission_012_direct_cli_with_credential(None, &runtime.join("anonymous-admission.json"));
    assert_eq!(anonymous_status.code(), Some(1));
    assert_eq!(anonymous["verdict"], "inconclusive");
    assert_eq!(
        fs::read(&journal_path).unwrap(),
        journal_before_refusal,
        "unauthorized read must fail before allocation"
    );
    assert!(!serde_json::to_string(&anonymous).unwrap().contains(TOKEN));
    assert!(!anonymous_stderr.contains(TOKEN));
    assert!(forgejo.authorization().last().is_some_and(Option::is_none));
    forgejo.stop();
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_rejects_stale_context_before_allocation_then_claims_after_base_advance() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("synthetic-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let host_admission_file = runtime.join("mission-host-admission.json");
    write_synthetic_secret(&secret);

    let requirement_sha256 = hex::encode(Sha256::digest(REQUIREMENTS));
    let webhook_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "title": "Issue title from delivery",
            "body": "Issue body from delivery is untrusted and stale."
        }
    }))
    .expect("serialize synthetic issue webhook");

    let mut fixture = ForgejoFixture::start();
    fixture.set_issue_closed(false);
    fixture.advance_branch_after_context_source();
    let (mut receiver, receiver_address) = start_receiver(&secret, &inbox);
    let (accepted_status, accepted) =
        send_webhook(receiver_address, "mission-base-advance-41", &webhook_body);
    assert_eq!(accepted_status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["sequence"], 1);

    let (status, response) = mission_cli(
        "mission-base-advance-41",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(status, Some(1));
    assert_eq!(response["verdict"], "inconclusive");
    assert_eq!(response["error"]["code"], "context_stale");
    assert_eq!(response["error"]["details"]["context_commit"], COMMIT);
    assert_eq!(
        response["error"]["details"]["workspace_base"],
        ADVANCED_COMMIT
    );
    let rejected_events = EventJournal::open(&mission_journal).replay().unwrap();
    assert!(
        rejected_events.is_empty(),
        "stale context must be refused before persistent generation allocation"
    );
    assert!(
        !host_admission_file.exists(),
        "stale context must not create a host handoff"
    );
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 1);

    let (retry_status, retry) = mission_cli(
        "mission-base-advance-41",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(retry_status, Some(0), "recovered admission: {retry}");
    assert_eq!(retry["verdict"], "claimed");
    assert_eq!(retry["claim"], "claimed");
    assert_eq!(retry["allocation_id"], "admission-41");
    assert_eq!(retry["allocation_replayed"], false);
    assert_eq!(retry["execution_generation"], 1);
    assert_eq!(
        retry["execution_request"]["workspace"]["base_commit"],
        ADVANCED_COMMIT
    );
    assert_eq!(
        retry["execution_request"]["context"]["manifest"]["source_commit"],
        ADVANCED_COMMIT
    );
    let recovered_handoff: MissionHostAdmissionV1 = serde_json::from_slice(
        &fs::read(&host_admission_file).expect("recovered admission persists its handoff"),
    )
    .expect("recovered handoff is versioned JSON");
    assert_eq!(recovered_handoff.base_commit, ADVANCED_COMMIT);
    assert!(recovered_handoff.validate().is_ok());
    let claimed_events = EventJournal::open(&mission_journal).replay().unwrap();
    assert_eq!(claimed_events.len(), 2);
    assert_eq!(claimed_events[0].kind, "mission_generation_allocated_v1");
    assert_eq!(claimed_events[1].kind, "mission_execution_claim_v1");
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 1);

    let paths = fixture.paths();
    assert!(
        paths.iter().any(|path| {
            path == &format!("/{OWNER}/{REPOSITORY}/raw/commit/{COMMIT}/src/lib.rs")
        }),
        "context uses the previously read commit: {paths:?}"
    );
    assert!(paths.iter().any(|path| {
        path == &format!(
            "/api/v1/repos/{OWNER}/{REPOSITORY}/git/trees/{ADVANCED_COMMIT}?recursive=true&page=1&per_page=1000"
        )
    }), "admission inventories the advanced base: {paths:?}");
    assert!(
        paths.iter().any(|path| {
            path == &format!(
                "/{OWNER}/{REPOSITORY}/raw/commit/{ADVANCED_COMMIT}/requirements/REQ-001.json"
            )
        }),
        "admission reads the requirement at the advanced base: {paths:?}"
    );

    let receiver_status = receiver.stop();
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));
    fixture.stop();
    println!(
        "process-e2e: branch moved from {COMMIT} to {ADVANCED_COMMIT} after context source read; stale context refused before allocation or handoff; retry of the same delivery used fresh context and claimed generation 1"
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_refuses_issue_snapshot_change_during_admission_then_retries_current_state() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("synthetic-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let host_admission_file = runtime.join("mission-host-admission-issue-race.json");
    write_synthetic_secret(&secret);

    let requirement_sha256 = hex::encode(Sha256::digest(REQUIREMENTS));
    let webhook_body = serde_json::to_vec(&json!({
        "action":"opened",
        "repository":{"full_name":format!("{OWNER}/{REPOSITORY}")},
        "issue":{
            "number":ISSUE_NUMBER,
            "state":"open",
            "title":"Stale delivery title",
            "body":"Stale delivery body"
        }
    }))
    .expect("serialize issue delivery for the snapshot race");

    let mut fixture = ForgejoFixture::start();
    fixture.set_issue_closed(false);
    fixture.edit_issue_after_requirement_inventory();
    let (mut receiver, receiver_address) = start_receiver(&secret, &inbox);
    let (delivery_status, delivery) =
        send_webhook(receiver_address, "mission-issue-race-41", &webhook_body);
    assert_eq!(delivery_status, 202);
    assert_eq!(delivery["accepted"], true);
    assert_eq!(delivery["sequence"], 1);

    let (status, response) = mission_cli(
        "mission-issue-race-41",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(
        status,
        Some(1),
        "admission must refuse a current issue update during protected-path validation: {response}"
    );
    assert_eq!(response["verdict"], "inconclusive");
    assert_eq!(
        response["error"]["code"],
        "mission_issue_changed_during_admission"
    );
    assert_eq!(
        response["error"]["action"],
        "retry this delivery to bind a stable current issue snapshot before allocating the generation"
    );
    assert!(
        EventJournal::open(&mission_journal)
            .replay()
            .unwrap()
            .is_empty(),
        "a changed issue snapshot must be refused before generation allocation"
    );
    assert!(!host_admission_file.exists());

    let paths_before_retry = fixture.paths();
    let issue_path = format!("/api/v1/repos/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}");
    let tree_path = format!(
        "/api/v1/repos/{OWNER}/{REPOSITORY}/git/trees/{COMMIT}?recursive=true&page=1&per_page=1000"
    );
    assert!(paths_before_retry.contains(&tree_path));
    assert!(paths_before_retry.contains(&issue_path));

    let (retry_status, retry) = mission_cli(
        "mission-issue-race-41",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(retry_status, Some(0), "stable-snapshot retry: {retry}");
    assert_eq!(retry["verdict"], "claimed");
    assert_eq!(retry["execution_generation"], 1);
    assert_eq!(retry["allocation_replayed"], false);
    assert_eq!(
        retry["execution_request"]["untrusted_work_item"]["title"],
        "Edited title from Forgejo API"
    );
    assert_eq!(
        retry["execution_request"]["untrusted_work_item"]["labels"],
        json!([{"id":12,"name":"edited/current"}])
    );
    assert_eq!(
        EventJournal::open(&mission_journal).replay().unwrap().len(),
        2
    );
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 1);

    let receiver_status = receiver.stop();
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));
    fixture.stop();
    println!(
        "process-e2e: issue API changed during protected requirement inventory; first admission refused before generation allocation or host handoff; retry of same signed delivery bound the stable current snapshot and claimed generation 1"
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_refuses_branch_head_change_during_admission_then_retries_current_state() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("synthetic-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let host_admission_file = runtime.join("mission-host-admission-base-race.json");
    write_synthetic_secret(&secret);

    let requirement_sha256 = hex::encode(Sha256::digest(REQUIREMENTS));
    let webhook_body = serde_json::to_vec(&json!({
        "action":"opened",
        "repository":{"full_name":format!("{OWNER}/{REPOSITORY}")},
        "issue":{
            "number":ISSUE_NUMBER,
            "state":"open",
            "title":"Issue title from delivery",
            "body":"Issue body from delivery is untrusted and stale."
        }
    }))
    .expect("serialize synthetic issue webhook for the branch race");

    let mut fixture = ForgejoFixture::start();
    fixture.set_issue_closed(false);
    fixture.advance_branch_after_requirement_inventory();
    let (mut receiver, receiver_address) = start_receiver(&secret, &inbox);
    let (delivery_status, delivery) = send_webhook(
        receiver_address,
        "mission-base-inventory-race-41",
        &webhook_body,
    );
    assert_eq!(delivery_status, 202);
    assert_eq!(delivery["accepted"], true);
    assert_eq!(delivery["sequence"], 1);

    let (status, response) = mission_cli(
        "mission-base-inventory-race-41",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(
        status,
        Some(1),
        "admission must refuse a protected branch update during requirement inventory: {response}"
    );
    assert_eq!(response["verdict"], "inconclusive");
    assert_eq!(
        response["error"]["code"],
        "mission_base_changed_during_admission"
    );
    assert_eq!(
        response["error"]["action"],
        "retry this delivery to bind a stable protected base before allocating the generation"
    );
    assert!(
        EventJournal::open(&mission_journal)
            .replay()
            .unwrap()
            .is_empty()
    );
    assert!(!host_admission_file.exists());

    let paths_before_retry = fixture.paths();
    let old_tree_path = format!(
        "/api/v1/repos/{OWNER}/{REPOSITORY}/git/trees/{COMMIT}?recursive=true&page=1&per_page=1000"
    );
    let old_requirement_path =
        format!("/{OWNER}/{REPOSITORY}/raw/commit/{COMMIT}/requirements/REQ-001.json");
    assert!(paths_before_retry.contains(&old_tree_path));
    assert!(paths_before_retry.contains(&old_requirement_path));

    let (retry_status, retry) = mission_cli(
        "mission-base-inventory-race-41",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(retry_status, Some(0), "stable-base retry: {retry}");
    assert_eq!(retry["verdict"], "claimed");
    assert_eq!(retry["execution_generation"], 1);
    assert_eq!(retry["allocation_replayed"], false);
    assert_eq!(
        retry["execution_request"]["workspace"]["base_commit"],
        ADVANCED_COMMIT
    );
    assert_eq!(
        retry["execution_request"]["context"]["manifest"]["source_commit"],
        ADVANCED_COMMIT
    );
    let advanced_tree_path = format!(
        "/api/v1/repos/{OWNER}/{REPOSITORY}/git/trees/{ADVANCED_COMMIT}?recursive=true&page=1&per_page=1000"
    );
    let advanced_requirement_path =
        format!("/{OWNER}/{REPOSITORY}/raw/commit/{ADVANCED_COMMIT}/requirements/REQ-001.json");
    let all_paths = fixture.paths();
    assert!(all_paths.contains(&advanced_tree_path));
    assert!(all_paths.contains(&advanced_requirement_path));
    assert_eq!(
        EventJournal::open(&mission_journal).replay().unwrap().len(),
        2
    );
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 1);

    let receiver_status = receiver.stop();
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));
    fixture.stop();
    println!(
        "process-e2e: protected branch advanced after head read during requirement inventory; first admission refused before generation allocation or host handoff; retry of the same signed delivery used the current base and claimed generation 1"
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_concurrent_duplicate_deliveries_claim_one_generation_once() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("synthetic-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let mission_lock = PathBuf::from(format!("{}.lock", mission_journal.display()));
    let host_admission_file = runtime.join("mission-host-admission-concurrent.json");
    write_synthetic_secret(&secret);

    let requirement_sha256 = hex::encode(Sha256::digest(REQUIREMENTS));
    let webhook_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "title": "Concurrent issue delivery",
            "body": "Two distinct deliveries request the same protected issue generation."
        }
    }))
    .expect("serialize synthetic issue webhook");

    let mut fixture = ForgejoFixture::start();
    fixture.set_issue_closed(false);
    let (mut receiver, receiver_address) = start_receiver(&secret, &inbox);
    for (delivery_id, expected_sequence) in [
        ("mission-concurrent-41-a", 1),
        ("mission-concurrent-41-b", 2),
    ] {
        let (status, accepted) = send_webhook(receiver_address, delivery_id, &webhook_body);
        assert_eq!(status, 202);
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["sequence"], expected_sequence);
    }

    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&mission_lock)
        .expect("create private mission journal lock");
    // Hold the shared mission journal lock until both actual CLIs have read
    // their context. They then contend at allocation/claim instead of relying
    // on scheduler timing to overlap the durable state transition.
    let lock_status = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
    assert_eq!(lock_status, 0, "hold mission journal lock for the race");

    let start = Arc::new(Barrier::new(3));
    let first_start = Arc::clone(&start);
    let first_requirement = requirement_sha256.clone();
    let first_handoff = host_admission_file.clone();
    let first = thread::spawn(move || {
        first_start.wait();
        mission_cli(
            "mission-concurrent-41-a",
            &first_requirement,
            &first_handoff,
        )
    });
    let second_start = Arc::clone(&start);
    let second_requirement = requirement_sha256.clone();
    let second_handoff = host_admission_file.clone();
    let second = thread::spawn(move || {
        second_start.wait();
        mission_cli(
            "mission-concurrent-41-b",
            &second_requirement,
            &second_handoff,
        )
    });
    start.wait();

    let source_path = format!("/{OWNER}/{REPOSITORY}/raw/commit/{COMMIT}/src/lib.rs");
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut source_reads = 0;
    while Instant::now() < deadline {
        source_reads = fixture
            .paths()
            .iter()
            .filter(|path| path.as_str() == source_path)
            .count();
        if source_reads >= 2 {
            thread::sleep(Duration::from_millis(40));
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // Always release and reap both callers before asserting the barrier result.
    let unlock_status = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_UN) };
    drop(lock_file);
    let first_result = first.join();
    let second_result = second.join();
    let receiver_status = receiver.stop();
    let initial_fixture_paths = fixture.paths();

    assert_eq!(unlock_status, 0, "release mission journal lock");
    assert_eq!(
        source_reads, 2,
        "both CLIs must reach context before unlock: {initial_fixture_paths:?}"
    );
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));
    let (first_status, first_response) = first_result.expect("first admission process completes");
    let (second_status, second_response) =
        second_result.expect("second admission process completes");
    let attempts = [
        ("mission-concurrent-41-a", first_status, &first_response),
        ("mission-concurrent-41-b", second_status, &second_response),
    ];
    let mut responses = Vec::new();
    let mut retry_delivery_ids = Vec::new();
    for (delivery_id, status, response) in attempts {
        if response["error"]["code"] == "execution_mission_busy" {
            assert_eq!(
                status,
                Some(1),
                "busy admission refuses with code 1: {response}"
            );
            assert_eq!(response["error"]["retryable"], true);
            assert_eq!(
                response["error"]["action"],
                "retry after the active mission or journal transaction releases its lock"
            );
            retry_delivery_ids.push(delivery_id);
        } else {
            assert_eq!(
                status,
                Some(0),
                "non-contention admission succeeds: {response}"
            );
            responses.push(response.clone());
        }
    }
    for delivery_id in retry_delivery_ids.iter().copied() {
        let (retry_status, retry_response) =
            mission_cli(delivery_id, &requirement_sha256, &host_admission_file);
        assert_eq!(
            retry_status,
            Some(0),
            "retry after lock release: {retry_response}"
        );
        assert!(matches!(
            retry_response["verdict"].as_str(),
            Some("claimed" | "already_claimed")
        ));
        responses.push(retry_response);
    }
    let fixture_paths = fixture.paths();
    fixture.stop();
    assert_eq!(
        fixture_paths
            .iter()
            .filter(|path| path.as_str() == source_path)
            .count(),
        2 + retry_delivery_ids.len(),
        "each original process and any directed retry must read the current source: {fixture_paths:?}"
    );
    assert_eq!(
        responses.len(),
        2,
        "each delivery has one final admission result"
    );
    assert_eq!(
        responses
            .iter()
            .filter(|response| response["verdict"] == "claimed")
            .count(),
        1,
        "exactly one process owns the execution claim: {responses:?}"
    );
    assert_eq!(
        responses
            .iter()
            .filter(|response| response["verdict"] == "already_claimed")
            .count(),
        1,
        "the competing delivery observes the durable claim after its directed retry: {responses:?}"
    );
    assert_eq!(
        responses
            .iter()
            .filter(|response| response["allocation_replayed"] == false)
            .count(),
        1,
        "exactly one process reserves the generation: {responses:?}"
    );
    for response in &responses {
        assert_eq!(response["allocation_id"], "admission-41");
        assert_eq!(response["execution_generation"], 1);
        assert_eq!(response["requirement_sha256"], requirement_sha256);
        assert_eq!(response["host_admission"], responses[0]["host_admission"]);
    }

    let persisted_handoff: MissionHostAdmissionV1 = serde_json::from_slice(
        &fs::read(&host_admission_file).expect("the unique handoff is persisted"),
    )
    .expect("persisted handoff is versioned JSON");
    assert!(persisted_handoff.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&persisted_handoff).unwrap(),
        responses[0]["host_admission"]
    );
    let mission_events = EventJournal::open(&mission_journal).replay().unwrap();
    assert_eq!(mission_events.len(), 2, "one allocation and one claim");
    assert_eq!(mission_events[0].kind, "mission_generation_allocated_v1");
    assert_eq!(mission_events[1].kind, "mission_execution_claim_v1");
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 2);
    println!(
        "process-e2e: two signed deliveries and concurrent real admission CLIs read the same context; retryable contention refusals={}; one stable generation and one claim remain with matching handoff",
        retry_delivery_ids.len()
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("synthetic-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let host_admission_file = runtime.join("mission-host-admission.json");
    write_synthetic_secret(&secret);

    let requirement_sha256 = hex::encode(Sha256::digest(REQUIREMENTS));
    let webhook_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "title": "Stale title from the webhook",
            "body": "Stale body from the webhook; do not use this as current context.",
            "labels": [{"id": 99, "name": "stale/webhook-label"}]
        }
    }))
    .expect("serialize synthetic issue webhook");

    let mut fixture = ForgejoFixture::start();
    let (mut receiver, receiver_address) = start_receiver(&secret, &inbox);
    let (accepted_status, accepted) =
        send_webhook(receiver_address, "mission-process-41-a", &webhook_body);
    assert_eq!(accepted_status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);

    let (closed_status, closed) = mission_cli(
        "mission-process-41-a",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(closed_status, Some(1));
    assert_eq!(closed["verdict"], "inconclusive");
    assert_eq!(closed["error"]["code"], "mission_issue_closed");
    assert_eq!(
        closed["error"]["action"],
        "reopen the issue through its authorized local workflow before admission"
    );
    assert!(
        !mission_journal.exists(),
        "closed issue refusal creates no allocation journal"
    );
    assert!(
        !host_admission_file.exists(),
        "closed issue refusal creates no host handoff"
    );
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 1);

    fixture.set_issue_closed(false);
    let (admitted_status, admitted) = mission_cli(
        "mission-process-41-a",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(
        admitted_status,
        Some(0),
        "successful admission response: {admitted}"
    );
    assert_eq!(admitted["verdict"], "claimed");
    assert_eq!(admitted["claim"], "claimed");
    assert_eq!(admitted["admission_mode"], "webhook_delivery");
    assert_eq!(admitted["webhook"]["delivery_id"], "mission-process-41-a");
    assert_eq!(admitted["allocation_id"], "admission-41");
    assert_eq!(admitted["allocation_replayed"], false);
    assert_eq!(admitted["execution_generation"], 1);
    assert_eq!(admitted["requirement_sha256"], requirement_sha256);
    assert_eq!(admitted["execution_request"]["schema_version"], 4);
    assert_eq!(
        admitted["execution_request"]["untrusted_work_item"]["schema_version"],
        3
    );
    assert_eq!(
        admitted["execution_request"]["untrusted_work_item"]["title"],
        "Current title from Forgejo API"
    );
    assert_eq!(
        admitted["execution_request"]["untrusted_work_item"]["body"],
        json!({
            "kind": "text",
            "text": "Current body from Forgejo API; this is optional untrusted context."
        })
    );
    assert_eq!(
        admitted["execution_request"]["untrusted_work_item"]["source"],
        "current_forgejo_issue_api"
    );
    assert_eq!(
        admitted["execution_request"]["untrusted_work_item"]["trust"],
        "untrusted_user_content"
    );
    assert_eq!(
        admitted["execution_request"]["untrusted_work_item"]["labels"],
        json!([{"id": 7, "name": "area/triage"}])
    );
    assert_eq!(
        admitted["execution_request"]["untrusted_work_item"]["source_url"],
        "http://127.0.0.1:3130/api/v1/repos/forge-admin/agent-forge-core/issues/41"
    );
    assert_eq!(
        admitted["execution_request"]["requirements_json"],
        String::from_utf8(REQUIREMENTS.to_vec()).unwrap()
    );
    assert_eq!(
        admitted["execution_request"]["requirement_sha256"], requirement_sha256,
        "current and stale issue text cannot replace the protected requirements"
    );
    let host_admission: MissionHostAdmissionV1 =
        serde_json::from_value(admitted["host_admission"].clone())
            .expect("actual mission CLI emits the versioned host handoff");
    assert_eq!(host_admission.schema_version, 1);
    assert_eq!(host_admission.scope.origin, "http://127.0.0.1:3130");
    assert_eq!(host_admission.issue_number, ISSUE_NUMBER);
    assert!(host_admission.validate().is_ok());
    assert_eq!(
        serde_json::from_slice::<MissionHostAdmissionV1>(
            &fs::read(&host_admission_file).expect("CLI persists the host handoff")
        )
        .expect("persisted handoff is versioned JSON"),
        host_admission
    );
    assert_eq!(
        admitted["webhook"]["requirement_sha256_matches_cli_argument"],
        true
    );
    let allocation_id = admitted["allocation_id"].clone();
    let mission_id = admitted["mission_id"].clone();
    let idempotency_key = admitted["idempotency_key"].clone();
    let journal_after_admission = fs::read(&mission_journal).expect("read scratch mission journal");
    let admission_after_admission =
        fs::read(&host_admission_file).expect("read host admission file");
    let journal_metadata = fs::symlink_metadata(&mission_journal).unwrap();
    assert_eq!(journal_metadata.mode() & 0o777, 0o600);
    assert_issue_request_counts(&fixture.paths(), 4);

    let (duplicate_status, duplicate) =
        send_webhook(receiver_address, "mission-process-41-a", &webhook_body);
    assert_eq!(duplicate_status, 202);
    assert_eq!(duplicate["accepted"], true);
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(duplicate["sequence"], 1);
    let (retry_status, retry) = mission_cli(
        "mission-process-41-a",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(retry_status, Some(0), "duplicate retry response: {retry}");
    assert_eq!(retry["verdict"], "already_claimed");
    assert_eq!(retry["claim"], "already_claimed");
    assert_eq!(retry["allocation_replayed"], true);
    assert_eq!(retry["execution_generation"], 1);
    assert_eq!(retry["allocation_id"], allocation_id);
    assert_eq!(retry["mission_id"], mission_id);
    assert_eq!(retry["idempotency_key"], idempotency_key);
    assert_eq!(retry["host_admission"], admitted["host_admission"]);
    assert_eq!(fs::read(&mission_journal).unwrap(), journal_after_admission);

    fixture.set_issue_labels_changed(true);
    let label_only_webhook_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "title": "Stale title from the webhook",
            "body": "Stale body from the webhook; label-only API change follows."
        }
    }))
    .expect("serialize synthetic label-only issue webhook");
    let (label_delivery_status, label_delivery) = send_webhook(
        receiver_address,
        "mission-process-41-label-only",
        &label_only_webhook_body,
    );
    assert_eq!(label_delivery_status, 202);
    assert_eq!(label_delivery["accepted"], true);
    assert_eq!(label_delivery["duplicate"], false);
    assert_eq!(label_delivery["sequence"], 2);
    let (label_conflict_status, label_conflict) = mission_cli(
        "mission-process-41-label-only",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(
        label_conflict_status,
        Some(1),
        "label-only context conflict response: {label_conflict}"
    );
    assert_eq!(label_conflict["verdict"], "inconclusive");
    assert_eq!(
        label_conflict["error"]["code"],
        "execution_idempotency_conflict"
    );
    assert_eq!(
        label_conflict["error"]["action"],
        "keep the same request for this generation or use a new host-approved generation"
    );
    assert_eq!(fs::read(&mission_journal).unwrap(), journal_after_admission);
    assert_eq!(
        fs::read(&host_admission_file).unwrap(),
        admission_after_admission
    );

    fixture.set_issue_edited(true);
    let edited_webhook_body = serde_json::to_vec(&json!({
        "action": "edited",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "updated_at": "2026-10-23T18:00:00Z",
            "title": "Stale edited title from webhook",
            "body": "Stale edited body from webhook; the canonical API now differs."
        }
    }))
    .expect("serialize edited issue webhook");
    let (new_delivery_status, new_delivery) = send_webhook(
        receiver_address,
        "mission-process-41-b",
        &edited_webhook_body,
    );
    assert_eq!(new_delivery_status, 202);
    assert_eq!(new_delivery["accepted"], true);
    assert_eq!(new_delivery["duplicate"], false);
    assert_eq!(new_delivery["sequence"], 3);
    let (resumed_status, resumed) = mission_cli(
        "mission-process-41-b",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(
        resumed_status,
        Some(1),
        "edited-context conflict response: {resumed}"
    );
    assert_eq!(resumed["verdict"], "inconclusive");
    assert_eq!(resumed["error"]["code"], "execution_idempotency_conflict");
    assert_eq!(
        resumed["error"]["action"],
        "keep the same request for this generation or use a new host-approved generation"
    );
    assert_eq!(fs::read(&mission_journal).unwrap(), journal_after_admission);
    assert_eq!(
        fs::read(&host_admission_file).unwrap(),
        admission_after_admission
    );
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 3);

    let receiver_status = receiver.stop();
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));
    let fixture_paths = fixture.paths();
    fixture.stop();
    assert_issue_request_counts(&fixture_paths, 13);
    assert_eq!(
        fixture_paths.len(),
        51,
        "closed refusal, initial claim, identical retry, label-only conflict, and edited-context refusal: {fixture_paths:#?}"
    );
    for (suffix, count) in [
        (
            "/api/v1/repos/forge-admin/agent-forge-core/branches/main",
            13,
        ),
        ("/api/v1/settings/api", 4),
        (
            "/api/v1/repos/forge-admin/agent-forge-core/git/trees/0123456789abcdef0123456789abcdef01234567?recursive=true&page=1&per_page=1000",
            4,
        ),
        (
            "/forge-admin/agent-forge-core/raw/commit/0123456789abcdef0123456789abcdef01234567/requirements/REQ-001.json",
            13,
        ),
        (
            "/forge-admin/agent-forge-core/raw/commit/0123456789abcdef0123456789abcdef01234567/src/lib.rs",
            4,
        ),
    ] {
        assert_eq!(
            fixture_paths
                .iter()
                .filter(|path| path.as_str() == suffix)
                .count(),
            count,
            "successful source reads use expected fixture route {suffix}: {fixture_paths:?}"
        );
    }

    println!(
        "process-e2e: closed refusal=mission_issue_closed; initial admission=claimed; identical retry=already_claimed; label-only and edited issue context on new deliveries=execution_idempotency_conflict without journal or handoff mutation; allocation={} generation=1; inbox deliveries=3; fixture HTTP requests={}",
        allocation_id
            .as_str()
            .expect("admitted allocation ID is a string"),
        fixture_paths.len()
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_admits_signed_webhook_with_v3_requirement_arbitration() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("synthetic-v3-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let host_admission_file = runtime.join("mission-v3-host-admission.json");
    let arbitration_directory = runtime.join("mission-arbitration");
    fs::create_dir(&arbitration_directory).expect("create private arbitration policy directory");
    fs::set_permissions(&arbitration_directory, fs::Permissions::from_mode(0o700))
        .expect("make arbitration policy directory private");
    write_synthetic_secret(&secret);
    write_private_json(
        &arbitration_directory.join("duration-parser.json"),
        &json!({
            "schema_version": 1,
            "revision": 4,
            "scope": {
                "origin": "http://127.0.0.1:3130",
                "owner": OWNER,
                "repository": REPOSITORY,
                "branch": "main"
            },
            "required_approvals": 1,
            "authorized_reviewer_ids": [9001]
        }),
    );

    let requirement_sha256 = hex::encode(Sha256::digest(REQUIREMENTS_V3));
    let webhook_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {
            "number": ISSUE_NUMBER,
            "state": "open",
            "title": "Untrusted request title",
            "body": "Untrusted request body"
        }
    }))
    .expect("serialize synthetic schema-v3 issue webhook");

    let mut fixture = ForgejoFixture::start();
    fixture.set_issue_closed(false);
    fixture.set_requirements(REQUIREMENTS_V3);
    let (mut receiver, receiver_address) = start_receiver(&secret, &inbox);
    let (accepted_status, accepted) =
        send_webhook(receiver_address, "mission-v3-delivery-1", &webhook_body);
    assert_eq!(accepted_status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);

    let (admission_status, admission) = mission_cli(
        "mission-v3-delivery-1",
        &requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(
        admission_status,
        Some(0),
        "signed webhook v3 admission response: {admission}"
    );
    assert_eq!(admission["admission_mode"], "webhook_delivery");
    assert_eq!(admission["admission_schema_version"], 3);
    assert_eq!(admission["webhook"]["delivery_id"], "mission-v3-delivery-1");
    assert_eq!(
        admission["requirement_arbitration"]["source_pull_request_number"],
        77
    );
    assert_eq!(
        admission["requirement_arbitration"]["reviewer_ids"],
        json!([9001])
    );
    assert_eq!(
        admission["requirement_arbitration"]["requirements_sha256"],
        requirement_sha256
    );
    assert_eq!(admission["agent_runtime_invoked"], false);
    assert!(mission_journal.is_file());
    assert!(host_admission_file.is_file());
    assert!(
        fixture
            .paths()
            .iter()
            .any(|path| path == "/api/v1/repos/forge-admin/agent-forge-core/pulls/77"),
        "v3 webhook admission must read its source pull request"
    );
    assert!(
        fixture.paths().iter().any(|path| path
            == "/api/v1/repos/forge-admin/agent-forge-core/pulls/77/reviews?page=1&limit=50"),
        "v3 webhook admission must read its source reviews"
    );

    let exit_status = receiver.stop();
    assert!(exit_status.success());
    fixture.stop();
    println!(
        "process-e2e: signed webhook delivery admitted schema-v3 requirements bound to PR 77 and authorized reviewer 9001; local HTTP fixture only; agent_runtime_invoked=false"
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_mission_cli_refuses_same_revision_requirement_edit_after_signed_delivery() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("synthetic-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let host_admission_file = runtime.join("mission-host-admission.json");
    write_synthetic_secret(&secret);

    let old_requirements: Value = serde_json::from_slice(REQUIREMENTS).unwrap();
    let edited_requirements: Value =
        serde_json::from_slice(REQUIREMENTS_CHANGED_SAME_REVISION).unwrap();
    assert_eq!(
        old_requirements["revision"],
        edited_requirements["revision"]
    );
    let pinned_requirement_sha256 = hex::encode(Sha256::digest(REQUIREMENTS));
    let webhook_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open"}
    }))
    .expect("serialize synthetic issue webhook");

    let mut fixture = ForgejoFixture::start();
    let (mut receiver, receiver_address) = start_receiver(&secret, &inbox);
    let (accepted_status, accepted) = send_webhook(
        receiver_address,
        "mission-process-requirement-edit",
        &webhook_body,
    );
    assert_eq!(accepted_status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);

    // The signed delivery has been durably accepted; the protected bytes then change
    // without a revision bump before the admission process reads them.
    fixture.set_issue_closed(false);
    fixture.set_requirements(REQUIREMENTS_CHANGED_SAME_REVISION);
    let (status, rejected) = mission_cli(
        "mission-process-requirement-edit",
        &pinned_requirement_sha256,
        &host_admission_file,
    );
    assert_eq!(status, Some(1), "stale requirement response: {rejected}");
    assert_eq!(rejected["verdict"], "inconclusive");
    assert_eq!(
        rejected["error"]["code"],
        "mission_requirement_hash_mismatch"
    );
    assert_eq!(
        rejected["error"]["action"],
        "review the protected requirements change and update the host-pinned digest before admission"
    );
    assert!(
        !host_admission_file.exists(),
        "stale requirement refusal must not produce a host handoff"
    );
    let mission_events = if mission_journal.exists() {
        EventJournal::open(&mission_journal)
            .replay()
            .expect("replay the empty or unchanged mission journal")
            .len()
    } else {
        0
    };
    assert_eq!(
        mission_events, 0,
        "stale requirements must not allocate a mission"
    );
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 1);

    let receiver_status = receiver.stop();
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));
    let fixture_paths = fixture.paths();
    fixture.stop();
    assert_issue_request_counts(&fixture_paths, 3);
    let requirement_path =
        format!("/{OWNER}/{REPOSITORY}/raw/commit/{COMMIT}/requirements/REQ-001.json");
    assert!(
        fixture_paths.iter().any(|path| path == &requirement_path),
        "admission must read the selected requirement from the fixed protected commit"
    );
    assert!(
        fixture_paths.iter().all(|path| {
            path.starts_with(&format!("/api/v1/repos/{OWNER}/{REPOSITORY}/"))
                || path.starts_with(&format!("/{OWNER}/{REPOSITORY}/raw/commit/{COMMIT}/"))
                || path == "/api/v1/settings/api"
        }),
        "all reads stay on the fixed local fixture routes: {fixture_paths:?}"
    );

    println!(
        "process-e2e: signed delivery accepted; same-revision protected requirement edit refused as mission_requirement_hash_mismatch; allocation events=0; handoff=false"
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn mission_profile_rejects_caller_digest_before_inbox_credential_or_allocation() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let admission_file = runtime.join("mission-host-admission.json");
    let credential_file = runtime.join("missing-host-credential");
    assert!(!inbox.exists());
    assert!(!mission_journal.exists());
    assert!(!admission_file.exists());
    assert!(!credential_file.exists());

    let public_key = hex::encode(
        SigningKey::from_bytes(&DEFAULT_ATTESTATION_SEED)
            .verifying_key()
            .as_bytes(),
    );
    let wrong_digest = "0".repeat(64);
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-mission-local"))
        .env_clear()
        .args([
            "--profile",
            "mission-012-clamp",
            "--issue",
            "1",
            "--requirement",
            "MISSION-012-CLAMP-001",
            "--revision",
            "1",
            "--allocation-id",
            "mission012-allocation-1",
            "--workspace-id",
            "mission-012-clamp-issue-1",
            "--delivery-id",
            "mission012-delivery-1",
            "--requirement-sha256",
            &wrong_digest,
            "--trusted-webhook-attestation-public-key-hex",
            &public_key,
            "--host-credential-file",
        ])
        .arg(&credential_file)
        .arg("--host-admission-file")
        .arg(&admission_file)
        .stdin(Stdio::null())
        .output()
        .expect("launch actual forgejo-mission-local process");
    let response: Value = serde_json::from_slice(&output.stdout)
        .expect("mission CLI returns one JSON refusal document");
    assert_eq!(output.status.code(), Some(1), "response: {response}");
    assert_eq!(response["verdict"], "inconclusive");
    assert_eq!(
        response["error"]["code"],
        "mission_profile_requirement_mismatch"
    );
    assert!(
        !inbox.exists() && !mission_journal.exists() && !admission_file.exists(),
        "caller-selected criteria must be rejected before inbox lookup or allocation"
    );
    assert!(
        !credential_file.exists(),
        "the missing host credential must not be created or consumed"
    );
    println!(
        "process-e2e: caller-supplied MISSION-012 digest refused before inbox, credential, or allocation access"
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn mission_profile_refuses_unpinned_current_requirements_before_allocation() {
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const CHANGED_REQUIREMENTS: &[u8] = br#"{"schema_version":2,"id":"MISSION-012-CLAMP-001","revision":1,"source_issue_number":1,"request":"Change the clamp requirements without host review.","acceptance":["A different implementation is permitted."],"ambiguities":[]}"#;

    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let mission_journal = runtime.join("mission-execution.jsonl");
    let admission_file = runtime.join("mission-host-admission.json");
    assert!(!mission_journal.exists());
    assert!(!admission_file.exists());

    let mut fixture = Mission012ForgejoFixture::start(COMMIT.to_owned());
    fixture.set_requirements(CHANGED_REQUIREMENTS);
    let (status, response, stderr) =
        run_mission_012_direct_cli_with_credential(None, &admission_file);
    let paths = fixture.paths();
    assert_eq!(
        status.code(),
        Some(1),
        "response: {response}; stderr: {stderr}; paths: {paths:?}"
    );
    assert_eq!(response["verdict"], "inconclusive");
    assert_eq!(
        response["error"]["code"], "mission_profile_requirement_mismatch",
        "response: {response}; paths: {paths:?}"
    );
    assert!(
        !mission_journal.exists() && !admission_file.exists(),
        "unapproved requirement bytes must not allocate or hand off a mission"
    );
    assert_eq!(
        paths,
        vec![
            "/api/v1/repos/forge-admin/mission-012-clamp/branches/main".to_owned(),
            format!(
                "/forge-admin/mission-012-clamp/raw/commit/{COMMIT}/requirements/MISSION-012-CLAMP-001.json"
            ),
        ],
        "only the protected branch and exact requirement bytes may be read before mismatch refusal"
    );
    fixture.stop();
    println!(
        "process-e2e: changed protected requirements refused before context reads, allocation, or host handoff"
    );
}

#[test]
#[ignore = "run directly with Cargo; opens no sockets and executes candidates only in the verifier sandbox"]
fn verifier_cli_process_runs_pinned_red_green_without_caller_digests() {
    let temp = Mission012TempDirectory::new();
    let root = temp.path();
    let base = root.join("base");
    let candidate = root.join("candidate");
    let candidate_worktree = root.join("candidate-branch");
    let trusted_git = root.join("trusted-git");
    write_mission_012_tree(&base);
    fs::create_dir(&candidate).expect("create candidate snapshot directory");
    copy_mission_012_tree(&base, &candidate);
    fs::create_dir(&trusted_git).expect("create host-owned verifier Git repository");
    copy_mission_012_tree(&base, &trusted_git);
    mission_git(&trusted_git, &["init", "-q", "-b", "main"]);
    mission_git(&trusted_git, &["add", "-A"]);
    mission_git(
        &trusted_git,
        &["commit", "-q", "-m", "protected MISSION-012 base"],
    );
    let base_commit = mission_git(&trusted_git, &["rev-parse", "refs/heads/main"]);

    mission_git(
        &trusted_git,
        &[
            "worktree",
            "add",
            "--detach",
            candidate_worktree.to_str().unwrap(),
            base_commit.as_str(),
        ],
    );
    fs::write(
        candidate_worktree.join("src/lib.rs"),
        MISSION_012_FIXED_SOURCE,
    )
    .expect("apply the accepted source correction");
    mission_git(&candidate_worktree, &["add", "src/lib.rs"]);
    mission_git(
        &candidate_worktree,
        &["commit", "-q", "-m", "correct upper clamp boundary"],
    );
    let candidate_commit = mission_git(&candidate_worktree, &["rev-parse", "HEAD"]);
    fs::copy(
        candidate_worktree.join("src/lib.rs"),
        candidate.join("src/lib.rs"),
    )
    .expect("make the candidate snapshot match its Git commit");

    let target = root.join("independent-verifier-target");
    let verification = Command::new(env!("CARGO_BIN_EXE_mission-012-verify-local"))
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .args([
            "--verify-pair",
            base.to_str().unwrap(),
            candidate.to_str().unwrap(),
            target.to_str().unwrap(),
            trusted_git.to_str().unwrap(),
            base_commit.as_str(),
            candidate_commit.as_str(),
        ])
        .stdin(Stdio::null())
        .output()
        .expect("launch the independent verifier with only snapshots and commits");
    let stdout = String::from_utf8(verification.stdout).expect("verifier JSON is UTF-8");
    let proof: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("verifier output was not JSON: {error}; {stdout}"));
    assert!(verification.status.success(), "verifier report: {proof}");
    assert_eq!(proof["verdict"], "pass");
    assert_eq!(proof["source_commit"], base_commit);
    assert_eq!(proof["candidate_commit"], candidate_commit);
    assert_eq!(proof["changed_paths"], json!(["src/lib.rs"]));
    assert_eq!(proof["binding"]["git_tree_binding_verified"], true);
    assert_eq!(proof["binding"]["pinned_snapshots_executed"], true);
    assert_eq!(
        proof["binding"]["requirement_sha256"],
        hex::encode(Sha256::digest(MISSION_012_REQUIREMENTS))
    );
    assert_eq!(
        proof["binding"]["policy_sha256"],
        hex::encode(Sha256::digest(MISSION_012_POLICY_V4))
    );
    assert_eq!(
        proof["command"],
        json!(["/toolchain/bin/cargo", "test", "--offline", "--locked"])
    );
    assert_eq!(proof["base"]["expected_acceptance_failure"], true);
    assert_eq!(proof["candidate"]["acceptance_pass"], true);
    assert_eq!(proof["candidate"]["unchanged_during_verification"], true);
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn verifier_process_refuses_caller_supplied_requirement_and_command() {
    let temp = Mission012TempDirectory::new();
    let root = temp.path();
    let base = root.join("base");
    let candidate = root.join("candidate");
    let trusted_git = root.join("trusted-git");
    fs::create_dir(&base).expect("create empty protected-base path");
    fs::create_dir(&candidate).expect("create empty candidate path");
    fs::create_dir(&trusted_git).expect("create empty trusted-Git path");

    let base_commit = "0".repeat(40);
    let candidate_commit = "1".repeat(40);
    let invoke = |target: &Path, trailing: &[&str]| {
        let rust_bin_dir = std::env::var_os("MISSION_E2E_RUST_BIN_DIR")
            .expect("isolated runner supplies the configured rustc/cargo directory");
        let mut command = Command::new(env!("CARGO_BIN_EXE_mission-012-verify-local"));
        command
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", PathBuf::from(rust_bin_dir).display()),
            )
            .args(["--verify-pair"])
            .arg(&base)
            .arg(&candidate)
            .arg(target)
            .arg(&trusted_git)
            .arg(&base_commit)
            .arg(&candidate_commit)
            .args(trailing)
            .stdin(Stdio::null())
            .output()
            .expect("launch the verifier with an untrusted override attempt")
    };

    let digest_override_target = root.join("digest-override-target");
    let digest_override = invoke(&digest_override_target, &[&"a".repeat(64), &"b".repeat(64)]);
    assert_eq!(
        digest_override.status.code(),
        Some(64),
        "requirement and policy digests must not be caller-controlled: stdout={:?}, stderr={:?}",
        String::from_utf8_lossy(&digest_override.stdout),
        String::from_utf8_lossy(&digest_override.stderr)
    );
    assert!(
        !digest_override_target.exists(),
        "argument refusal must precede creation of verifier state"
    );

    let command_override_target = root.join("command-override-target");
    let command_override = invoke(
        &command_override_target,
        &["--verification-command", "/tmp/attacker-controlled-command"],
    );
    assert_eq!(
        command_override.status.code(),
        Some(64),
        "verification command must not be caller-controlled: stdout={:?}, stderr={:?}",
        String::from_utf8_lossy(&command_override.stdout),
        String::from_utf8_lossy(&command_override.stderr)
    );
    assert!(
        !command_override_target.exists(),
        "argument refusal must precede creation of verifier state"
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn mission_012_signed_request_context_red_green_independent_verification_and_sandbox_merge() {
    let masked_home = PathBuf::from(
        std::env::var_os("MISSION_E2E_MASKED_HOME")
            .expect("isolated runner identifies the home directory it masked"),
    );
    assert!(masked_home.is_absolute());
    for private_path in [
        ".ssh", ".aws", ".azure", ".config", ".local", ".gnupg", ".docker", ".kube", ".codex",
        ".agents", ".cargo", ".rustup",
    ] {
        assert!(
            !masked_home.join(private_path).exists(),
            "runner must not expose host home path {}",
            masked_home.join(private_path).display()
        );
    }
    assert!(Path::new("/toolchain/bin/cargo").is_file());

    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("mission-012-e2e-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let admission_path = runtime.join("mission-012-host-admission.json");
    let attestation_seed_path = runtime.join("mission-012-webhook-attestation.seed");
    let attestation_seed = DEFAULT_ATTESTATION_SEED;
    let attestation_signing_key = SigningKey::from_bytes(&attestation_seed);
    write_synthetic_secret(&secret);
    write_synthetic_attestation_seed(&attestation_seed_path, &attestation_seed);

    let temp = Mission012TempDirectory::new();
    let root = temp.path();
    let base = root.join("base");
    let candidate = root.join("candidate");
    let target = root.join("target");
    let trusted_git = root.join("trusted-git");
    let candidate_worktree = root.join("candidate-branch");
    write_mission_012_tree(&base);
    fs::create_dir(&candidate).expect("create candidate snapshot directory");
    copy_mission_012_tree(&base, &candidate);
    fs::create_dir(&trusted_git).expect("create trusted Git worktree");
    copy_mission_012_tree(&base, &trusted_git);
    fs::create_dir(&target).expect("create empty candidate Cargo target");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
        .expect("make candidate Cargo target private");
    mission_git(&trusted_git, &["init", "-q", "-b", "main"]);
    mission_git(&trusted_git, &["add", "-A"]);
    mission_git(
        &trusted_git,
        &["commit", "-q", "-m", "protected MISSION-012 base"],
    );
    let base_commit = mission_git(&trusted_git, &["rev-parse", "refs/heads/main"]);
    assert_eq!(base_commit.len(), 40);

    let requirement_sha256 = hex::encode(Sha256::digest(MISSION_012_REQUIREMENTS));
    let webhook_body = serde_json::to_vec(&json!({
        "action":"opened",
        "repository":{"full_name":"forge-admin/mission-012-clamp"},
        "issue":{"number":1,"state":"open"}
    }))
    .expect("serialize synthetic MISSION-012 issue webhook");
    let mut fixture = Mission012ForgejoFixture::start(base_commit.clone());
    let (mut receiver, receiver_address) =
        start_receiver_with_attestation(&secret, &inbox, &attestation_seed_path);
    let (accepted_status, accepted) =
        send_webhook(receiver_address, "mission012-process-e2e-1", &webhook_body);
    assert_eq!(accepted_status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);

    let (admission_status, admission) = run_mission_012_cli(
        "mission012-process-e2e-1",
        &requirement_sha256,
        &admission_path,
    );
    assert!(
        admission_status.success(),
        "admission response: {admission}"
    );
    assert_eq!(admission["verdict"], "claimed");
    assert_eq!(admission["admission_mode"], "webhook_delivery");
    assert_eq!(admission["profile_id"], "mission-012-clamp");
    assert_eq!(admission["base_commit"], base_commit);
    assert_eq!(admission["requirement_sha256"], requirement_sha256);
    assert_eq!(admission["host_admission"]["base_commit"], base_commit);
    assert_eq!(
        admission["host_admission"]["requirement_path"],
        "requirements/MISSION-012-CLAMP-001.json"
    );
    let admission_source_file = PathBuf::from(
        admission["host_admission_source_file"]
            .as_str()
            .expect("webhook admission must persist its per-delivery source receipt"),
    );
    let admission_source: Value = serde_json::from_slice(
        &fs::read(&admission_source_file).expect("read the host admission source receipt"),
    )
    .expect("parse the versioned host admission source receipt");
    assert_eq!(admission_source["schema_version"], 1);
    assert_eq!(admission_source["delivery_id"], "mission012-process-e2e-1");
    assert_eq!(admission_source["journal_sequence"], 1);
    assert_eq!(
        admission["execution_request"]["context"]["manifest"]["source_commit"],
        base_commit
    );
    assert_eq!(
        admission["execution_request"]["context"]["manifest"]["sources"][1]["path"],
        "src/lib.rs"
    );
    assert_eq!(
        admission["execution_request"]["context"]["manifest"]["sources"][1]["file_sha256"],
        hex::encode(Sha256::digest(MISSION_012_BROKEN_SOURCE))
    );
    assert!(admission_path.exists());
    assert!(mission_journal.exists());

    let (duplicate_status, duplicate) =
        send_webhook(receiver_address, "mission012-process-e2e-1", &webhook_body);
    assert_eq!(duplicate_status, 202);
    assert_eq!(duplicate["duplicate"], true);
    let (replay_status, replay) = run_mission_012_cli(
        "mission012-process-e2e-1",
        &requirement_sha256,
        &admission_path,
    );
    assert!(replay_status.success(), "admission replay: {replay}");
    assert_eq!(replay["verdict"], "already_claimed");
    assert_eq!(replay["allocation_replayed"], true);
    assert_eq!(replay["host_admission"], admission["host_admission"]);
    assert_eq!(EventJournal::open(&inbox).replay().unwrap().len(), 1);

    assert_eq!(fixture.commit, base_commit);
    let fixture_paths = fixture.paths();
    assert!(
        fixture_paths
            .iter()
            .any(|path| path == "/api/v1/repos/forge-admin/mission-012-clamp/branches/main")
    );
    assert!(fixture_paths.contains(&format!(
        "/forge-admin/mission-012-clamp/raw/commit/{base_commit}/requirements/MISSION-012-CLAMP-001.json"
    )));
    assert!(fixture_paths.contains(&format!(
        "/forge-admin/mission-012-clamp/raw/commit/{base_commit}/src/lib.rs"
    )));
    fixture.stop();
    let receiver_status = receiver.stop();
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));

    let sysroot = PathBuf::from(
        std::env::var_os("MISSION_E2E_RUST_SYSROOT")
            .expect("isolated runner provides the resolved Rust sysroot"),
    );
    assert!(sysroot.is_absolute() && sysroot.is_dir());
    let config_path = root.join("mission-012-mcp-host-config.json");
    let receipt_path = root.join("mission-012-receipts.jsonl");
    let host_config = json!({
        "schema_version":5,
        "admission_source_mode":"webhook_delivery",
        "trusted_webhook_attestation_public_keys_hex":[hex::encode(attestation_signing_key.verifying_key().as_bytes())],
        "webhook_inbox_file":inbox,
        "admission_source_file":admission_source_file,
        "workspace":{
            "base_dir":base,
            "candidate_dir":candidate,
            "target_dir":target,
            "rust_sysroot":sysroot,
            "trusted_git_dir":trusted_git,
        },
        "receipt_file":receipt_path,
        "journal_file":mission_journal,
        "admission_file":admission_path,
        "lease_id":"mission012-process-e2e-lease-1",
        "holder_id":"mission012-process-e2e-host",
        "lease_ttl_ms":300_000,
    });
    write_private_json(&config_path, &host_config);

    // The host must reject a missing source receipt before it serves any tools.
    let missing_source_path = admission_source_file
        .parent()
        .expect("source receipt has a parent directory")
        .join("mission-012-mcp-source-receipt-saved.json");
    fs::rename(&admission_source_file, &missing_source_path)
        .expect("temporarily hide the source receipt for the refusal case");
    let (missing_source_status, _, missing_source_stderr) = run_mission_012_host(&config_path, "");
    assert_eq!(missing_source_status.code(), Some(65));
    assert!(missing_source_stderr.contains("host_admission_source_unavailable"));
    fs::rename(&missing_source_path, &admission_source_file)
        .expect("restore the source receipt after the refusal case");

    // A syntactically valid receipt that points at a different event is also
    // rejected by comparing it with the private inbox journal.
    let source_bytes = fs::read(&admission_source_file).expect("save source receipt bytes");
    let mut corrupted_source: Value =
        serde_json::from_slice(&source_bytes).expect("source receipt is JSON");
    corrupted_source["journal_event_sha256"] = json!("0".repeat(64));
    fs::write(
        &admission_source_file,
        serde_json::to_vec(&corrupted_source).expect("serialize tampered source receipt"),
    )
    .expect("write tampered source receipt");
    let (mismatched_source_status, _, mismatched_source_stderr) =
        run_mission_012_host(&config_path, "");
    assert_eq!(mismatched_source_status.code(), Some(65));
    assert!(mismatched_source_stderr.contains("host_admission_source_mismatch"));
    fs::write(&admission_source_file, source_bytes).expect("restore original source receipt");

    let requests = [
        mission_rpc_line(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","method":"notifications/initialized"})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mission_context","arguments":{}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"workspace_read_text","arguments":{"path":"src/lib.rs"}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"cargo_test","arguments":{}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"workspace_write_text","arguments":{"path":"tests/acceptance.rs","content":"agent must not rewrite protected acceptance"}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"forgejo_status_set","arguments":{"status":"success"}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"workspace_write_text","arguments":{"path":"src/lib.rs","content":MISSION_012_FIXED_SOURCE}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"cargo_test","arguments":{}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":10,"method":"shutdown","params":{}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","method":"notifications/exit"})),
    ]
    .concat();
    let (host_status, responses, host_stderr) = run_mission_012_host(&config_path, &requests);
    assert!(
        host_status.success(),
        "MCP host exited as {host_status:?}; stderr: {host_stderr}; responses: {responses:?}"
    );
    let tools = &mission_response_for(&responses, 2)["result"]["tools"];
    let tool_names: Vec<&str> = tools
        .as_array()
        .expect("MCP tools is an array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool has a name"))
        .collect();
    assert_eq!(
        tool_names,
        [
            "mission_context",
            "workspace_read_text",
            "workspace_write_text",
            "cargo_test"
        ]
    );
    let context = mission_tool_text(mission_response_for(&responses, 3));
    assert_eq!(context["source_commit"], base_commit);
    assert_eq!(context["requirement_sha256"], requirement_sha256);
    assert_eq!(
        context["requirements_json"],
        String::from_utf8(MISSION_012_REQUIREMENTS.to_vec()).unwrap()
    );
    assert_eq!(
        context["context"]["manifest"]["sources"][1]["file_sha256"],
        hex::encode(Sha256::digest(MISSION_012_BROKEN_SOURCE))
    );
    assert_eq!(
        mission_tool_text(mission_response_for(&responses, 4))["content"],
        String::from_utf8(MISSION_012_BROKEN_SOURCE.to_vec()).unwrap()
    );
    assert_eq!(
        mission_tool_text(mission_response_for(&responses, 5))["expected_baseline_failure"],
        true
    );
    assert_eq!(
        mission_tool_text(mission_response_for(&responses, 6))["code"],
        "tool_path_denied"
    );
    assert_eq!(
        mission_tool_text(mission_response_for(&responses, 7))["code"],
        "tool_capability_denied"
    );
    assert_eq!(
        mission_response_for(&responses, 8)["result"]["isError"],
        false
    );
    assert_eq!(
        mission_tool_text(mission_response_for(&responses, 9))["verified_acceptance_pass"],
        true
    );
    assert!(mission_response_for(&responses, 10)["result"].is_object());

    let receipt_text = fs::read_to_string(&receipt_path).expect("MCP host writes bound receipts");
    let receipts: Vec<Value> = receipt_text
        .lines()
        .map(|line| serde_json::from_str(line).expect("receipt line is JSON"))
        .collect();
    let finalization = receipts
        .iter()
        .find(|record| record["event"] == "host_finalization")
        .expect("host records independent candidate finalization");
    assert_eq!(finalization["outcome"], "candidate_ready");
    assert_eq!(
        finalization["result"]["changed_paths"],
        json!(["src/lib.rs"])
    );
    assert_eq!(
        finalization["execution_binding"]["base_commit"],
        base_commit
    );
    assert_eq!(
        finalization["execution_binding"]["requirement_sha256"],
        requirement_sha256
    );
    assert_eq!(
        finalization["execution_binding"]["mission_id"],
        admission["mission_id"]
    );
    let denials = receipts
        .iter()
        .filter(|record| record["event"] == "tool_call" && record["success"] == false)
        .count();
    assert_eq!(
        denials, 2,
        "forbidden writes and self-published status are receipted"
    );

    // Create a real candidate Git commit without moving the protected main ref.
    mission_git(
        &trusted_git,
        &[
            "worktree",
            "add",
            "--detach",
            candidate_worktree.to_str().unwrap(),
            base_commit.as_str(),
        ],
    );
    fs::copy(
        candidate.join("src/lib.rs"),
        candidate_worktree.join("src/lib.rs"),
    )
    .expect("copy verified candidate change into its Git branch");
    mission_git(&candidate_worktree, &["add", "src/lib.rs"]);
    mission_git(
        &candidate_worktree,
        &["commit", "-q", "-m", "fix upper clamp boundary"],
    );
    let candidate_commit = mission_git(&candidate_worktree, &["rev-parse", "HEAD"]);
    assert_eq!(
        mission_git(&trusted_git, &["rev-parse", "refs/heads/main"]),
        base_commit,
        "the independent verifier runs while protected main is still at its admitted base"
    );

    let verifier_target = root.join("independent-verifier-target");
    let rust_bin_dir = std::env::var_os("MISSION_E2E_RUST_BIN_DIR")
        .expect("isolated runner provides the configured rustc/cargo directory");
    let verification = Command::new(env!("CARGO_BIN_EXE_mission-012-verify-local"))
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", PathBuf::from(rust_bin_dir).display()),
        )
        .args([
            "--verify-pair",
            base.to_str().unwrap(),
            candidate.to_str().unwrap(),
            verifier_target.to_str().unwrap(),
            trusted_git.to_str().unwrap(),
            base_commit.as_str(),
            candidate_commit.as_str(),
        ])
        .stdin(Stdio::null())
        .output()
        .expect("launch independent MISSION-012 verifier process");
    let verification_stdout =
        String::from_utf8(verification.stdout).expect("independent verifier stdout is UTF-8");
    let proof: Value = serde_json::from_str(verification_stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "verifier emits one JSON decision (exit {}, stderr {:?}): {error}",
            verification.status,
            String::from_utf8_lossy(&verification.stderr)
        )
    });
    assert!(
        verification.status.success(),
        "independent verifier report: {proof}"
    );
    assert_eq!(proof["verdict"], "pass");
    assert_eq!(proof["source_commit"], base_commit);
    assert_eq!(proof["candidate_commit"], candidate_commit);
    assert_eq!(proof["changed_paths"], json!(["src/lib.rs"]));
    assert_eq!(proof["binding"]["git_tree_binding_verified"], true);
    assert_eq!(proof["binding"]["pinned_snapshots_executed"], true);
    assert_eq!(proof["binding"]["requirement_sha256"], requirement_sha256);
    assert_eq!(
        proof["command"],
        json!(["/toolchain/bin/cargo", "test", "--offline", "--locked"])
    );

    // The sandbox decision checks the base again after verification, then advances
    // only its private main ref to the independently verified candidate tree.
    assert_eq!(
        mission_git(&trusted_git, &["rev-parse", "refs/heads/main"]),
        base_commit,
        "protected main remained unchanged while the verifier ran"
    );
    mission_git(
        &trusted_git,
        &["merge", "--ff-only", "--no-edit", candidate_commit.as_str()],
    );
    let merged_commit = mission_git(&trusted_git, &["rev-parse", "refs/heads/main"]);
    let candidate_tree = mission_git(
        &trusted_git,
        &["rev-parse", &format!("{candidate_commit}^{{tree}}")],
    );
    let merged_tree = mission_git(
        &trusted_git,
        &["rev-parse", &format!("{merged_commit}^{{tree}}")],
    );
    assert_eq!(merged_tree, candidate_tree);

    println!(
        "mission-012-process-e2e: signed issue delivery admitted and replayed idempotently; context tied to {base_commit}; red acceptance failure followed by green fix; forbidden test rewrite and self-status denied; independent verifier=pass for {candidate_commit}; local sandbox main advanced to {merged_commit} with matching tree. Forgejo API, PR creation and LLM were not exercised in this test."
    );
}

#[test]
#[ignore = "run only through the bwrap network-namespace runner with a scratch .runtime mount"]
fn actual_cli_current_issue_labels_reach_mcp_context_from_signed_webhook() {
    let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".runtime");
    assert_private_runtime(&runtime);
    let secret = runtime.join("mission-012-context-e2e-webhook.secret");
    let inbox = runtime.join("forgejo-webhook-inbox.jsonl");
    let mission_journal = runtime.join("mission-execution.jsonl");
    let admission_path = runtime.join("mission-012-context-e2e-admission.json");
    let attestation_seed_path = runtime.join("mission-012-context-e2e-attestation.seed");
    write_synthetic_secret(&secret);
    write_synthetic_attestation_seed(&attestation_seed_path, &DEFAULT_ATTESTATION_SEED);

    let temp = Mission012TempDirectory::new();
    let root = temp.path();
    let base = root.join("base");
    let candidate = root.join("candidate");
    let target = root.join("target");
    let trusted_git = root.join("trusted-git");
    write_mission_012_tree(&base);
    fs::create_dir(&candidate).expect("create candidate snapshot directory");
    copy_mission_012_tree(&base, &candidate);
    fs::create_dir(&trusted_git).expect("create trusted Git worktree");
    copy_mission_012_tree(&base, &trusted_git);
    fs::create_dir(&target).expect("create fresh Cargo target");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
        .expect("make fresh Cargo target private");
    mission_git(&trusted_git, &["init", "-q", "-b", "main"]);
    mission_git(&trusted_git, &["add", "-A"]);
    mission_git(
        &trusted_git,
        &["commit", "-q", "-m", "protected MISSION-012 context base"],
    );
    let base_commit = mission_git(&trusted_git, &["rev-parse", "refs/heads/main"]);
    let requirement_sha256 = hex::encode(Sha256::digest(MISSION_012_REQUIREMENTS));

    let webhook_body = serde_json::to_vec(&json!({
        "action":"opened",
        "repository":{"full_name":"forge-admin/mission-012-clamp"},
        "issue":{
            "number":1,
            "state":"open",
            "title":"Stale webhook title",
            "body":"Stale webhook body",
            "labels":[{"id":99,"name":"stale/webhook-label"}],
            "milestone":{"id":99,"title":"Stale webhook milestone","state":"closed"}
        }
    }))
    .expect("serialize signed issue fixture with stale labels");
    let mut fixture = Mission012ForgejoFixture::start(base_commit.clone());
    fixture.set_issue_content(json!({
        "title":"Current title from Forgejo API",
        "body":"Current body from Forgejo API",
        "labels":[{"id":7,"name":"current/api-label"}],
        "milestone":{"id":12,"title":"Current API milestone","state":"open"}
    }));
    let (mut receiver, receiver_address) =
        start_receiver_with_attestation(&secret, &inbox, &attestation_seed_path);
    let (delivery_status, delivery) =
        send_webhook(receiver_address, "mission012-context-e2e-1", &webhook_body);
    assert_eq!(delivery_status, 202);
    assert_eq!(delivery["accepted"], true);
    assert_eq!(delivery["sequence"], 1);

    let (admission_status, admission) = run_mission_012_cli(
        "mission012-context-e2e-1",
        &requirement_sha256,
        &admission_path,
    );
    assert!(
        admission_status.success(),
        "admission response: {admission}"
    );
    assert_eq!(admission["verdict"], "claimed");
    let admitted_work_item = admission["execution_request"]["untrusted_work_item"].clone();
    assert_eq!(admission["execution_request"]["schema_version"], 4);
    assert_eq!(admitted_work_item["schema_version"], 3);
    assert_eq!(
        admitted_work_item["title"],
        "Current title from Forgejo API"
    );
    assert_eq!(
        admitted_work_item["body"],
        json!({"kind":"text","text":"Current body from Forgejo API"})
    );
    assert_eq!(
        admitted_work_item["labels"],
        json!([{"id":7,"name":"current/api-label"}])
    );
    assert_eq!(
        admitted_work_item["milestone"],
        json!({
            "observation":"assigned",
            "value":{"id":12,"title":"Current API milestone","state":"open"}
        })
    );
    assert_ne!(
        admitted_work_item["labels"],
        serde_json::from_slice::<Value>(&webhook_body).unwrap()["issue"]["labels"]
    );
    assert_ne!(
        admitted_work_item["milestone"],
        serde_json::from_slice::<Value>(&webhook_body).unwrap()["issue"]["milestone"]
    );
    assert_eq!(
        admission["host_admission"]["request"]["requirements_json"],
        String::from_utf8(MISSION_012_REQUIREMENTS.to_vec()).unwrap()
    );
    let admission_source_file = PathBuf::from(
        admission["host_admission_source_file"]
            .as_str()
            .expect("admission persists its exact webhook source receipt"),
    );

    // A milestone-only change in the current Forgejo issue snapshot must
    // change the request digest and refuse reuse of the already-claimed
    // generation, even when title, body, labels, requirements, and base stay
    // byte-for-byte the same.
    let initial_admission_bytes =
        fs::read(&admission_path).expect("capture the first host handoff");
    fixture.set_issue_content(json!({
        "title":"Current title from Forgejo API",
        "body":"Current body from Forgejo API",
        "labels":[{"id":7,"name":"current/api-label"}],
        "milestone":{"id":13,"title":"Next API milestone","state":"open"}
    }));
    let milestone_change_body = serde_json::to_vec(&json!({
        "action":"edited",
        "repository":{"full_name":"forge-admin/mission-012-clamp"},
        "issue":{
            "number":1,
            "state":"open",
            "title":"Stale webhook title",
            "body":"Stale webhook body",
            "labels":[{"id":7,"name":"current/api-label"}],
            "milestone":{"id":12,"title":"Current API milestone","state":"open"}
        }
    }))
    .expect("serialize the second signed delivery with an old milestone snapshot");
    let (milestone_delivery_status, milestone_delivery) = send_webhook(
        receiver_address,
        "mission012-context-e2e-milestone-change",
        &milestone_change_body,
    );
    assert_eq!(milestone_delivery_status, 202);
    assert_eq!(milestone_delivery["accepted"], true);
    assert_eq!(milestone_delivery["sequence"], 2);
    let (milestone_conflict_status, milestone_conflict) = run_mission_012_cli(
        "mission012-context-e2e-milestone-change",
        &requirement_sha256,
        &admission_path,
    );
    assert_eq!(
        milestone_conflict_status.code(),
        Some(1),
        "milestone-only context conflict: {milestone_conflict}"
    );
    assert_eq!(milestone_conflict["verdict"], "inconclusive");
    assert_eq!(
        milestone_conflict["error"]["code"],
        "execution_idempotency_conflict"
    );
    assert_eq!(
        milestone_conflict["error"]["action"],
        "keep the same request for this generation or use a new host-approved generation"
    );
    assert_eq!(
        fs::read(&admission_path).unwrap(),
        initial_admission_bytes,
        "the changed work item cannot overwrite the first host handoff"
    );

    fixture.stop();
    let receiver_status = receiver.stop();
    assert!(receiver_status.success() || receiver_status.signal() == Some(libc::SIGTERM));

    let sysroot = PathBuf::from(
        std::env::var_os("MISSION_E2E_RUST_SYSROOT")
            .expect("isolated runner provides the resolved Rust sysroot"),
    );
    let config_path = root.join("mission-012-context-mcp-config.json");
    let receipt_path = root.join("mission-012-context-mcp-receipts.jsonl");
    let host_config = json!({
        "schema_version":5,
        "admission_source_mode":"webhook_delivery",
        "trusted_webhook_attestation_public_keys_hex":[hex::encode(SigningKey::from_bytes(&DEFAULT_ATTESTATION_SEED).verifying_key().as_bytes())],
        "webhook_inbox_file":inbox,
        "admission_source_file":admission_source_file,
        "workspace":{
            "base_dir":base,
            "candidate_dir":candidate,
            "target_dir":target,
            "rust_sysroot":sysroot,
            "trusted_git_dir":trusted_git,
        },
        "receipt_file":receipt_path,
        "journal_file":mission_journal,
        "admission_file":admission_path,
        "lease_id":"mission012-context-e2e-lease-1",
        "holder_id":"mission012-context-e2e-host",
        "lease_ttl_ms":300_000,
    });
    write_private_json(&config_path, &host_config);

    let requests = [
        mission_rpc_line(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","method":"notifications/initialized"})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mission_context","arguments":{}}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","id":4,"method":"shutdown","params":{}})),
        mission_rpc_line(json!({"jsonrpc":"2.0","method":"notifications/exit"})),
    ]
    .concat();
    let (host_status, responses, host_stderr) = run_mission_012_host(&config_path, &requests);
    assert!(
        responses
            .iter()
            .any(|response| response.get("id") == Some(&json!(3))),
        "MCP host did not answer mission_context: status {host_status:?}, stderr {host_stderr:?}, responses {responses:?}"
    );
    let context = mission_tool_text(mission_response_for(&responses, 3));
    assert_eq!(context["untrusted_work_item"], admitted_work_item);
    assert_eq!(
        context["requirements_json"],
        admission["host_admission"]["request"]["requirements_json"]
    );
    assert_eq!(
        context["untrusted_work_item"]["source"],
        "current_forgejo_issue_api"
    );
    assert_eq!(
        context["untrusted_work_item"]["trust"],
        "untrusted_user_content"
    );
    assert!(
        context["trust_boundary"]
            .as_str()
            .is_some_and(|boundary| boundary.contains("milestone")),
        "the MCP response describes milestone metadata as untrusted issue context"
    );
    assert_eq!(
        host_status.code(),
        Some(2),
        "context alone must not finalize a candidate: {host_stderr}"
    );
    let receipts: Vec<Value> = fs::read_to_string(&receipt_path)
        .expect("host records finalization outcome")
        .lines()
        .map(|line| serde_json::from_str(line).expect("receipt line is JSON"))
        .collect();
    let finalization = receipts
        .iter()
        .find(|record| record["event"] == "host_finalization")
        .expect("host records finalization");
    assert_eq!(finalization["outcome"], "refused");
    println!(
        "mission-012-context-process-e2e: signed webhook -> current Forgejo issue API -> admission -> MCP mission_context; stale label/milestone excluded, current API label 7/milestone 12 delivered; shutdown without independent verification refused."
    );
}
