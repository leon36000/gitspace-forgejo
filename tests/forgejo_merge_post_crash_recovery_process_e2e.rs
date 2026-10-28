#![cfg(unix)]

//! Real CLI process-restart checks for the durable Forgejo merge intent.
//!
//! Run only inside an isolated network namespace: this fixture binds the
//! compile-time mission profile at 127.0.0.1:3132.

use agent_forge_core::{
    ContextManifest, ContextSource, EventJournal, ExpectedBinding, Outcome, Policy, ProofEvidence,
    ProofTargetV2,
    agent_runtime::{
        AgentExecutionRequestV1, ContextCitationV1, ContextPacketV1, MergeAttemptBindingV1,
        MergeAttemptResolutionV1, MissionExecutionJournalV1, RuntimeBudgetV1,
        RuntimeCapabilitiesV1, WorkspaceRefV1, derive_mission_identity_v1,
    },
    forgejo::{BaseBranchScope, MergeApprovalPolicyV1},
    mission_admission::{MissionHostAdmissionV1, MissionHostAllocationV1, MissionHostScopeV1},
    sha256, sign_target_bound_proof_v2,
    supervisor_capability::{
        MergeCapabilityTargetV1, ProcessBindingV1, SupervisorMergeCapabilityEnvelopeV1,
        monotonic_time_nanoseconds,
    },
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::ops::{Deref, DerefMut};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            fs::{OpenOptionsExt, PermissionsExt},
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

const ORIGIN: &str = "http://127.0.0.1:3132";
const OWNER: &str = "forge-admin";
const REPOSITORY: &str = "mission-012-clamp";
const PR_NUMBER: u64 = 23;
const ISSUE_NUMBER: u64 = 1;
const BASE_SHA: &str = "1111111111111111111111111111111111111111";
const HEAD_SHA: &str = "2222222222222222222222222222222222222222";
const MERGE_SHA: &str = "3333333333333333333333333333333333333333";
const ADVANCED_BASE_SHA: &str = "4444444444444444444444444444444444444444";
const VERIFIER_KEY: &[u8] = b"forgejo-process-recovery-key-material-0123456789";
const HOST_TOKEN: &[u8] = b"synthetic-loopback-merge-token";
const WEBHOOK_SECRET: &[u8] = b"synthetic-loopback-webhook-secret-for-merge-bridge";
const REQUIREMENTS: &[u8] =
    include_bytes!("fixtures/mission-012/requirements/MISSION-012-CLAMP-001.json");

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "forgejo-merge-recovery-process-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create private process fixture directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("restrict process fixture directory permissions");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct MergeInputs {
    _directory: ScratchDir,
    admission: PathBuf,
    journal: PathBuf,
    proof: PathBuf,
    expected: PathBuf,
    key: PathBuf,
    policy: PathBuf,
    review: PathBuf,
}

fn write_private(root: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = root.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create private host input");
    file.write_all(contents).expect("write private host input");
    file.sync_all().expect("flush private host input");
    path
}

struct LocalWebhookReceiver(Child);

impl Drop for LocalWebhookReceiver {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn start_ready_webhook_receiver(
    secret: &Path,
    journal: &Path,
) -> (LocalWebhookReceiver, std::net::SocketAddr) {
    for _ in 0..5 {
        let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve webhook loopback port");
        let address = reservation
            .local_addr()
            .expect("read webhook loopback address");
        drop(reservation);

        let child = Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
            .env_clear()
            .arg(address.to_string())
            .arg(secret)
            .arg(journal)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the actual webhook receiver binary");
        let mut receiver = LocalWebhookReceiver(child);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if receiver
                .0
                .try_wait()
                .expect("check webhook receiver startup")
                .is_some()
            {
                break;
            }
            if webhook_receiver_is_ready(address) {
                return (receiver, address);
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
    panic!("webhook receiver did not bind and pass its readiness probe after five port attempts");
}

fn webhook_receiver_is_ready(address: std::net::SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .is_err()
        || stream
            .write_all(
                b"GET /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n",
            )
            .is_err()
        || stream.shutdown(Shutdown::Write).is_err()
    {
        return false;
    }
    let mut response = Vec::new();
    if stream.take(8 * 1024).read_to_end(&mut response).is_err() {
        return false;
    }
    String::from_utf8(response)
        .ok()
        .and_then(|response| {
            response
                .split_once("\r\n\r\n")
                .map(|(headers, _)| headers.to_owned())
        })
        .and_then(|headers| headers.split_whitespace().nth(1)?.parse::<u16>().ok())
        == Some(405)
}

fn stop_webhook_receiver(receiver: &mut LocalWebhookReceiver) -> ExitStatus {
    let result = unsafe { libc::kill(receiver.0.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(result, 0, "send graceful shutdown to webhook receiver");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = receiver
            .0
            .try_wait()
            .expect("wait for webhook receiver shutdown")
        {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = receiver.0.kill();
            let _ = receiver.0.wait();
            panic!("webhook receiver did not shut down within five seconds");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn send_signed_webhook(
    address: std::net::SocketAddr,
    delivery_id: &str,
    event_name: &str,
    body: &[u8],
) -> (u16, Value) {
    let mut mac = HmacSha256::new_from_slice(WEBHOOK_SECRET).expect("valid fixture webhook key");
    mac.update(body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let mut request = format!(
        "POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Forgejo-Delivery: {}\r\nX-Forgejo-Event: {}\r\nX-Forgejo-Signature: {}\r\n\r\n",
        body.len(), delivery_id, event_name, signature
    )
    .into_bytes();
    request.extend_from_slice(body);
    let mut stream = TcpStream::connect(address).expect("connect to actual webhook receiver");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("bound webhook response read");
    stream.write_all(&request).expect("send signed webhook");
    stream
        .shutdown(Shutdown::Write)
        .expect("half close webhook request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read webhook receiver response");
    let response = String::from_utf8(response).expect("webhook HTTP response is UTF-8");
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .expect("webhook HTTP response has a body separator");
    let status = headers
        .split_whitespace()
        .nth(1)
        .expect("webhook HTTP status exists")
        .parse()
        .expect("webhook HTTP status is numeric");
    (
        status,
        serde_json::from_str(body).expect("webhook response is JSON"),
    )
}

fn request_and_admission(root: &Path) -> (PathBuf, PathBuf, String) {
    let requirement_json = std::str::from_utf8(REQUIREMENTS)
        .expect("versioned requirement fixture is UTF-8")
        .to_owned();
    let requirement_sha256 = sha256(REQUIREMENTS);
    let scope = BaseBranchScope::new(ORIGIN, OWNER, REPOSITORY, "main")
        .expect("mission scope uses the exact compiled profile");
    let journal_path = root.join("mission-execution.jsonl");
    let journal = MissionExecutionJournalV1::open(&journal_path);
    let allocation = journal
        .allocate_generation(
            &scope,
            ISSUE_NUMBER,
            &requirement_sha256,
            "process-crash-recovery-001",
        )
        .expect("allocate one durable process fixture generation");
    let identity = derive_mission_identity_v1(
        &scope,
        ISSUE_NUMBER,
        &requirement_sha256,
        allocation.execution_generation,
    )
    .expect("derive admitted mission identity");
    let context_text = "Inspect src/lib.rs at the admitted base before changing clamp_to_range.";
    let request = AgentExecutionRequestV1 {
        schema_version: 1,
        mission_id: identity.mission_id.clone(),
        idempotency_key: identity.idempotency_key,
        requirement_sha256: requirement_sha256.clone(),
        requirements_json: requirement_json,
        untrusted_work_item: None,
        requirement_arbitration: None,
        context: ContextPacketV1 {
            manifest: ContextManifest {
                schema_version: 1,
                source_commit: BASE_SHA.to_owned(),
                requirement_sha256: requirement_sha256.clone(),
                token_budget: 128,
                estimated_tokens: 24,
                sources: vec![ContextSource {
                    path: "src/lib.rs".to_owned(),
                    line_start: 1,
                    line_end: 1,
                    file_sha256: "b".repeat(64),
                    provenance: format!("git:{BASE_SHA}:src/lib.rs#L1"),
                }],
            },
            text: context_text.to_owned(),
            citations: vec![ContextCitationV1 {
                text_start_byte: 0,
                text_end_byte: context_text.len(),
                source_index: 0,
            }],
        },
        workspace: WorkspaceRefV1 {
            id: "crash-recovery-workspace".to_owned(),
            base_commit: BASE_SHA.to_owned(),
        },
        capabilities: RuntimeCapabilitiesV1 {
            read_paths: vec!["src/lib.rs".to_owned()],
            write_paths: vec!["src/lib.rs".to_owned()],
            tools: vec!["workspace.read_text".to_owned(), "cargo.test".to_owned()],
            budget: RuntimeBudgetV1 {
                max_tool_calls: 4,
                timeout_ms: 10_000,
                max_output_tokens: 1000,
            },
        },
    };
    journal
        .claim_allocated(
            &scope,
            ISSUE_NUMBER,
            allocation.execution_generation,
            &allocation.allocation_id,
            &request,
        )
        .expect("durably claim exact mission request");
    let admission = MissionHostAdmissionV1 {
        schema_version: 1,
        scope: MissionHostScopeV1 {
            origin: ORIGIN.to_owned(),
            owner: OWNER.to_owned(),
            repository: REPOSITORY.to_owned(),
            branch: "main".to_owned(),
        },
        issue_number: ISSUE_NUMBER,
        base_commit: BASE_SHA.to_owned(),
        requirement_path: "requirements/MISSION-012-CLAMP-001.json".to_owned(),
        requirement_sha256,
        allocation: MissionHostAllocationV1 {
            schema_version: allocation.schema_version,
            mission_id: allocation.mission_id,
            allocation_id: allocation.allocation_id,
            requirement_sha256: allocation.requirement_sha256,
            execution_generation: allocation.execution_generation,
        },
        request,
    };
    admission.validate().expect("valid host admission handoff");
    let admission_bytes = serde_json::to_vec(&admission).expect("serialize host admission");
    let admission_sha256 = sha256(&admission_bytes);
    let admission_path = write_private(root, "admission.json", &admission_bytes);
    (admission_path, journal_path, admission_sha256)
}

fn build_inputs() -> MergeInputs {
    let directory = ScratchDir::new();
    let (admission, journal, admission_sha256) = request_and_admission(directory.path());
    let requirement_sha256 = sha256(REQUIREMENTS);
    let policy = Policy {
        schema_version: 1,
        revision: 1,
        verifier_id: "independent-verifier".to_owned(),
        environment: "private process crash fixture".to_owned(),
        acceptance_command: vec!["cargo".to_owned(), "test".to_owned()],
        commands: vec![vec!["cargo".to_owned(), "test".to_owned()]],
        protected_criteria: vec!["requirements/**".to_owned()],
        timeout_seconds: 60,
        merge_approval_policy: Some(MergeApprovalPolicyV1 {
            schema_version: 1,
            required_approvals: 1,
            authorized_reviewer_ids: vec![42],
        }),
    };
    let policy_bytes = serde_json::to_vec(&policy).expect("serialize private gate policy");
    let expected = ExpectedBinding {
        requirement_sha256,
        policy_sha256: sha256(&policy_bytes),
        candidate_commit: HEAD_SHA.to_owned(),
        base_commit: BASE_SHA.to_owned(),
        merge_commit: None,
        verifier_id: policy.verifier_id.clone(),
    };
    let target = ProofTargetV2::new("mission-012-clamp", ORIGIN, OWNER, REPOSITORY, "main")
        .expect("construct exact target-bound proof scope");
    let proof = sign_target_bound_proof_v2(
        ProofEvidence {
            schema_version: 1,
            requirement_sha256: expected.requirement_sha256.clone(),
            policy_sha256: expected.policy_sha256.clone(),
            candidate_commit: expected.candidate_commit.clone(),
            base_commit: expected.base_commit.clone(),
            merge_commit: None,
            verifier_id: expected.verifier_id.clone(),
            environment: "private process crash fixture".to_owned(),
            commands: vec![vec!["cargo".to_owned(), "test".to_owned()]],
            artifact_sha256: vec!["c".repeat(64)],
            outcome: Outcome::Pass,
            detail: "signed fixture proof for process-boundary testing".to_owned(),
        },
        target,
        VERIFIER_KEY,
    )
    .expect("sign target-bound fixture proof");
    let review = json!({
        "schema_version": 1,
        "review_id": 17,
        "reviewer_id": 42,
        "reviewer_login": "verifier-bot",
        "pull_request_number": PR_NUMBER,
        "base_branch": "main",
        "base_commit": BASE_SHA,
        "candidate_commit": HEAD_SHA,
        "source": "isolated process crash fixture",
        "mission_admission_sha256": admission_sha256,
        "mission_source_issue_number": ISSUE_NUMBER,
    });
    MergeInputs {
        proof: write_private(
            directory.path(),
            "proof.json",
            &serde_json::to_vec(&proof).expect("serialize signed proof"),
        ),
        expected: write_private(
            directory.path(),
            "expected.json",
            &serde_json::to_vec(&expected).expect("serialize expected binding"),
        ),
        key: write_private(directory.path(), "verifier.key", VERIFIER_KEY),
        policy: write_private(directory.path(), "policy.json", &policy_bytes),
        review: write_private(
            directory.path(),
            "review.json",
            &serde_json::to_vec(&review).expect("serialize review binding"),
        ),
        _directory: directory,
        admission,
        journal,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CrashPoint {
    NoCrash,
    AfterForgejoCommitsPost,
    AfterDurableIntentBeforePost,
    AfterFinalProtectionReadBeforePost,
    BaseAdvancesAfterFinalCheckBeforePost,
    ChangedReviewTotalsAcrossPages,
    UnauthorizedSecondReviewPage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FixtureNotice {
    MergePostCommitted,
    MergePostReceivedBeforeBaseAdvance,
    IntentSeenBeforePost,
    FinalProtectionReadAfterIntent,
}

struct ForgejoFixture {
    merged: Arc<AtomicBool>,
    current_main_sha: Arc<Mutex<String>>,
    merge_parent_sha: Arc<Mutex<String>>,
    requests: Arc<Mutex<Vec<String>>>,
    notice_receiver: mpsc::Receiver<FixtureNotice>,
    release_sender: mpsc::Sender<()>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ForgejoFixture {
    fn start(crash_point: CrashPoint, journal_path: PathBuf) -> Self {
        Self::start_with_draft(crash_point, journal_path, false)
    }

    fn start_with_draft(
        crash_point: CrashPoint,
        journal_path: PathBuf,
        current_draft: bool,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:3132")
            .expect("sandbox reserves the exact fixed Forgejo test profile");
        listener
            .set_nonblocking(true)
            .expect("fixture accept loop is stoppable");
        let merged = Arc::new(AtomicBool::new(false));
        let current_main_sha = Arc::new(Mutex::new(BASE_SHA.to_owned()));
        let merge_parent_sha = Arc::new(Mutex::new(BASE_SHA.to_owned()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_merged = Arc::clone(&merged);
        let worker_current_main_sha = Arc::clone(&current_main_sha);
        let worker_merge_parent_sha = Arc::clone(&merge_parent_sha);
        let worker_requests = Arc::clone(&requests);
        let worker_stop = Arc::clone(&stop);
        let (notice_sender, notice_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut intent_notice_sent = false;
            let mut post_notice_sent = false;
            let mut final_protection_notice_sent = false;
            while !worker_stop.load(Ordering::SeqCst) {
                assert!(
                    Instant::now() < deadline,
                    "Forgejo process fixture timed out"
                );
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(3));
                        continue;
                    }
                    Err(error) => panic!("accept isolated Forgejo request: {error}"),
                };
                let request = read_request(&mut stream);
                let line = request.lines().next().unwrap_or_default().to_owned();
                worker_requests.lock().unwrap().push(line.clone());
                let method = line.split(' ').next().unwrap_or_default();
                let target = line.split(' ').nth(1).unwrap_or_default();
                if method == "POST" && target.ends_with("/pulls/23/merge") {
                    if crash_point == CrashPoint::BaseAdvancesAfterFinalCheckBeforePost
                        && !post_notice_sent
                    {
                        post_notice_sent = true;
                        notice_sender
                            .send(FixtureNotice::MergePostReceivedBeforeBaseAdvance)
                            .expect("test observes merge POST before fixture applies it");
                        let _ = release_receiver.recv_timeout(Duration::from_secs(20));
                    }
                    *worker_merge_parent_sha.lock().unwrap() =
                        worker_current_main_sha.lock().unwrap().clone();
                    worker_merged.store(true, Ordering::SeqCst);
                    if crash_point == CrashPoint::AfterForgejoCommitsPost && !post_notice_sent {
                        post_notice_sent = true;
                        notice_sender
                            .send(FixtureNotice::MergePostCommitted)
                            .expect("test observes committed POST before releasing response");
                        let _ = release_receiver.recv_timeout(Duration::from_secs(20));
                        let _ = write_json_response(&mut stream, 200, None, &[]);
                        continue;
                    }
                }
                if crash_point == CrashPoint::AfterDurableIntentBeforePost
                    && !intent_notice_sent
                    && method == "GET"
                    && journal_contains_started_intent(&journal_path)
                {
                    intent_notice_sent = true;
                    notice_sender
                        .send(FixtureNotice::IntentSeenBeforePost)
                        .expect("test observes durable intent before releasing read");
                    let _ = release_receiver.recv_timeout(Duration::from_secs(20));
                    let _ = write_json_response(&mut stream, 200, None, b"{}");
                    continue;
                }
                if matches!(
                    crash_point,
                    CrashPoint::AfterFinalProtectionReadBeforePost
                        | CrashPoint::BaseAdvancesAfterFinalCheckBeforePost
                ) && !final_protection_notice_sent
                    && method == "GET"
                    && target == format!("/api/v1/repos/{OWNER}/{REPOSITORY}/branch_protections")
                    && journal_contains_started_intent(&journal_path)
                {
                    final_protection_notice_sent = true;
                    notice_sender
                        .send(FixtureNotice::FinalProtectionReadAfterIntent)
                        .expect("test observes final Forgejo read after durable intent");
                    let _ = release_receiver.recv_timeout(Duration::from_secs(20));
                    let (status, total_count, body) = fixture_response(
                        target,
                        worker_merged.load(Ordering::SeqCst),
                        &worker_current_main_sha.lock().unwrap(),
                        &worker_merge_parent_sha.lock().unwrap(),
                        current_draft,
                    );
                    write_json_response(&mut stream, status, total_count, &body)
                        .expect("release final protected-state read");
                    continue;
                }
                if crash_point == CrashPoint::ChangedReviewTotalsAcrossPages {
                    let review_prefix = format!(
                        "/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{PR_NUMBER}/reviews?limit=50&page="
                    );
                    if let Some(page) = target
                        .strip_prefix(&review_prefix)
                        .and_then(|value| value.parse::<u64>().ok())
                    {
                        let total = match page {
                            1 => 51,
                            2 => 52,
                            _ => panic!("merge authority requested unexpected review page {page}"),
                        };
                        let (first_review, last_review) = match page {
                            1 => (1, 50),
                            2 => (51, 52),
                            _ => unreachable!("page number was validated above"),
                        };
                        let reviews = (first_review..=last_review)
                            .map(|id| {
                                json!({
                                    "id": id,
                                    "user": {"id": 42},
                                    "state": "APPROVED",
                                    "commit_id": HEAD_SHA,
                                    "pull_request_url": format!("{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/{PR_NUMBER}"),
                                    "stale": false,
                                    "official": true,
                                    "dismissed": false
                                })
                            })
                            .collect::<Vec<_>>();
                        let body = serde_json::to_vec(&reviews)
                            .expect("serialize complete review page contents");
                        write_json_response(&mut stream, 200, Some(total), &body)
                            .expect("write deliberately inconsistent review page totals");
                        continue;
                    }
                }
                if crash_point == CrashPoint::UnauthorizedSecondReviewPage {
                    let review_prefix = format!(
                        "/api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{PR_NUMBER}/reviews?limit=50&page="
                    );
                    if let Some(page) = target
                        .strip_prefix(&review_prefix)
                        .and_then(|value| value.parse::<u64>().ok())
                    {
                        match page {
                            1 => {
                                let reviews = (1..=50)
                                    .map(|id| {
                                        json!({
                                            "id": id,
                                            "user": {"id": 42},
                                            "state": "APPROVED",
                                            "commit_id": HEAD_SHA,
                                            "pull_request_url": format!("{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/{PR_NUMBER}"),
                                            "stale": false,
                                            "official": true,
                                            "dismissed": false
                                        })
                                    })
                                    .collect::<Vec<_>>();
                                let body = serde_json::to_vec(&reviews)
                                    .expect("serialize the first complete review page");
                                write_json_response(&mut stream, 200, Some(51), &body)
                                    .expect("write first review page before authorization refusal");
                            }
                            2 => {
                                write_json_response(
                                    &mut stream,
                                    401,
                                    None,
                                    br#"{"message":"private-review-401-marker"}"#,
                                )
                                .expect("write unauthorized second review page");
                            }
                            _ => panic!("merge authority requested unexpected review page {page}"),
                        }
                        continue;
                    }
                }
                let (status, total_count, body) = fixture_response(
                    target,
                    worker_merged.load(Ordering::SeqCst),
                    &worker_current_main_sha.lock().unwrap(),
                    &worker_merge_parent_sha.lock().unwrap(),
                    current_draft,
                );
                write_json_response(&mut stream, status, total_count, &body)
                    .expect("write bounded fixture response");
            }
        });
        Self {
            merged,
            current_main_sha,
            merge_parent_sha,
            requests,
            notice_receiver,
            release_sender,
            stop,
            worker: Some(worker),
        }
    }

    fn wait_notice(&self) -> FixtureNotice {
        self.notice_receiver
            .recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|error| {
                panic!(
                    "child did not reach its configured crash boundary: {error:?}; fixture requests: {:?}",
                    self.requests()
                )
            })
    }

    fn release_held_request(&self) {
        self.release_sender
            .send(())
            .expect("release fixture request after child was killed");
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    fn merged(&self) -> bool {
        self.merged.load(Ordering::SeqCst)
    }

    fn advance_main(&self, sha: &str) {
        *self.current_main_sha.lock().unwrap() = sha.to_owned();
    }

    fn merge_parent(&self) -> String {
        self.merge_parent_sha.lock().unwrap().clone()
    }
}

impl Drop for ForgejoFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("fixture worker exits cleanly");
        }
    }
}

fn journal_contains_started_intent(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    bytes.split(|byte| *byte == b'\n').any(|line| {
        serde_json::from_slice::<Value>(line)
            .ok()
            .and_then(|event| event.get("kind").and_then(Value::as_str).map(str::to_owned))
            .as_deref()
            == Some("mission_merge_attempt_started_v1")
    })
}

fn fixture_response(
    target: &str,
    merged: bool,
    current_main_sha: &str,
    merge_parent_sha: &str,
    draft: bool,
) -> (u16, Option<usize>, Vec<u8>) {
    let repo = format!("/api/v1/repos/{OWNER}/{REPOSITORY}");
    let pull_url = format!("{ORIGIN}/{OWNER}/{REPOSITORY}/pulls/{PR_NUMBER}");
    let full_name = format!("{OWNER}/{REPOSITORY}");
    let value = if target == format!("{repo}/pulls/{PR_NUMBER}") {
        if merged {
            json!({
                "number": PR_NUMBER,
                "html_url": pull_url,
                "state": "closed",
                "merged": true,
                "draft": draft,
                "merge_commit_sha": MERGE_SHA,
                "updated_at": "2026-10-25T22:00:00Z",
                "base": {"ref":"main","sha":MERGE_SHA,"repo":{"full_name":full_name}},
                "head": {"sha":HEAD_SHA,"repo":{"full_name":full_name}}
            })
        } else {
            json!({
                "number": PR_NUMBER,
                "html_url": pull_url,
                "state": "open",
                "merged": false,
                "draft": draft,
                "mergeable": true,
                "base": {"ref":"main","sha":BASE_SHA,"repo":{"full_name":full_name}},
                "head": {"sha":HEAD_SHA,"repo":{"full_name":full_name}}
            })
        }
    } else if target == "/api/v1/user" {
        json!({"id":9,"login":"forge-admin"})
    } else if target == format!("{repo}/pulls/{PR_NUMBER}/reviews?limit=50&page=1") {
        json!([{
            "id": 17,
            "user": {"id": 42},
            "state": "APPROVED",
            "commit_id": HEAD_SHA,
            "pull_request_url": pull_url,
            "stale": false,
            "official": true,
            "dismissed": false
        }])
    } else if target == format!("{repo}/branch_protections") {
        json!([{
            "rule_name": "main",
            "branch_name": "main",
            "required_approvals": 1,
            "enable_approvals_whitelist": true,
            "approvals_whitelist_username": ["verifier-bot"],
            "apply_to_admins": true,
            "dismiss_stale_approvals": true,
            "block_on_outdated_branch": true,
            "enable_merge_whitelist": true,
            "merge_whitelist_usernames": ["forge-admin"],
            "merge_whitelist_teams": [],
            "enable_push": false,
            "unprotected_file_patterns": ""
        }])
    } else if target == "/api/v1/users/verifier-bot" {
        json!({"id":42,"login":"verifier-bot"})
    } else if target == format!("{repo}/branches/main") {
        json!({"commit":{"id": if merged { MERGE_SHA } else { current_main_sha }}})
    } else if target == format!("{repo}/git/commits/{MERGE_SHA}") && merged {
        json!({"sha":MERGE_SHA,"parents":[{"sha":merge_parent_sha},{"sha":HEAD_SHA}]})
    } else if target == format!("{repo}/git/commits/{HEAD_SHA}") && merged {
        json!({"sha":HEAD_SHA})
    } else if target == format!("{repo}/issues/{ISSUE_NUMBER}") {
        json!({
            "number": ISSUE_NUMBER,
            "state": "closed",
            "updated_at": "2026-10-27T13:40:00Z",
            "html_url": format!("{ORIGIN}/{OWNER}/{REPOSITORY}/issues/{ISSUE_NUMBER}"),
            "repository": {"full_name": full_name},
            "title": "canonical title from Forgejo",
            "body": "canonical body from Forgejo"
        })
    } else if target == format!("{repo}/issues/{PR_NUMBER}") {
        json!({"number":PR_NUMBER,"state":if merged { "closed" } else { "open" }})
    } else if target == format!("{repo}/git/trees/{MERGE_SHA}?page=1&per_page=1000&recursive=false")
        && merged
    {
        tree_response(MERGE_SHA)
    } else if target == format!("{repo}/git/trees/{HEAD_SHA}?page=1&per_page=1000&recursive=false")
        && merged
    {
        tree_response(HEAD_SHA)
    } else {
        return (404, None, br#"{"message":"not found"}"#.to_vec());
    };
    let total_count =
        (target == format!("{repo}/pulls/{PR_NUMBER}/reviews?limit=50&page=1")).then_some(1);
    (
        200,
        total_count,
        serde_json::to_vec(&value).expect("serialize fixture response"),
    )
}

fn tree_response(sha: &str) -> Value {
    json!({
        "sha": sha,
        "tree": [{"path":"src","mode":"040000","type":"tree","size":0,"sha":"5555555555555555555555555555555555555555"}],
        "truncated": false,
        "page": 1,
        "total_count": 1
    })
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("bound fixture request read");
    let mut request = Vec::new();
    let header_end = loop {
        let mut byte = [0_u8; 1];
        let count = stream.read(&mut byte).expect("read fixture request header");
        assert_ne!(
            count, 0,
            "connection closed before complete request headers"
        );
        request.push(byte[0]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        assert!(
            request.len() < 16 * 1024,
            "fixture request headers are bounded"
        );
    };
    let headers = String::from_utf8(request[..header_end].to_vec())
        .expect("fixture request headers are UTF-8");
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while request.len() - header_end < content_length {
        let mut buffer = [0_u8; 1024];
        let count = stream.read(&mut buffer).expect("read fixture request body");
        assert_ne!(count, 0, "fixture request body is truncated");
        request.extend_from_slice(&buffer[..count]);
    }
    headers
}

fn write_json_response(
    stream: &mut TcpStream,
    status: u16,
    total_count: Option<usize>,
    body: &[u8],
) -> std::io::Result<()> {
    let total_header = total_count
        .map(|count| format!("X-Total-Count: {count}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\n{total_header}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

struct SupervisedMergeChild {
    child: Child,
    _supervisor_endpoint: OwnedFd,
}

impl SupervisedMergeChild {
    fn wait_with_output(self) -> std::io::Result<std::process::Output> {
        let Self {
            child,
            _supervisor_endpoint,
        } = self;
        let result = child.wait_with_output();
        drop(_supervisor_endpoint);
        result
    }
}

impl Deref for SupervisedMergeChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl DerefMut for SupervisedMergeChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

fn process_binding_for_pid(pid: u32) -> Result<ProcessBindingV1, String> {
    let start_time_ticks = || -> Result<u64, String> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|error| format!("read child process stat: {error}"))?;
        let close_paren = stat
            .rfind(')')
            .ok_or_else(|| "child comm ends before stat fields".to_owned())?;
        stat[close_paren + 1..]
            .split_whitespace()
            .nth(19)
            .and_then(|field| field.parse::<u64>().ok())
            .ok_or_else(|| "child process start time is unavailable".to_owned())
    };
    let before = start_time_ticks()?;
    let binding = ProcessBindingV1 {
        pid,
        start_time_ticks: before,
        pid_namespace: fs::read_link(format!("/proc/{pid}/ns/pid"))
            .map_err(|error| format!("read child PID namespace: {error}"))?
            .to_string_lossy()
            .into_owned(),
        network_namespace: fs::read_link(format!("/proc/{pid}/ns/net"))
            .map_err(|error| format!("read child network namespace: {error}"))?
            .to_string_lossy()
            .into_owned(),
    };
    if start_time_ticks()? != before {
        return Err("child PID was reused while reading process namespaces".to_owned());
    }
    Ok(binding)
}

fn send_test_supervisor_capability(supervisor_endpoint: &OwnedFd, gate: ProcessBindingV1) {
    let supervisor = ProcessBindingV1::current().expect("read test supervisor process binding");
    let now = monotonic_time_nanoseconds().expect("read monotonic clock for capability");
    let target = MergeCapabilityTargetV1 {
        profile_selector: "mission-012-clamp".to_owned(),
        origin: ORIGIN.to_owned(),
        owner: OWNER.to_owned(),
        repository: REPOSITORY.to_owned(),
        pull_request_number: PR_NUMBER,
        base_branch: "main".to_owned(),
    };
    let nonce = sha256(format!("{}:{}:{now}", supervisor.pid, gate.start_time_ticks).as_bytes());
    let envelope = SupervisorMergeCapabilityEnvelopeV1 {
        schema_version: 1,
        target,
        supervisor: supervisor.clone(),
        supervisor_uid: unsafe { libc::geteuid() },
        gate,
        // The test HTTP fixture is a thread in the trusted test supervisor process.
        forgejo: supervisor,
        expires_at_monotonic_ns: now + 60_000_000_000,
        nonce,
    };
    let packet = serde_json::to_vec(&envelope).expect("serialize synthetic supervisor receipt");
    let sent = unsafe {
        libc::send(
            supervisor_endpoint.as_raw_fd(),
            packet.as_ptr().cast(),
            packet.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    assert_eq!(sent, packet.len() as isize, "send one capability packet");
}

fn spawn_merge_cli(inputs: &MergeInputs) -> SupervisedMergeChild {
    let mut pipe = [0; 2];
    // SAFETY: pipe writes two descriptors into the initialized array.
    assert_eq!(
        unsafe { libc::pipe(pipe.as_mut_ptr()) },
        0,
        "create private token pipe"
    );
    // SAFETY: pipe returned owned descriptors which are transferred to File.
    let reader = unsafe { File::from_raw_fd(pipe[0]) };
    // SAFETY: pipe returned a second owned descriptor.
    let mut writer = unsafe { File::from_raw_fd(pipe[1]) };
    let writer_flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFD) };
    assert!(writer_flags >= 0);
    assert_eq!(
        unsafe {
            libc::fcntl(
                writer.as_raw_fd(),
                libc::F_SETFD,
                writer_flags | libc::FD_CLOEXEC,
            )
        },
        0,
        "prevent the child from keeping the token pipe open"
    );
    writer.write_all(HOST_TOKEN).expect("write synthetic token");
    drop(writer);
    let descriptor = reader.as_raw_fd();
    let mut capability_descriptors = [-1; 2];
    assert_eq!(
        unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                0,
                capability_descriptors.as_mut_ptr(),
            )
        },
        0,
        "create the test supervisor capability channel"
    );
    let gate_socket = unsafe { OwnedFd::from_raw_fd(capability_descriptors[0]) };
    let supervisor_endpoint = unsafe { OwnedFd::from_raw_fd(capability_descriptors[1]) };
    let gate_socket_flags = unsafe { libc::fcntl(gate_socket.as_raw_fd(), libc::F_GETFD) };
    assert!(gate_socket_flags >= 0);
    assert_eq!(
        unsafe {
            libc::fcntl(
                gate_socket.as_raw_fd(),
                libc::F_SETFD,
                gate_socket_flags & !libc::FD_CLOEXEC,
            )
        },
        0,
        "inherit only the gate endpoint into the merge CLI"
    );
    let capability_descriptor = gate_socket.as_raw_fd().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    child
        .env_clear()
        .args([
            "--merge-local-test",
            "--writer-admission-profile",
            "local_single_writer_sandbox",
            "--target-profile",
            "mission-012-clamp",
            "--supervisor-capability-fd",
            capability_descriptor.as_str(),
            "--mission-admission-file",
        ])
        .arg(&inputs.admission)
        .arg("--mission-execution-journal")
        .arg(&inputs.journal)
        .args([PR_NUMBER.to_string()])
        .arg(&inputs.proof)
        .arg(&inputs.expected)
        .arg(&inputs.key)
        .arg(&inputs.policy)
        .arg(&inputs.review)
        .arg("fixture-agent")
        .arg(descriptor.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut process = child
        .spawn()
        .expect("spawn the real Forgejo reconciliation CLI");
    drop(reader);
    drop(gate_socket);
    let gate_binding_deadline = Instant::now() + Duration::from_secs(2);
    let gate = loop {
        match process_binding_for_pid(process.id()) {
            Ok(binding) => break binding,
            Err(error) => {
                if let Some(status) = process
                    .try_wait()
                    .expect("check merge child while binding its identity")
                {
                    let output = process
                        .wait_with_output()
                        .expect("collect merge child that exited before its capability");
                    panic!(
                        "merge child exited before supervisor capability binding: {status:?}; process binding error={error}; stdout={}; stderr={}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                assert!(
                    Instant::now() < gate_binding_deadline,
                    "merge child process binding stayed unavailable: {error}"
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
    };
    send_test_supervisor_capability(&supervisor_endpoint, gate);
    SupervisedMergeChild {
        child: process,
        _supervisor_endpoint: supervisor_endpoint,
    }
}

fn kill_process_group(child: &mut Child) -> ExitStatus {
    let process_group = -(child.id() as i32);
    // SAFETY: the child established its own process group and all inherited
    // curl work belongs to that group, so the SIGKILL targets this test child.
    assert_eq!(unsafe { libc::kill(process_group, libc::SIGKILL) }, 0);
    child.wait().expect("reap killed CLI process group")
}

fn output_json(output: &std::process::Output) -> Value {
    assert!(
        output.stderr.is_empty(),
        "the real CLI keeps structured decisions on stdout: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse structured CLI decision")
}

fn retry_recovery_after_mission_contention(
    inputs: &MergeInputs,
    mut output: std::process::Output,
) -> std::process::Output {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let result = output_json(&output);
        if output.status.success() {
            return output;
        }
        let retryable_busy = result.pointer("/error/code").and_then(Value::as_str)
            == Some("execution_mission_busy")
            && result.pointer("/error/retryable").and_then(Value::as_bool) == Some(true);
        assert!(
            retryable_busy && Instant::now() < deadline,
            "recovery failed for a non-retryable reason or exceeded its bounded contention retry: {result}"
        );
        thread::sleep(Duration::from_millis(10));
        output = spawn_merge_cli(inputs)
            .wait_with_output()
            .expect("retry readback after the other recovery releases its mission lock");
    }
}

fn event_kinds(path: &Path) -> Vec<String> {
    EventJournal::open(path)
        .replay()
        .expect("verify durable mission journal")
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

fn assert_one_intent_and_one_resolution(path: &Path) {
    let kinds = event_kinds(path);
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_started_v1")
            .count(),
        1,
        "one attempt intent is durable across process restart"
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_resolved_v1")
            .count(),
        1,
        "one host resolution is durable"
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn killed_cli_after_forgejo_merge_recovers_readback_only_and_serializes_duplicate_retries() {
    let inputs = build_inputs();
    let fixture =
        ForgejoFixture::start(CrashPoint::AfterForgejoCommitsPost, inputs.journal.clone());
    let mut first = spawn_merge_cli(&inputs);
    assert_eq!(fixture.wait_notice(), FixtureNotice::MergePostCommitted);
    assert!(
        fixture.merged(),
        "Forgejo commits the merge before withholding its response"
    );
    assert!(journal_contains_started_intent(&inputs.journal));
    assert_eq!(kill_process_group(&mut first).signal(), Some(libc::SIGKILL));
    fixture.release_held_request();
    assert_eq!(
        event_kinds(&inputs.journal)
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_resolved_v1")
            .count(),
        0,
        "the killed process cannot publish its post-merge resolution"
    );

    let recovery_a = spawn_merge_cli(&inputs);
    let recovery_b = spawn_merge_cli(&inputs);
    let output_a = recovery_a
        .wait_with_output()
        .expect("collect first concurrent recovery");
    let output_b = recovery_b
        .wait_with_output()
        .expect("collect second concurrent recovery");
    let output_a = retry_recovery_after_mission_contention(&inputs, output_a);
    let output_b = retry_recovery_after_mission_contention(&inputs, output_b);
    assert!(
        output_a.status.success(),
        "{}",
        String::from_utf8_lossy(&output_a.stdout)
    );
    assert!(
        output_b.status.success(),
        "{}",
        String::from_utf8_lossy(&output_b.stdout)
    );
    let result_a = output_json(&output_a);
    let result_b = output_json(&output_b);
    for result in [&result_a, &result_b] {
        assert_eq!(result["verdict"], "merged");
        assert_eq!(result["merge_attempted"], false);
        assert_eq!(result["merge_attempt_recovery"], true);
        assert_eq!(result["merge_commit_sha"], MERGE_SHA);
    }
    let requests = fixture.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.starts_with("POST "))
            .count(),
        1,
        "the original process sent the only merge POST"
    );
    let post_index = requests
        .iter()
        .position(|line| line.starts_with("POST "))
        .expect("the original process sent its merge POST");
    assert!(
        requests[post_index + 1..]
            .iter()
            .all(|line| line.starts_with("GET ")),
        "every request after the committed POST is authenticated readback"
    );
    assert_one_intent_and_one_resolution(&inputs.journal);

    let before_duplicate = fixture.requests().len();
    let duplicate = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("collect resolved duplicate retry");
    assert!(duplicate.status.success());
    let duplicate_result = output_json(&duplicate);
    assert_eq!(duplicate_result["verdict"], "merged");
    assert_eq!(duplicate_result["merge_attempted"], false);
    assert_eq!(duplicate_result["merge_attempt_recovery"], true);
    assert_eq!(duplicate_result["merge_post_attempt_status"], "unknown");
    assert_eq!(
        fixture.requests().len(),
        before_duplicate,
        "a resolved duplicate replays the saved host decision without network writes or reads"
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn inconsistent_review_totals_stop_merge_authority_process_before_post() {
    let inputs = build_inputs();
    let fixture = ForgejoFixture::start(
        CrashPoint::ChangedReviewTotalsAcrossPages,
        inputs.journal.clone(),
    );
    let output = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("collect merge authority refusal on inconsistent pagination");
    assert!(!output.status.success());
    let result = output_json(&output);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["merge_authorized"], false);
    assert_eq!(result["merge_attempted"], false);
    assert_eq!(result["error"]["stage"], "review-list-read");
    assert_eq!(result["error"]["code"], "review_pagination_inconsistent");
    assert_eq!(result["error"]["retryable"], true);
    assert_eq!(
        result["error"]["action"],
        "discard all pages and reread the complete review list before retrying the merge preflight"
    );

    let requests = fixture.requests();
    assert!(requests.iter().any(|line| {
        line == "GET /api/v1/repos/forge-admin/mission-012-clamp/pulls/23/reviews?limit=50&page=1 HTTP/1.1"
    }));
    assert!(requests.iter().any(|line| {
        line == "GET /api/v1/repos/forge-admin/mission-012-clamp/pulls/23/reviews?limit=50&page=2 HTTP/1.1"
    }));
    assert!(
        requests.iter().all(|line| !line.starts_with("POST ")),
        "the authority must not issue any POST after an inconsistent review snapshot: {requests:?}"
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn unauthorized_second_review_page_stops_merge_before_authority() {
    let inputs = build_inputs();
    let fixture = ForgejoFixture::start(
        CrashPoint::UnauthorizedSecondReviewPage,
        inputs.journal.clone(),
    );
    let output = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("collect merge authority refusal on unauthorized review page");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = output_json(&output);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["merge_authorized"], false);
    assert_eq!(result["merge_attempted"], false);
    assert_eq!(result["error"]["stage"], "review-list-read");
    assert_eq!(result["error"]["code"], "upstream_unauthorized");
    assert_eq!(result["error"]["retryable"], false);
    assert!(!stdout.contains("private-review-401-marker"));
    assert!(!stdout.contains(std::str::from_utf8(HOST_TOKEN).expect("synthetic token is UTF-8")));

    let requests = fixture.requests();
    assert!(requests.iter().any(|line| {
        line == "GET /api/v1/repos/forge-admin/mission-012-clamp/pulls/23/reviews?limit=50&page=1 HTTP/1.1"
    }));
    assert!(requests.iter().any(|line| {
        line == "GET /api/v1/repos/forge-admin/mission-012-clamp/pulls/23/reviews?limit=50&page=2 HTTP/1.1"
    }));
    assert!(
        requests.iter().all(|line| !line.starts_with("POST ")),
        "the merge authority must not POST after a partial review snapshot: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .all(|line| !line.ends_with("/branch_protections HTTP/1.1")),
        "branch protection checks must not proceed after review collection fails"
    );
    assert!(!fixture.merged());
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn signed_webhook_reconciliation_preserves_the_merge_cli_readback_sha() {
    const DELIVERY_ID: &str = "merge-webhook-sha-bridge-001";

    let inputs = build_inputs();
    let fixture = ForgejoFixture::start(
        CrashPoint::AfterFinalProtectionReadBeforePost,
        inputs.journal.clone(),
    );
    let mut merge_process = spawn_merge_cli(&inputs);
    let notice = fixture
        .notice_receiver
        .recv_timeout(Duration::from_secs(20));
    if let Err(error) = notice {
        let status = merge_process
            .try_wait()
            .expect("check merge child after fixture timeout");
        if status.is_some() {
            let output = merge_process
                .wait_with_output()
                .expect("collect early merge child exit after fixture timeout");
            panic!(
                "merge child exited before reaching fixture boundary: {status:?}; stdout={}; stderr={}; fixture requests={:?}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                fixture.requests()
            );
        }
        panic!(
            "merge child stayed running without reaching fixture boundary: {error:?}; fixture requests={:?}",
            fixture.requests()
        );
    }
    assert_eq!(
        notice.expect("the merge child reached its fixture boundary"),
        FixtureNotice::FinalProtectionReadAfterIntent
    );
    fixture.release_held_request();
    let merge_output = merge_process
        .wait_with_output()
        .expect("collect the actual merge authority decision");
    assert!(
        merge_output.status.success(),
        "{}",
        String::from_utf8_lossy(&merge_output.stdout)
    );
    let merge_result = output_json(&merge_output);
    assert_eq!(merge_result["verdict"], "merged");
    assert_eq!(merge_result["merge_commit_sha"], MERGE_SHA);
    assert!(fixture.merged(), "the fixture applied the merge POST");

    let webhook_directory = ScratchDir::new();
    let secret = write_private(webhook_directory.path(), "webhook.secret", WEBHOOK_SECRET);
    let inbox = webhook_directory.path().join("inbox.jsonl");
    let body = serde_json::to_vec(&json!({
        "action": "closed",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "pull_request": {
            "number": PR_NUMBER,
            "state": "closed",
            "merged": true,
            "merge_commit_sha": MERGE_SHA,
            "head": {"sha": HEAD_SHA},
            "base": {"sha": MERGE_SHA}
        }
    }))
    .expect("serialize the signed merged pull request webhook");
    let (mut receiver, address) = start_ready_webhook_receiver(&secret, &inbox);
    let (status, accepted) = send_signed_webhook(address, DELIVERY_ID, "pull_request", &body);
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert!(stop_webhook_receiver(&mut receiver).success());
    let inbox_after_admission = fs::read(&inbox).expect("read durable webhook inbox");

    let reconcile_output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(["--target-profile", "mission-012-clamp"])
        .arg(&inbox)
        .arg(DELIVERY_ID)
        .stdin(Stdio::null())
        .output()
        .expect("run the actual webhook reconciliation CLI");
    assert!(
        reconcile_output.status.success(),
        "{}",
        String::from_utf8_lossy(&reconcile_output.stdout)
    );
    assert!(reconcile_output.stderr.is_empty());
    let reconciled: Value =
        serde_json::from_slice(&reconcile_output.stdout).expect("parse reconciliation decision");
    assert_eq!(reconciled["verdict"], "current_state_read");
    assert_eq!(reconciled["resource"]["kind"], "pull_request");
    assert_eq!(reconciled["canonical_state"]["state"], "closed");
    assert_eq!(reconciled["canonical_state"]["merged"], true);
    assert_eq!(
        reconciled["canonical_state"]["updated_at"],
        "2026-10-25T22:00:00Z"
    );
    assert_eq!(reconciled["canonical_state"]["merge_commit_sha"], MERGE_SHA);
    assert_eq!(
        reconciled["canonical_state"]["merge_commit_sha"], merge_result["merge_commit_sha"],
        "webhook reconciliation must expose the exact SHA returned by merge readback"
    );
    assert_eq!(
        fs::read(&inbox).expect("read inbox after reconciliation"),
        inbox_after_admission,
        "reconciliation is a read of current host state and does not rewrite the inbox"
    );

    let requests = fixture.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.starts_with("POST ") && line.contains("/pulls/23/merge"))
            .count(),
        1,
        "the bridge observes one controlled merge POST"
    );
    assert!(
        requests
            .iter()
            .any(|line| { line.starts_with("GET ") && line.contains("/pulls/23 HTTP/1.1") }),
        "the reconciler reads the merged PR from the same isolated Forgejo fixture"
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn corrected_issue_action_retries_same_delivery_then_reconciles_through_real_processes() {
    let directory = ScratchDir::new();
    let secret = write_private(directory.path(), "webhook.secret", WEBHOOK_SECRET);
    let inbox = directory.path().join("webhook-inbox.jsonl");
    let fixture = ForgejoFixture::start(
        CrashPoint::AfterFinalProtectionReadBeforePost,
        inbox.clone(),
    );
    let (mut receiver, address) = start_ready_webhook_receiver(&secret, &inbox);
    let repository = json!({"full_name": format!("{OWNER}/{REPOSITORY}")});
    let delivery_id = "issue-action-process-retry-001";
    let invalid_body = serde_json::to_vec(&json!({
        "repository": repository,
        "issue": {"number": ISSUE_NUMBER, "state": "open", "title": "stale event title"}
    }))
    .expect("serialize signed issue event without action");

    let (status, rejected) = send_signed_webhook(address, delivery_id, "issues", &invalid_body);
    assert_eq!(status, 400);
    assert_eq!(rejected["accepted"], false);
    assert_eq!(rejected["error"]["code"], "event_payload_invalid");
    assert_eq!(rejected["error"]["retryable"], false);
    let action = rejected["error"]["action"]
        .as_str()
        .expect("invalid event response has a repair action");
    assert!(action.contains("GET /forgejo/capabilities"));
    assert!(action.contains("redeliver with the same delivery ID"));
    assert!(action.contains("rejected body was not journaled"));
    assert!(
        EventJournal::open_webhook_inbox(&inbox)
            .replay()
            .expect("replay receiver journal after action refusal")
            .is_empty(),
        "an invalid delivery must not reserve its ID"
    );

    let corrected_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": format!("{OWNER}/{REPOSITORY}")},
        "issue": {"number": ISSUE_NUMBER, "state": "open", "title": "stale event title"}
    }))
    .expect("serialize corrected issue event with the same delivery ID");
    let (status, accepted) = send_signed_webhook(address, delivery_id, "issues", &corrected_body);
    assert_eq!(status, 202);
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["duplicate"], false);
    assert_eq!(accepted["sequence"], 1);

    let (status, duplicate) = send_signed_webhook(address, delivery_id, "issues", &corrected_body);
    assert_eq!(status, 202);
    assert_eq!(duplicate["accepted"], true);
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(duplicate["sequence"], 1);
    assert!(stop_webhook_receiver(&mut receiver).success());

    let journal_events = EventJournal::open_webhook_inbox(&inbox)
        .replay()
        .expect("replay receiver journal after correction and duplicate");
    assert_eq!(journal_events.len(), 1);
    assert_eq!(journal_events[0].payload["payload"]["action"], "opened");
    let inbox_before_reconcile = fs::read(&inbox).expect("read accepted receiver journal");

    let reconcile = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .args(["--target-profile", "mission-012-clamp"])
        .arg(&inbox)
        .arg(delivery_id)
        .stdin(Stdio::null())
        .output()
        .expect("launch the actual Forgejo reconciliation CLI");
    assert!(
        reconcile.status.success(),
        "{}",
        String::from_utf8_lossy(&reconcile.stdout)
    );
    assert!(reconcile.stderr.is_empty());
    let reconciled: Value =
        serde_json::from_slice(&reconcile.stdout).expect("parse process reconciliation result");
    assert_eq!(reconciled["verdict"], "current_state_read");
    assert_eq!(reconciled["resource"]["kind"], "issue");
    assert_eq!(reconciled["canonical_state"]["state"], "closed");
    assert_eq!(
        reconciled["canonical_state"]["updated_at"],
        "2026-10-27T13:40:00Z"
    );
    assert_eq!(
        reconciled["canonical_state"]["work_item_content"]["title"],
        "canonical title from Forgejo"
    );
    assert_eq!(
        reconciled["canonical_state"]["work_item_content"]["body"],
        "canonical body from Forgejo"
    );
    assert_eq!(
        fs::read(&inbox).expect("read inbox after current-state reconciliation"),
        inbox_before_reconcile,
        "reconciliation observes current Forgejo state without rewriting the journal"
    );
    let requests = fixture.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|line| {
                line.starts_with("GET ")
                    && line.contains(&format!("/issues/{ISSUE_NUMBER} HTTP/1.1"))
            })
            .count(),
        1,
        "the actual reconciler makes one current-issue read"
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn draft_pull_request_refuses_merge_authority_before_review_or_post() {
    let inputs = build_inputs();
    let fixture =
        ForgejoFixture::start_with_draft(CrashPoint::NoCrash, inputs.journal.clone(), true);
    let output = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("collect merge authority refusal for a draft pull request");
    assert!(!output.status.success());

    let result = output_json(&output);
    assert_eq!(result["verdict"], "rejected");
    assert_eq!(result["merge_authorized"], false);
    assert_eq!(result["merge_attempted"], false);
    assert_eq!(result["error"]["code"], "draft_pull_request");
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(
        result["error"]["action"],
        "resolve the draft state through the normal review decision"
    );
    assert!(
        !fixture.merged(),
        "the fixture must not observe a merge POST"
    );
    assert!(
        !journal_contains_started_intent(&inputs.journal),
        "a draft refusal must not durably start a merge attempt"
    );
    assert_eq!(
        fixture.requests(),
        vec![format!(
            "GET /api/v1/repos/{OWNER}/{REPOSITORY}/pulls/{PR_NUMBER} HTTP/1.1"
        )],
        "draft state is enough to refuse before actor, review, protection or base reads"
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn killed_cli_after_intent_before_post_keeps_open_pr_unresolved_without_posting() {
    let inputs = build_inputs();
    let fixture = ForgejoFixture::start(
        CrashPoint::AfterDurableIntentBeforePost,
        inputs.journal.clone(),
    );
    let mut first = spawn_merge_cli(&inputs);
    assert_eq!(fixture.wait_notice(), FixtureNotice::IntentSeenBeforePost);
    assert!(journal_contains_started_intent(&inputs.journal));
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|line| line.starts_with("POST "))
            .count(),
        0,
        "fixture pauses a GET after the fsynced intent and before any POST"
    );
    assert_eq!(kill_process_group(&mut first).signal(), Some(libc::SIGKILL));
    fixture.release_held_request();

    let output = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("collect open-PR readback recovery");
    assert!(
        !output.status.success(),
        "an open PR is not a merged success"
    );
    let result = output_json(&output);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["merge_attempted"], false);
    assert_eq!(result["merge_attempt_recovery"], true);
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|line| line.starts_with("POST "))
            .count(),
        0,
        "recovery of an unresolved intent never reissues the merge POST"
    );
    assert_eq!(
        event_kinds(&inputs.journal)
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_started_v1")
            .count(),
        1
    );
    assert_eq!(
        event_kinds(&inputs.journal)
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_resolved_v1")
            .count(),
        0,
        "an open PR is a temporary inconclusive observation, not a terminal decision"
    );
    let request_count_after_first_recovery = fixture.requests().len();
    let retry = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("collect retried open-PR readback");
    assert!(!retry.status.success());
    assert_eq!(output_json(&retry)["verdict"], "inconclusive");
    assert!(fixture.requests().len() > request_count_after_first_recovery);
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|line| line.starts_with("POST "))
            .count(),
        0,
        "retrying an unresolved open-PR observation never issues a merge POST"
    );
    assert_eq!(
        event_kinds(&inputs.journal)
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_resolved_v1")
            .count(),
        0
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn protected_host_input_change_after_final_forgejo_read_refuses_merge_post() {
    let inputs = build_inputs();
    let fixture = ForgejoFixture::start(
        CrashPoint::AfterFinalProtectionReadBeforePost,
        inputs.journal.clone(),
    );
    let child = spawn_merge_cli(&inputs);
    assert_eq!(
        fixture.wait_notice(),
        FixtureNotice::FinalProtectionReadAfterIntent
    );

    let mut policy = OpenOptions::new()
        .append(true)
        .open(&inputs.policy)
        .expect("open the test policy while the final upstream read is held");
    policy
        .write_all(b" \n")
        .expect("change protected policy bytes after preflight");
    policy.sync_all().expect("persist the test policy change");
    drop(policy);
    fixture.release_held_request();

    let output = child
        .wait_with_output()
        .expect("collect merge refusal after protected input changes");
    let result = output_json(&output);
    assert!(
        !output.status.success(),
        "stale host policy must refuse merge"
    );
    assert_eq!(result["verdict"], "rejected");
    assert_eq!(result["merge_attempted"], false);
    assert_eq!(result["error"]["code"], "merge_attempt_binding_mismatch");
    assert!(
        fixture
            .requests()
            .iter()
            .all(|line| !line.starts_with("POST ")),
        "no merge POST follows a change to host-owned authorization inputs"
    );
    assert_one_intent_and_one_resolution(&inputs.journal);
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn unresolved_merge_on_same_base_refuses_another_pr_before_forgejo_reads() {
    let inputs = build_inputs();
    let admission: MissionHostAdmissionV1 = serde_json::from_slice(
        &fs::read(&inputs.admission).expect("read admitted mission fixture"),
    )
    .expect("parse admitted mission fixture");
    let scope = BaseBranchScope::new(ORIGIN, OWNER, REPOSITORY, "main")
        .expect("construct exact branch scope");
    let blocker_issue = ISSUE_NUMBER + 1;
    let blocker_pr = PR_NUMBER + 1;
    let mut blocker_requirements: Value =
        serde_json::from_slice(REQUIREMENTS).expect("parse protected requirement fixture");
    blocker_requirements["source_issue_number"] = json!(blocker_issue);
    let blocker_requirement_bytes =
        serde_json::to_vec(&blocker_requirements).expect("serialize blocker requirement");
    let blocker_requirement_sha256 = sha256(&blocker_requirement_bytes);
    let journal = MissionExecutionJournalV1::open(&inputs.journal);
    let allocation = journal
        .allocate_generation(
            &scope,
            blocker_issue,
            &blocker_requirement_sha256,
            "branch-scope-blocker-001",
        )
        .expect("allocate a second mission on the same base branch");
    let identity = derive_mission_identity_v1(
        &scope,
        blocker_issue,
        &blocker_requirement_sha256,
        allocation.execution_generation,
    )
    .expect("derive blocker mission identity");
    let mut blocker_request = admission.request.clone();
    blocker_request.requirement_sha256 = blocker_requirement_sha256.clone();
    blocker_request.requirements_json =
        String::from_utf8(blocker_requirement_bytes).expect("blocker requirement is UTF-8");
    blocker_request.context.manifest.requirement_sha256 = blocker_requirement_sha256.clone();
    blocker_request.mission_id = identity.mission_id.clone();
    blocker_request.idempotency_key = identity.idempotency_key;
    journal
        .claim_allocated(
            &scope,
            blocker_issue,
            allocation.execution_generation,
            &allocation.allocation_id,
            &blocker_request,
        )
        .expect("durably claim the blocker mission");

    let blocker_admission = MissionHostAdmissionV1 {
        schema_version: admission.schema_version,
        scope: MissionHostScopeV1 {
            origin: ORIGIN.to_owned(),
            owner: OWNER.to_owned(),
            repository: REPOSITORY.to_owned(),
            branch: "main".to_owned(),
        },
        issue_number: blocker_issue,
        base_commit: admission.base_commit.clone(),
        requirement_path: admission.requirement_path.clone(),
        requirement_sha256: blocker_requirement_sha256.clone(),
        allocation: MissionHostAllocationV1 {
            schema_version: allocation.schema_version,
            mission_id: allocation.mission_id,
            allocation_id: allocation.allocation_id,
            requirement_sha256: allocation.requirement_sha256,
            execution_generation: allocation.execution_generation,
        },
        request: blocker_request.clone(),
    };
    blocker_admission
        .validate()
        .expect("blocker admission is valid");
    let blocker_admission_sha256 =
        sha256(&serde_json::to_vec(&blocker_admission).expect("serialize blocker admission"));
    let blocker_request_sha256 =
        sha256(&serde_json::to_vec(&blocker_request).expect("serialize blocker request"));
    let policy_sha256 = sha256(&fs::read(&inputs.policy).expect("read private policy fixture"));
    let proof_sha256 = sha256(&fs::read(&inputs.proof).expect("read proof fixture"));
    let review_sha256 = sha256(&fs::read(&inputs.review).expect("read review fixture"));
    let binding = MergeAttemptBindingV1::new(
        &scope,
        blocker_issue,
        blocker_pr,
        BASE_SHA,
        HEAD_SHA,
        &blocker_requirement_sha256,
        &policy_sha256,
        &proof_sha256,
        &review_sha256,
        &blocker_request_sha256,
        &blocker_admission_sha256,
    )
    .expect("bind the unresolved blocker to a different PR in the same scope");
    let unresolved = journal
        .with_merge_attempt(
            &scope,
            blocker_issue,
            &blocker_admission.allocation.allocation_id,
            blocker_admission.allocation.execution_generation,
            &blocker_request,
            binding,
            |_, _| Ok(()),
            |_, _, ()| {
                Ok(MergeAttemptResolutionV1::inconclusive(
                    "fixture_interrupted_before_merge_readback",
                ))
            },
        )
        .expect("persist the simulated unresolved blocker");
    assert_eq!(
        unresolved.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Inconclusive
    );

    let fixture = ForgejoFixture::start(
        CrashPoint::AfterFinalProtectionReadBeforePost,
        inputs.journal.clone(),
    );
    let output = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("collect refusal from the real merge CLI");
    assert!(!output.status.success());
    let result = output_json(&output);
    assert_eq!(
        result["error"]["code"], "execution_merge_target_unresolved",
        "unexpected CLI result: {result}"
    );
    assert_eq!(result["verdict"], "inconclusive", "result={result}");
    assert_eq!(result["error"]["retryable"], true);
    assert_eq!(result["merge_attempted"], false);
    assert_eq!(result["error"]["details"]["pull_request_number"], PR_NUMBER);
    assert_eq!(
        result["error"]["details"]["blocking_pull_request_number"],
        blocker_pr
    );
    assert!(
        result["error"]["action"]
            .as_str()
            .is_some_and(|action| action.contains("authenticated readback"))
    );
    assert!(fixture.requests().is_empty(), "no Forgejo API read is sent");
    assert!(!fixture.merged(), "no Forgejo merge is applied");
    assert_eq!(
        event_kinds(&inputs.journal)
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_started_v1")
            .count(),
        1
    );
    assert_eq!(
        event_kinds(&inputs.journal)
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_resolved_v1")
            .count(),
        0,
        "the refusal does not falsely resolve the other mission's intent"
    );
}

#[test]
#[ignore = "run only through the bwrap network-isolated process E2E runner"]
fn external_base_advance_during_merge_post_is_fail_closed_but_still_mutates_branch() {
    let inputs = build_inputs();
    let fixture = ForgejoFixture::start(
        CrashPoint::BaseAdvancesAfterFinalCheckBeforePost,
        inputs.journal.clone(),
    );
    let child = spawn_merge_cli(&inputs);
    assert_eq!(
        fixture.wait_notice(),
        FixtureNotice::FinalProtectionReadAfterIntent
    );
    fixture.release_held_request();
    assert_eq!(
        fixture.wait_notice(),
        FixtureNotice::MergePostReceivedBeforeBaseAdvance
    );

    // The final branch SHA read has already passed. Model an external Forgejo
    // writer advancing main before the server applies this merge request.
    fixture.advance_main(ADVANCED_BASE_SHA);
    fixture.release_held_request();

    let output = child
        .wait_with_output()
        .expect("collect the real CLI result after the concurrent base advance");
    let result = output_json(&output);
    assert!(
        fixture.merged(),
        "the non-conditional endpoint applies the request after the external base advance"
    );
    assert!(
        !output.status.success(),
        "a merge based on an unvalidated parent must not be reported as success"
    );
    assert_eq!(fixture.merge_parent(), ADVANCED_BASE_SHA);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["merge_attempted"], true);
    assert_eq!(result["error"]["code"], "merge_outcome_not_confirmed");

    let requests = fixture.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.starts_with("POST ") && line.ends_with("/pulls/23/merge HTTP/1.1"))
            .count(),
        1,
        "the client submits one merge request and never retries it"
    );
    let recovery = spawn_merge_cli(&inputs)
        .wait_with_output()
        .expect("read back the raced merge without retrying its POST");
    assert!(!recovery.status.success());
    let recovered = output_json(&recovery);
    assert_eq!(recovered["verdict"], "inconclusive");
    assert_eq!(recovered["merge_attempt_recovery"], true);
    assert_eq!(recovered["merge_attempted"], false);
    assert_eq!(recovered["error"]["code"], "merge_outcome_not_confirmed");

    let requests = fixture.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.starts_with("POST ") && line.ends_with("/pulls/23/merge HTTP/1.1"))
            .count(),
        1,
        "readback recovery does not resubmit the merge"
    );
    let kinds = event_kinds(&inputs.journal);
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_started_v1")
            .count(),
        1
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| kind.as_str() == "mission_merge_attempt_resolved_v1")
            .count(),
        0,
        "the changed parent stays unresolved instead of becoming a false success"
    );
}
