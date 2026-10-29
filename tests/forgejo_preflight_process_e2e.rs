#![cfg(unix)]

use agent_forge_core::{ExpectedBinding, Outcome, ProofEvidence, sign_proof};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PR_NUMBER: u64 = 23;
const OWNER: &str = "forge-admin";
const REPOSITORY: &str = "agent-forge-core";
const BASE_BRANCH: &str = "main";
const VERIFIER_KEY: &[u8] = b"preflight-process-e2e-only-verifier-key";
const EXPECTED_BASE: &str = "2222222222222222222222222222222222222222";
const EXPECTED_HEAD: &str = "1111111111111111111111111111111111111111";

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "forgejo-preflight-process-e2e-{}-{nonce}",
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

fn write_private(path: &std::path::Path, contents: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create private adapter input");
    file.write_all(contents).expect("write adapter input");
    file.sync_all().expect("flush adapter input");
}

fn expected_binding() -> ExpectedBinding {
    ExpectedBinding {
        requirement_sha256: "a".repeat(64),
        policy_sha256: "b".repeat(64),
        candidate_commit: EXPECTED_HEAD.to_owned(),
        base_commit: EXPECTED_BASE.to_owned(),
        merge_commit: None,
        verifier_id: "independent-verifier".into(),
    }
}

fn write_preflight_inputs(root: &std::path::Path) -> (PathBuf, PathBuf, PathBuf) {
    let expected = expected_binding();
    let evidence = ProofEvidence {
        schema_version: 1,
        requirement_sha256: expected.requirement_sha256.clone(),
        policy_sha256: expected.policy_sha256.clone(),
        candidate_commit: expected.candidate_commit.clone(),
        base_commit: expected.base_commit.clone(),
        merge_commit: None,
        verifier_id: expected.verifier_id.clone(),
        environment: "private loopback fixture".into(),
        commands: vec![vec!["cargo".into(), "test".into()]],
        artifact_sha256: vec!["c".repeat(64)],
        outcome: Outcome::Pass,
        detail: "synthetic passing proof for process boundary test".into(),
    };
    let proof = sign_proof(evidence, VERIFIER_KEY).expect("sign synthetic verifier proof");
    let proof_path = root.join("proof.json");
    let expected_path = root.join("expected.json");
    let key_path = root.join("verifier.key");
    write_private(
        &proof_path,
        &serde_json::to_vec(&proof).expect("serialize synthetic proof"),
    );
    write_private(
        &expected_path,
        &serde_json::to_vec(&expected).expect("serialize expected binding"),
    );
    write_private(&key_path, VERIFIER_KEY);
    (proof_path, expected_path, key_path)
}

fn current_pull_request(base_sha: &str, head_sha: &str) -> Value {
    json!({
        "number": PR_NUMBER,
        "state": "open",
        "merged": false,
        "draft": false,
        "mergeable": true,
        "html_url": format!("http://127.0.0.1:3130/{OWNER}/{REPOSITORY}/pulls/{PR_NUMBER}"),
        "base": {
            "ref": BASE_BRANCH,
            "sha": base_sha,
            "repo": {
                "owner": {"login": OWNER},
                "name": REPOSITORY,
                "full_name": format!("{OWNER}/{REPOSITORY}")
            }
        },
        "head": {
            "sha": head_sha,
            "repo": {"full_name": format!("{OWNER}/{REPOSITORY}")}
        }
    })
}

fn read_request_headers(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound fixture request read");
    let mut request = Vec::new();
    while request.len() < 16 * 1024 && !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => request.push(byte[0]),
            Err(error) => panic!("read preflight CLI request headers: {error}"),
        }
    }
    assert!(
        request.windows(4).any(|window| window == b"\r\n\r\n"),
        "preflight CLI request headers are incomplete or oversized"
    );
    String::from_utf8(request).expect("fixture request headers are UTF-8")
}

fn try_read_request_headers(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Option<String> {
    while pending.len() < 16 * 1024 && !pending.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => {
                assert!(
                    pending.is_empty(),
                    "preflight CLI closed a partial HTTP request"
                );
                return None;
            }
            Ok(_) => pending.push(byte[0]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(error) => panic!("read follow-up preflight request headers: {error}"),
        }
    }
    assert!(
        pending.len() < 16 * 1024,
        "preflight CLI request headers exceed 16 KiB"
    );
    if pending.windows(4).any(|window| window == b"\r\n\r\n") {
        Some(String::from_utf8(std::mem::take(pending)).expect("request headers are UTF-8"))
    } else {
        None
    }
}

fn reject_extra_request(stream: &mut TcpStream, requests: &mut Vec<String>, request: String) {
    requests.push(request);
    let response =
        b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let _ = stream.write_all(response);
}

fn start_preflight_http_fixture(
    response_body: Value,
) -> (thread::JoinHandle<Vec<String>>, mpsc::Sender<()>) {
    start_preflight_http_status_fixture("200 OK", response_body)
}

fn start_preflight_http_status_fixture(
    status_line: &'static str,
    response_body: Value,
) -> (thread::JoinHandle<Vec<String>>, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:3130")
        .expect("private network namespace reserves the fixed Forgejo API endpoint");
    listener
        .set_nonblocking(true)
        .expect("bound fixture accept polling");
    let body = serde_json::to_vec(&response_body).expect("serialize current PR fixture");
    let (finished_sender, finished_receiver) = mpsc::channel();

    let fixture = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "preflight CLI made no API request"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept preflight CLI request: {error}"),
            }
        };
        let mut requests = vec![read_request_headers(&mut stream)];
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write current PR fixture response headers");
        stream
            .write_all(&body)
            .expect("write current PR fixture response body");
        stream
            .set_nonblocking(true)
            .expect("poll the original connection for pipelined follow-up requests");
        let mut pending = Vec::new();

        // Keep listening until the parent confirms the CLI process has exited.
        loop {
            if let Some(request) = try_read_request_headers(&mut stream, &mut pending) {
                reject_extra_request(&mut stream, &mut requests, request);
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request_headers(&mut stream);
                    reject_extra_request(&mut stream, &mut requests, request);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    match finished_receiver.recv_timeout(Duration::from_millis(5)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    }
                }
                Err(error) => panic!("accept unexpected preflight follow-up: {error}"),
            }
        }

        // Drain same-connection and listener requests queued before the stop signal.
        while let Some(request) = try_read_request_headers(&mut stream, &mut pending) {
            reject_extra_request(&mut stream, &mut requests, request);
        }
        while let Ok((mut stream, _)) = listener.accept() {
            let request = read_request_headers(&mut stream);
            reject_extra_request(&mut stream, &mut requests, request);
        }
        requests
    });
    (fixture, finished_sender)
}

#[test]
fn request_header_reader_preserves_a_pipelined_follow_up_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral parser fixture");
    let address = listener.local_addr().expect("read parser fixture address");
    let parser = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept parser fixture client");
        let first = read_request_headers(&mut stream);
        let second = read_request_headers(&mut stream);
        (first, second)
    });
    let mut client = TcpStream::connect(address).expect("connect parser fixture client");
    client
        .write_all(
            b"GET /first HTTP/1.1\r\nHost: fixture\r\n\r\nPOST /second HTTP/1.1\r\nHost: fixture\r\n\r\n",
        )
        .expect("send two pipelined HTTP requests");
    let (first, second) = parser.join().expect("parser fixture completed");
    assert!(first.starts_with("GET /first HTTP/1.1\r\n"));
    assert!(second.starts_with("POST /second HTTP/1.1\r\n"));
}

fn run_preflight(
    proof_path: &std::path::Path,
    expected_path: &std::path::Path,
    key_path: &std::path::Path,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(["--preflight", &PR_NUMBER.to_string()])
        .arg(proof_path)
        .arg(expected_path)
        .arg(key_path)
        .arg("fixture-agent")
        .stdin(Stdio::null())
        .output()
        .expect("run real read-only preflight CLI process")
}

fn write_expected_variant(
    root: &std::path::Path,
    name: &str,
    expected: &ExpectedBinding,
) -> PathBuf {
    let path = root.join(name);
    write_private(
        &path,
        &serde_json::to_vec(expected).expect("serialize changed expected binding"),
    );
    path
}

fn assert_refusal(
    response: Value,
    expected_code: &str,
    output: &std::process::Output,
    requests: &[String],
) {
    assert!(!output.status.success(), "stale proof is not accepted");
    assert!(
        output.stderr.is_empty(),
        "stale proof has structured stdout"
    );
    assert_eq!(response["operation"], "read_only_merge_preflight");
    assert_eq!(response["verdict"], "inconclusive");
    assert_eq!(response["error"]["code"], expected_code);
    assert_eq!(response["merge_authorized"], false);
    assert_eq!(response["merge_attempted"], false);
    assert_eq!(response["upstream_read_attempts"], 1);
    assert_eq!(
        requests.len(),
        1,
        "preflight performs no merge/write request"
    );
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the fixture CLI has no credential provider"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_preflight_cli_accepts_target_base_with_fork_head_without_writes() {
    let scratch = ScratchDir::new();
    let (proof_path, expected_path, key_path) = write_preflight_inputs(scratch.path());
    let mut response = current_pull_request(EXPECTED_BASE, EXPECTED_HEAD);
    response["head"]["repo"]["full_name"] = json!("contributor/agent-forge-core");

    let (fixture, stop_fixture) = start_preflight_http_fixture(response);
    let output = run_preflight(&proof_path, &expected_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop fork PR fixture after CLI process exits");
    let requests = fixture.join().expect("fork PR fixture completed");

    assert!(
        output.status.success(),
        "valid fork PR should pass preflight"
    );
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).expect("preflight JSON");
    assert_eq!(result["operation"], "read_only_merge_preflight");
    assert_eq!(result["verdict"], "proof_current");
    assert_eq!(result["merge_authorized"], false);
    assert_eq!(result["merge_attempted"], false);
    assert_eq!(result["upstream_read_attempts"], 1);
    assert_eq!(requests.len(), 1, "preflight issues a read only");
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "fixture preflight has no credential provider"
    );
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_preflight_cli_refuses_foreign_base_repository_without_writes() {
    let scratch = ScratchDir::new();
    let (proof_path, expected_path, key_path) = write_preflight_inputs(scratch.path());
    let mut response = current_pull_request(EXPECTED_BASE, EXPECTED_HEAD);
    response["base"]["repo"]["full_name"] = json!("other-owner/other-repository");
    response["head"]["repo"]["full_name"] = json!("contributor/agent-forge-core");

    let (fixture, stop_fixture) = start_preflight_http_fixture(response);
    let output = run_preflight(&proof_path, &expected_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop foreign-base fixture after CLI process exits");
    let requests = fixture.join().expect("foreign-base PR fixture completed");
    let result: Value = serde_json::from_slice(&output.stdout).expect("preflight JSON");

    assert_refusal(result, "wrong_scope", &output, &requests);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_preflight_cli_refuses_draft_pull_request_without_writes() {
    let scratch = ScratchDir::new();
    let (proof_path, expected_path, key_path) = write_preflight_inputs(scratch.path());
    let mut response = current_pull_request(EXPECTED_BASE, EXPECTED_HEAD);
    response["draft"] = json!(true);

    let (fixture, stop_fixture) = start_preflight_http_fixture(response);
    let output = run_preflight(&proof_path, &expected_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop draft PR fixture after CLI process exits");
    let requests = fixture.join().expect("draft PR fixture completed");
    let result: Value = serde_json::from_slice(&output.stdout).expect("draft refusal JSON");

    assert_refusal(result, "draft_pull_request", &output, &requests);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_preflight_cli_refuses_stale_head_base_requirements_and_policy_without_writes() {
    let scratch = ScratchDir::new();
    let (proof_path, expected_path, key_path) = write_preflight_inputs(scratch.path());

    let (fixture, stop_fixture) =
        start_preflight_http_fixture(current_pull_request(EXPECTED_BASE, EXPECTED_HEAD));
    let current = run_preflight(&proof_path, &expected_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop fixture after fresh CLI process exits");
    let requests = fixture.join().expect("fresh PR fixture completed");
    assert!(
        current.status.success(),
        "fresh proof preflight should pass"
    );
    assert!(current.stderr.is_empty());
    let current_json: Value =
        serde_json::from_slice(&current.stdout).expect("fresh preflight JSON");
    assert_eq!(current_json["verdict"], "proof_current");
    assert_eq!(current_json["merge_authorized"], false);
    assert_eq!(current_json["merge_attempted"], false);
    assert_eq!(requests.len(), 1, "preflight issues only a read");
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "the fixture CLI has no credential provider"
    );

    let changed_head = "4444444444444444444444444444444444444444";
    let (fixture, stop_fixture) =
        start_preflight_http_fixture(current_pull_request(EXPECTED_BASE, changed_head));
    let stale_head = run_preflight(&proof_path, &expected_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop fixture after changed-head CLI process exits");
    let requests = fixture.join().expect("changed-head PR fixture completed");
    let stale_head_json: Value =
        serde_json::from_slice(&stale_head.stdout).expect("stale-head refusal JSON");
    assert_refusal(stale_head_json, "stale_candidate", &stale_head, &requests);

    let changed_base = "3333333333333333333333333333333333333333";
    let (fixture, stop_fixture) =
        start_preflight_http_fixture(current_pull_request(changed_base, EXPECTED_HEAD));
    let stale_base = run_preflight(&proof_path, &expected_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop fixture after changed-base CLI process exits");
    let requests = fixture.join().expect("changed-base PR fixture completed");
    let stale_base_json: Value =
        serde_json::from_slice(&stale_base.stdout).expect("stale-base refusal JSON");
    assert_refusal(stale_base_json, "stale_base", &stale_base, &requests);

    let mut changed_requirements = expected_binding();
    changed_requirements.requirement_sha256 = "d".repeat(64);
    let changed_requirements_path = write_expected_variant(
        scratch.path(),
        "expected-changed-requirements.json",
        &changed_requirements,
    );
    let (fixture, stop_fixture) =
        start_preflight_http_fixture(current_pull_request(EXPECTED_BASE, EXPECTED_HEAD));
    let stale_requirements = run_preflight(&proof_path, &changed_requirements_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop fixture after changed-requirements CLI process exits");
    let requests = fixture
        .join()
        .expect("changed-requirements PR fixture completed");
    let stale_requirements_json: Value = serde_json::from_slice(&stale_requirements.stdout)
        .expect("stale-requirements refusal JSON");
    assert_refusal(
        stale_requirements_json,
        "stale_requirements",
        &stale_requirements,
        &requests,
    );

    let mut changed_policy = expected_binding();
    changed_policy.policy_sha256 = "e".repeat(64);
    let changed_policy_path = write_expected_variant(
        scratch.path(),
        "expected-changed-policy.json",
        &changed_policy,
    );
    let (fixture, stop_fixture) =
        start_preflight_http_fixture(current_pull_request(EXPECTED_BASE, EXPECTED_HEAD));
    let stale_policy = run_preflight(&proof_path, &changed_policy_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop fixture after changed-policy CLI process exits");
    let requests = fixture.join().expect("changed-policy PR fixture completed");
    let stale_policy_json: Value =
        serde_json::from_slice(&stale_policy.stdout).expect("stale-policy refusal JSON");
    assert_refusal(stale_policy_json, "stale_policy", &stale_policy, &requests);
}

#[test]
#[ignore = "run inside bwrap --unshare-net; fixture reserves the fixed allowlisted API endpoint"]
fn actual_preflight_cli_reports_anonymous_401_without_claiming_a_credential_was_sent() {
    let scratch = ScratchDir::new();
    let (proof_path, expected_path, key_path) = write_preflight_inputs(scratch.path());
    let response_marker = "private-401-body-marker";
    let (fixture, stop_fixture) =
        start_preflight_http_status_fixture("401 Unauthorized", json!({"message":response_marker}));

    let output = run_preflight(&proof_path, &expected_path, &key_path);
    stop_fixture
        .send(())
        .expect("stop unauthorized fixture after preflight exits");
    let requests = fixture.join().expect("unauthorized PR fixture completed");

    assert!(
        !output.status.success(),
        "an anonymous 401 is not a fresh proof"
    );
    assert!(
        output.stderr.is_empty(),
        "refusal must be structured JSON on stdout"
    );
    let response: Value = serde_json::from_slice(&output.stdout).expect("401 preflight JSON");
    assert_eq!(response["operation"], "read_only_merge_preflight");
    assert_eq!(response["verdict"], "inconclusive");
    assert_eq!(response["merge_authorized"], false);
    assert_eq!(response["merge_attempted"], false);
    assert_eq!(response["upstream_read_attempts"], 1);
    assert_eq!(response["error"]["code"], "upstream_unauthorized");
    assert_eq!(response["error"]["retryable"], false);
    assert_eq!(response["error"]["details"]["http_status"], 401);
    let action = response["error"]["action"]
        .as_str()
        .expect("actionable refusal");
    assert!(
        action.contains("anonymous"),
        "action must explain the auth contract: {action}"
    );
    assert!(
        action.contains("sends no credential"),
        "action must not imply a credential was sent: {action}"
    );
    assert_eq!(requests.len(), 1, "401 preflight performs one read only");
    assert!(
        requests[0]
            .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1\r\n")
    );
    assert!(
        !requests[0]
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "read-only preflight is explicitly anonymous"
    );
    let output_text = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output_text.contains(response_marker),
        "upstream body must not leak"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains(response_marker));
}
