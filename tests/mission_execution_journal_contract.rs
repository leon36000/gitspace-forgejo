use agent_forge_core::{
    ContextManifest, ContextSource, EventJournal,
    agent_runtime::{
        AgentExecutionRequestV1, ContextCitationV1, ContextPacketV1, ExecutionClaimResultV1,
        MergeAttemptBindingV1, MergeAttemptModeV1, MergeAttemptResolutionV1,
        MissionExecutionJournalV1, MissionGenerationAllocationV1, RuntimeBudgetV1,
        RuntimeCapabilitiesV1, WorkspaceRefV1, derive_mission_identity_v1,
    },
    forgejo::BaseBranchScope,
    webhook::ingest_forgejo_delivery,
};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::os::{
    fd::AsRawFd,
    unix::fs::{MetadataExt, PermissionsExt},
    unix::process::ExitStatusExt,
};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type HmacSha256 = Hmac<Sha256>;

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mission-execution-ledger-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn journal_lock_is_contended(journal_path: &Path) -> bool {
    let mut lock_path = journal_path.as_os_str().to_owned();
    lock_path.push(".lock");
    lock_file_is_contended(&PathBuf::from(lock_path))
}

fn mission_lock_path_for(journal_path: &Path, mission_id: &str) -> PathBuf {
    let mut directory = journal_path.as_os_str().to_owned();
    directory.push(".mission-locks");
    PathBuf::from(directory).join(format!("{mission_id}.lock"))
}

fn lock_file_is_contended(lock_path: &Path) -> bool {
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .unwrap();
    let result = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        unsafe {
            libc::flock(lock_file.as_raw_fd(), libc::LOCK_UN);
        }
        false
    } else {
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK)
    }
}

/// Inject a hash-chain-valid but semantically malformed event, as if it had
/// been written by an older or unauthorized producer. This deliberately bypasses
/// EventJournal's public write API for corruption-recovery contract tests.
fn append_raw_event(path: &Path, idempotency_key: &str, kind: &str, payload: serde_json::Value) {
    let events = EventJournal::open(path).replay().unwrap();
    let sequence = events.len() as u64 + 1;
    let previous_hash = events
        .last()
        .map(|event| event.hash.clone())
        .unwrap_or_else(|| "0".repeat(64));
    let unsigned = serde_json::json!({
        "sequence": sequence,
        "event_id": format!("evt-{sequence}"),
        "idempotency_key": idempotency_key,
        "kind": kind,
        "payload": payload,
        "previous_hash": previous_hash,
    });
    let hash = sha256(&serde_json::to_vec(&unsigned).unwrap());
    let record = serde_json::json!({
        "sequence": sequence,
        "event_id": format!("evt-{sequence}"),
        "idempotency_key": idempotency_key,
        "kind": kind,
        "payload": payload,
        "previous_hash": previous_hash,
        "hash": hash,
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    serde_json::to_writer(&mut file, &record).unwrap();
    file.write_all(b"\n").unwrap();
}

fn scope() -> BaseBranchScope {
    BaseBranchScope::new(
        "http://127.0.0.1:3130",
        "forge-admin",
        "agent-forge-core",
        "main",
    )
    .unwrap()
}

const REQUIREMENTS_JSON: &str = r#"{"schema_version":2,"id":"REQ-CLAIM-001","revision":1,"source_issue_number":41,"request":"Compile a small Rust program using the cited context.","acceptance":["The program compiles."],"ambiguities":[]}"#;

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn request(generation: u64) -> AgentExecutionRequestV1 {
    request_for_issue(41, generation)
}

fn request_for_issue(issue_number: u64, generation: u64) -> AgentExecutionRequestV1 {
    let mut requirements: serde_json::Value = serde_json::from_str(REQUIREMENTS_JSON).unwrap();
    requirements["source_issue_number"] = json!(issue_number);
    let requirements_json = serde_json::to_string(&requirements).unwrap();
    let requirement_sha256 = sha256(requirements_json.as_bytes());
    let identity =
        derive_mission_identity_v1(&scope(), issue_number, &requirement_sha256, generation)
            .unwrap();
    let base_commit = "c".repeat(40);
    let context_text = "read src/lib.rs:1".to_owned();
    AgentExecutionRequestV1 {
        schema_version: 1,
        mission_id: identity.mission_id,
        idempotency_key: identity.idempotency_key,
        requirement_sha256: requirement_sha256.clone(),
        requirements_json,
        untrusted_work_item: None,
        requirement_arbitration: None,
        context: ContextPacketV1 {
            manifest: ContextManifest {
                schema_version: 1,
                source_commit: base_commit.clone(),
                requirement_sha256,
                token_budget: 100,
                estimated_tokens: 10,
                sources: vec![ContextSource {
                    path: "src/lib.rs".to_owned(),
                    line_start: 1,
                    line_end: 1,
                    file_sha256: "b".repeat(64),
                    provenance: format!("git blob at {base_commit}"),
                }],
            },
            citations: vec![ContextCitationV1 {
                text_start_byte: 0,
                text_end_byte: context_text.len(),
                source_index: 0,
            }],
            text: context_text,
        },
        workspace: WorkspaceRefV1 {
            id: "claim-workspace".to_owned(),
            base_commit,
        },
        capabilities: RuntimeCapabilitiesV1 {
            read_paths: vec!["src/lib.rs".to_owned()],
            write_paths: vec!["main.rs".to_owned()],
            tools: vec!["workspace.write_text".to_owned()],
            budget: RuntimeBudgetV1 {
                max_tool_calls: 2,
                timeout_ms: 10_000,
                max_output_tokens: 128,
            },
        },
    }
}

fn claim(
    journal_path: &Path,
    generation: u64,
    request: &AgentExecutionRequestV1,
) -> Result<ExecutionClaimResultV1, agent_forge_core::agent_runtime::AgentRuntimeErrorV1> {
    let journal = MissionExecutionJournalV1::open(journal_path);
    let allocation_id = format!("claim-contract-{generation}");
    let allocation = retry_mission_busy(|| {
        journal.allocate_generation(&scope(), 41, &request.requirement_sha256, &allocation_id)
    })?;
    if allocation.execution_generation != generation {
        return Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
            "test_generation_mismatch",
            false,
            "allocate generations in order in this journal contract helper",
            serde_json::json!({"requested":generation,"allocated":allocation.execution_generation}),
        ));
    }
    retry_mission_busy(|| {
        journal.claim_allocated(&scope(), 41, generation, &allocation.allocation_id, request)
    })
}

fn retry_mission_busy<T>(
    mut operation: impl FnMut() -> Result<T, agent_forge_core::agent_runtime::AgentRuntimeErrorV1>,
) -> Result<T, agent_forge_core::agent_runtime::AgentRuntimeErrorV1> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match operation() {
            Err(error) if error.code == "execution_mission_busy" && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(1));
            }
            result => return result,
        }
    }
}

fn allocation_and_claim(
    journal_path: &Path,
) -> (
    MissionExecutionJournalV1,
    MissionGenerationAllocationV1,
    AgentExecutionRequestV1,
) {
    allocation_and_claim_for_issue(journal_path, 41)
}

fn allocation_and_claim_for_issue(
    journal_path: &Path,
    issue_number: u64,
) -> (
    MissionExecutionJournalV1,
    MissionGenerationAllocationV1,
    AgentExecutionRequestV1,
) {
    let journal = MissionExecutionJournalV1::open(journal_path);
    let request = request_for_issue(issue_number, 1);
    let allocation = retry_mission_busy(|| {
        journal.allocate_generation(
            &scope(),
            issue_number,
            &request.requirement_sha256,
            &format!("lease-admission-{issue_number}"),
        )
    })
    .unwrap();
    retry_mission_busy(|| {
        journal.claim_allocated(
            &scope(),
            issue_number,
            allocation.execution_generation,
            &allocation.allocation_id,
            &request,
        )
    })
    .unwrap();
    (journal, allocation, request)
}

fn lease_for_workspace_write(
    journal_path: &Path,
    issue_number: u64,
) -> (
    MissionExecutionJournalV1,
    MissionGenerationAllocationV1,
    AgentExecutionRequestV1,
    agent_forge_core::agent_runtime::MissionExecutionLeaseV1,
) {
    let (journal, allocation, request) = allocation_and_claim_for_issue(journal_path, issue_number);
    let lease = retry_mission_busy(|| {
        journal.acquire_lease(
            &scope(),
            issue_number,
            allocation.execution_generation,
            &allocation.allocation_id,
            "workspace-write-lease-1",
            "worker-a",
            || 1_000,
            5_000,
            &request,
        )
    })
    .unwrap();
    (journal, allocation, request, lease)
}

fn receive_one_of(
    receiver: &mpsc::Receiver<String>,
    expected: &[&str],
    timeout: Duration,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = receiver.recv_timeout(remaining).unwrap();
        if expected.contains(&line.as_str()) {
            return line;
        }
    }
}

#[test]
fn first_claim_is_admitted_and_reopen_is_idempotent() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);

    let first = claim(&path, 1, &request).unwrap();
    let replay = claim(&path, 1, &request).unwrap();

    assert!(matches!(first, ExecutionClaimResultV1::Claimed(_)));
    assert!(matches!(replay, ExecutionClaimResultV1::AlreadyClaimed(_)));
    let events = MissionExecutionJournalV1::open(&path).replay().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, "mission_generation_allocated_v1");
    assert_eq!(events[1].kind, "mission_execution_claim_v1");
    assert_eq!(
        events[1].payload["request_sha256"],
        sha256(&serde_json::to_vec(&request).unwrap())
    );
}

#[test]
fn merge_authority_can_verify_the_exact_claimed_request() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);

    let verified = journal
        .verify_claimed_request(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
        )
        .expect("the exact durable allocation and claim should verify");

    assert_eq!(verified.mission_id, request.mission_id);
    assert_eq!(verified.allocation_id, allocation.allocation_id);
    assert_eq!(
        verified.execution_generation,
        allocation.execution_generation
    );
    assert_eq!(
        verified.request_sha256,
        sha256(&serde_json::to_vec(&request).unwrap())
    );
}

#[test]
fn merge_effect_holds_mission_lock_against_new_generation_allocation() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let competing_writer = MissionExecutionJournalV1::open(&path);

    let effect_result = journal
        .with_current_claimed_request(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            |verified| {
                assert_eq!(
                    verified.execution_generation,
                    allocation.execution_generation
                );
                let error = competing_writer
                    .allocate_generation(
                        &scope(),
                        41,
                        &request.requirement_sha256,
                        "allocation-during-merge-effect",
                    )
                    .expect_err("a newer generation must not overtake the protected merge effect");
                assert_eq!(error.code, "execution_mission_busy");
                Ok(verified.clone())
            },
        )
        .expect("the exact current claim should authorize its bounded effect");

    assert_eq!(
        effect_result.execution_generation,
        allocation.execution_generation
    );
    let newer = competing_writer
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "allocation-after-merge-effect",
        )
        .expect("the lock must be released after the effect completes");
    assert_eq!(
        newer.execution_generation,
        allocation.execution_generation + 1
    );
}

#[test]
fn merge_authority_rejects_a_different_request_for_the_claimed_generation() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, mut request) = allocation_and_claim(&path);
    request.workspace.id = "other-workspace".into();

    let error = journal
        .verify_claimed_request(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
        )
        .expect_err("a claim must not authorize changed request bytes");

    assert_eq!(error.code, "execution_claim_request_mismatch");
}

#[test]
fn merge_authority_rejects_a_superseded_claim() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "newer-host-allocation",
        )
        .expect("a newer host generation can supersede the existing claim");

    let error = journal
        .verify_claimed_request(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
        )
        .expect_err("a stale claim must not authorize a merge");

    assert_eq!(error.code, "execution_claim_superseded");
}

#[test]
fn merge_effect_rejects_a_superseded_claim_without_running_the_effect() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "newer-host-allocation-before-effect",
        )
        .expect("a newer host generation can supersede the existing claim");
    let mut effect_called = false;

    let error = journal
        .with_current_claimed_request(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            |_| {
                effect_called = true;
                Ok(())
            },
        )
        .expect_err("a stale claim must not authorize its effect");

    assert_eq!(error.code, "execution_claim_superseded");
    assert!(
        !effect_called,
        "the protected effect must not run on a stale claim"
    );
}

#[test]
fn merge_authority_refuses_an_unterminated_journal_without_repairing_it() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let original = std::fs::read(&path).unwrap();
    assert_eq!(original.last(), Some(&b'\n'));
    let truncated = &original[..original.len() - 1];
    std::fs::write(&path, truncated).unwrap();

    let before = std::fs::read(&path).unwrap();
    let error = journal
        .verify_claimed_request(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
        )
        .expect_err("the merge authority must not repair an incomplete journal tail");

    assert_eq!(error.code, "execution_journal_corrupt");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn direct_claim_cannot_bypass_the_requirement_issue_binding() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let journal = MissionExecutionJournalV1::open(&path);
    let mut request = request(1);
    let mut requirements: serde_json::Value =
        serde_json::from_str(&request.requirements_json).unwrap();
    requirements["source_issue_number"] = json!(42);
    request.requirements_json = serde_json::to_string(&requirements).unwrap();
    request.requirement_sha256 = sha256(request.requirements_json.as_bytes());
    request.context.manifest.requirement_sha256 = request.requirement_sha256.clone();
    let identity =
        derive_mission_identity_v1(&scope(), 41, &request.requirement_sha256, 1).unwrap();
    request.mission_id = identity.mission_id;
    request.idempotency_key = identity.idempotency_key;

    let allocation = journal
        .allocate_generation(&scope(), 41, &request.requirement_sha256, "mismatch-source")
        .unwrap();
    let error = journal
        .claim_allocated(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            &request,
        )
        .unwrap_err();

    assert_eq!(error.code, "mission_requirement_issue_mismatch");
    assert_eq!(journal.replay().unwrap().len(), 1);
}

#[test]
fn lease_rejects_a_requirement_linked_to_another_issue_before_append() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let journal = MissionExecutionJournalV1::open(&path);
    let mut request = request(1);
    let allocation = journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "lease-link-check",
        )
        .unwrap();

    let mut requirements: serde_json::Value =
        serde_json::from_str(&request.requirements_json).unwrap();
    requirements["source_issue_number"] = json!(42);
    request.requirements_json = serde_json::to_string(&requirements).unwrap();
    request.requirement_sha256 = sha256(request.requirements_json.as_bytes());
    request.context.manifest.requirement_sha256 = request.requirement_sha256.clone();
    let identity =
        derive_mission_identity_v1(&scope(), 41, &request.requirement_sha256, 1).unwrap();
    request.mission_id = identity.mission_id;
    request.idempotency_key = identity.idempotency_key;

    let error = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-link-check-01",
            "worker-a",
            || 1_000,
            100,
            &request,
        )
        .unwrap_err();

    assert_eq!(error.code, "mission_requirement_issue_mismatch");
    assert_eq!(journal.replay().unwrap().len(), 1);
}

#[test]
fn one_generation_cannot_be_reused_with_changed_request_bytes() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let first = request(1);
    claim(&path, 1, &first).unwrap();
    let mut changed = first;
    changed.capabilities.budget.max_output_tokens += 1;

    let error = claim(&path, 1, &changed).unwrap_err();

    assert_eq!(error.code, "execution_idempotency_conflict");
    assert!(!error.retryable);
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn host_supplied_new_generation_creates_a_separate_claim() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let first = request(1);
    let second = request(2);
    let first_result = claim(&path, 1, &first).unwrap();
    let second_result = claim(&path, 2, &second).unwrap();

    assert!(matches!(first_result, ExecutionClaimResultV1::Claimed(_)));
    assert!(matches!(second_result, ExecutionClaimResultV1::Claimed(_)));
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn generation_allocation_is_monotonic_and_replayable_after_reopen() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let requirement_sha256 = request(1).requirement_sha256;
    let journal = MissionExecutionJournalV1::open(&path);

    let first = journal
        .allocate_generation(&scope(), 41, &requirement_sha256, "admission-01")
        .unwrap();
    let replay = MissionExecutionJournalV1::open(&path)
        .allocate_generation(&scope(), 41, &requirement_sha256, "admission-01")
        .unwrap();
    let second = MissionExecutionJournalV1::open(&path)
        .allocate_generation(&scope(), 41, &requirement_sha256, "admission-02")
        .unwrap();

    assert_eq!(first.execution_generation, 1);
    assert_eq!(first.mission_id, replay.mission_id);
    assert_eq!(first.execution_generation, replay.execution_generation);
    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(second.execution_generation, 2);
    assert!(!second.replayed);
    assert_eq!(journal.replay().unwrap().len(), 2);
}

#[test]
fn allocation_reservation_recovers_after_process_termination_before_first_claim() {
    const JOURNAL_ENV: &str = "AGENT_FORGE_ALLOCATION_CRASH_TEST_JOURNAL";
    const READY_ENV: &str = "AGENT_FORGE_ALLOCATION_CRASH_TEST_READY";

    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let ready_path = scratch.path().join("allocation-ready");
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("allocation_child_pauses_after_durable_append")
        .arg("--ignored")
        .arg("--nocapture")
        .env(JOURNAL_ENV, &path)
        .env(READY_ENV, &ready_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut child = KillOnDrop(child);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if ready_path.exists() {
            break;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("allocation subprocess exited before its durable-append marker: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "allocation subprocess did not reach the pre-claim interruption point"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&ready_path).unwrap(),
        "generation=1\n"
    );

    let events_before_termination = EventJournal::open(&path).replay().unwrap();
    assert_eq!(events_before_termination.len(), 1);
    assert_eq!(
        events_before_termination[0].kind,
        "mission_generation_allocated_v1"
    );

    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));

    let request = request(1);
    let reopened = MissionExecutionJournalV1::open(&path);
    let allocation = reopened
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "interrupted-admission",
        )
        .unwrap();
    assert_eq!(allocation.execution_generation, 1);
    assert!(allocation.replayed);

    assert!(matches!(
        reopened.claim_allocated(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            &request,
        ),
        Ok(ExecutionClaimResultV1::Claimed(_))
    ));
    let events_after_claim = reopened.replay().unwrap();
    assert_eq!(events_after_claim.len(), 2);
    assert_eq!(
        events_after_claim[0].kind,
        "mission_generation_allocated_v1"
    );
    assert_eq!(events_after_claim[1].kind, "mission_execution_claim_v1");

    assert!(matches!(
        MissionExecutionJournalV1::open(&path).claim_allocated(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            &request,
        ),
        Ok(ExecutionClaimResultV1::AlreadyClaimed(_))
    ));
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        2
    );
}

#[test]
#[ignore = "spawned by allocation_reservation_recovers_after_process_termination_before_first_claim"]
fn allocation_child_pauses_after_durable_append() {
    let Ok(journal_path) = std::env::var("AGENT_FORGE_ALLOCATION_CRASH_TEST_JOURNAL") else {
        return;
    };
    let ready_path = std::env::var("AGENT_FORGE_ALLOCATION_CRASH_TEST_READY").unwrap();
    let request = request(1);
    MissionExecutionJournalV1::open(&journal_path)
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "interrupted-admission",
        )
        .unwrap();

    let mut marker = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(ready_path)
        .unwrap();
    marker.write_all(b"generation=1\n").unwrap();
    marker.sync_all().unwrap();
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[test]
fn generation_allocator_advances_after_an_allocated_claim() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);
    claim(&path, 1, &request).unwrap();

    let allocation = MissionExecutionJournalV1::open(&path)
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "after-legacy-claim",
        )
        .unwrap();

    assert_eq!(allocation.execution_generation, 2);
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn legacy_claim_cannot_consume_a_generation_reserved_by_the_allocator() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);
    let journal = MissionExecutionJournalV1::open(&path);
    journal
        .allocate_generation(&scope(), 41, &request.requirement_sha256, "reserved-for-v2")
        .unwrap();

    let error = journal
        .claim_allocated(&scope(), 41, 1, "wrong-allocation", &request)
        .unwrap_err();

    assert_eq!(error.code, "execution_generation_reserved");
    assert_eq!(journal.replay().unwrap().len(), 1);
}

#[test]
fn allocation_replay_validates_later_records_for_the_same_mission() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);
    let journal = MissionExecutionJournalV1::open(&path);
    journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "first-allocation",
        )
        .unwrap();
    append_raw_event(
        &path,
        &format!("mission-generation-v1:{}:2", request.mission_id),
        "mission_generation_allocated_v1",
        serde_json::json!({
            "schema_version": 1,
            "mission_id": request.mission_id,
            "allocation_id": "damaged-later-record",
            "requirement_sha256": request.requirement_sha256,
            "execution_generation": "two",
        }),
    );

    let error = journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "first-allocation",
        )
        .unwrap_err();

    assert_eq!(error.code, "execution_generation_journal_invalid");
}

#[test]
fn claim_requires_a_durable_allocation() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let journal = MissionExecutionJournalV1::open(&path);
    let request = request(1);

    let error = journal
        .claim_allocated(&scope(), 41, 1, "not-reserved", &request)
        .unwrap_err();

    assert_eq!(error.code, "execution_allocation_missing");
    assert!(journal.replay().unwrap().is_empty());
}

#[test]
fn execution_lease_requires_a_matching_allocated_claim() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let initial_request = request(1);
    let journal = MissionExecutionJournalV1::open(&path);
    let allocation = journal
        .allocate_generation(
            &scope(),
            41,
            &initial_request.requirement_sha256,
            "lease-without-claim",
        )
        .unwrap();

    let future_request = request(2);
    let events_before_future_lease = journal.replay().unwrap().len();
    let future_generation_error = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation + 1,
            "future-allocation-id",
            "future-lease-id",
            "worker-future",
            || 1_000,
            100,
            &future_request,
        )
        .unwrap_err();

    assert_eq!(future_generation_error.code, "execution_allocation_missing");
    assert_eq!(journal.replay().unwrap().len(), events_before_future_lease);

    let error = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-01",
            "worker-a",
            || 1_000,
            100,
            &initial_request,
        )
        .unwrap_err();

    assert_eq!(error.code, "execution_claim_missing");
    assert_eq!(journal.replay().unwrap().len(), 1);
}

#[test]
fn malformed_lease_record_blocks_further_admission() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    append_raw_event(
        &path,
        "malformed-lease-record",
        "mission_execution_lease_acquired_v1",
        serde_json::json!({"schema_version": 1}),
    );

    let error = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-after-malformed-record",
            "worker-a",
            || 1_000,
            100,
            &request,
        )
        .unwrap_err();

    assert_eq!(error.code, "execution_generation_journal_invalid");
    assert_eq!(journal.replay().unwrap().len(), 3);
}

#[test]
fn workspace_write_intent_precedes_and_binds_the_exact_result() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, _request, lease) = lease_for_workspace_write(&path, 41);
    let request_id = json!("write-call-1");
    let input = json!({"path":"src/lib.rs","content":"fn fixed() -> i32 { 10 }\n"});
    let before_sha256 = sha256(b"fn broken() -> i32 { -2 }\n");
    let before_sha256_upper = before_sha256.to_ascii_uppercase();

    let intent = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_001,
            request_id.clone(),
            input.clone(),
            &before_sha256_upper,
        )
        .expect("the host should durably record an authorized write intent");

    assert!(!intent.replayed);
    assert_eq!(intent.execution_generation, allocation.execution_generation);
    assert_eq!(intent.request_id, request_id);
    assert_eq!(intent.input, input);
    assert_eq!(intent.before_source_sha256, before_sha256);
    assert_eq!(
        intent.after_source_sha256,
        sha256(b"fn fixed() -> i32 { 10 }\n")
    );
    let events = journal.replay().unwrap();
    assert_eq!(
        events.last().unwrap().kind,
        "mission_tool_operation_intent_v1"
    );
    assert_eq!(
        events.last().unwrap().payload["input"]["content"],
        "fn fixed() -> i32 { 10 }\n"
    );

    let result_body = json!({
        "content":[{"type":"text","text":"{\"written\":true,\"path\":\"src/lib.rs\"}"}],
        "isError":false
    });
    let after_sha256_upper = intent.after_source_sha256.to_ascii_uppercase();
    let result = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_002,
            &intent.operation_id,
            &after_sha256_upper,
            result_body.clone(),
        )
        .expect("the host should record the observed effect before replying");

    assert!(!result.replayed);
    assert_eq!(result.result_body, result_body);
    assert_eq!(
        result.result_sha256,
        sha256(&serde_json::to_vec(&result_body).unwrap())
    );
    let events = journal.replay().unwrap();
    assert_eq!(
        events.last().unwrap().kind,
        "mission_tool_operation_result_v1"
    );

    let status = journal
        .workspace_write_operation_status(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_003,
            &json!({"path":"src/lib.rs","content":"fn fixed() -> i32 { 10 }\n"}),
            &intent.after_source_sha256,
        )
        .unwrap()
        .expect("the completed operation should be readable by its current lease");
    assert_eq!(status.result.as_ref().unwrap().result_body, result_body);
    let replayed_result = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_004,
            &intent.operation_id,
            &intent.after_source_sha256,
            status.result.as_ref().unwrap().result_body.clone(),
        )
        .unwrap();
    assert!(replayed_result.replayed);
    assert_eq!(journal.replay().unwrap().len(), 5);

    let mut changed_result = result_body;
    changed_result["content"][0]["text"] = json!("different result");
    let conflict = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_005,
            &intent.operation_id,
            &intent.after_source_sha256,
            changed_result,
        )
        .unwrap_err();
    assert_eq!(conflict.code, "tool_operation_result_conflict");
    assert_eq!(journal.replay().unwrap().len(), 5);
}

#[test]
fn workspace_write_operation_replay_is_idempotent_and_rejects_changed_input() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, _allocation, _request, lease) = lease_for_workspace_write(&path, 41);
    let request_id = json!("write-call-1");
    let original_input = json!({"path":"src/lib.rs","content":"first\n"});
    let before_sha256 = sha256(b"base\n");
    let first = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_001,
            request_id.clone(),
            original_input.clone(),
            &before_sha256,
        )
        .unwrap();
    let replay = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_002,
            json!("write-call-retry-2"),
            original_input,
            &first.after_source_sha256,
        )
        .unwrap();

    assert!(replay.replayed);
    assert_eq!(replay.operation_id, first.operation_id);
    assert_eq!(journal.replay().unwrap().len(), 4);

    let conflict = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_003,
            request_id,
            json!({"path":"src/lib.rs","content":"different\n"}),
            &before_sha256,
        )
        .unwrap_err();
    assert_eq!(conflict.code, "tool_operation_request_id_conflict");
    assert_eq!(journal.replay().unwrap().len(), 4);
}

#[test]
fn unresolved_workspace_write_must_be_replayed_before_a_different_write() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request, first_lease) = lease_for_workspace_write(&path, 41);
    let first_input = json!({"path":"src/lib.rs","content":"first operation\n"});
    let before_sha256 = sha256(b"base\n");
    let pending = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &first_lease,
            "worker-a",
            1_001,
            json!("original-rpc"),
            first_input,
            &before_sha256,
        )
        .expect("the first operation intent is durable before its filesystem effect");
    let replacement_lease = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "workspace-write-recovery-lease",
            "worker-b",
            || first_lease.expires_at_unix_ms,
            4_000,
            &request,
        )
        .expect("a new fence can recover after the prior lease expires");
    let different_input = json!({"path":"src/lib.rs","content":"different operation\n"});

    let begin_error = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &replacement_lease,
            "worker-b",
            first_lease.expires_at_unix_ms + 1,
            json!("different-rpc"),
            different_input.clone(),
            &before_sha256,
        )
        .expect_err("a pending write must be reconciled before creating another intent");
    assert_eq!(begin_error.code, "tool_operation_recovery_pending");
    assert!(!begin_error.retryable);
    assert_eq!(
        begin_error.action,
        "replay the exact unresolved workspace.write_text input under the current lease before requesting a different write"
    );
    assert_eq!(begin_error.details["operation_id"], pending.operation_id);

    let source = std::sync::Arc::new(std::sync::Mutex::new("base\n".to_owned()));
    let source_for_read = source.clone();
    let execute_error = journal
        .execute_workspace_write(
            &scope(),
            41,
            &replacement_lease,
            "worker-b",
            || first_lease.expires_at_unix_ms + 2,
            json!("different-rpc-execute"),
            different_input,
            move || Ok(sha256(source_for_read.lock().unwrap().as_bytes())),
            |_, _| panic!("a different write must stop before its filesystem effect"),
        )
        .expect_err("the filesystem writer must enforce the same unresolved-intent fence");
    assert_eq!(execute_error.code, "tool_operation_recovery_pending");
    assert!(!execute_error.retryable);
    assert_eq!(execute_error.action, begin_error.action);
    assert_eq!(execute_error.details["operation_id"], pending.operation_id);
    assert_eq!(*source.lock().unwrap(), "base\n");
    assert_eq!(
        journal
            .replay()
            .unwrap()
            .iter()
            .filter(|event| event.kind == "mission_tool_operation_intent_v1")
            .count(),
        1
    );
}

#[test]
fn workspace_write_result_requires_an_intent_and_the_declared_post_state() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, _allocation, _request, lease) = lease_for_workspace_write(&path, 41);
    let result_body = json!({"content":[{"type":"text","text":"done"}],"isError":false});

    let missing = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_001,
            "mission-workspace-write-v1:missing",
            &"a".repeat(64),
            result_body.clone(),
        )
        .unwrap_err();
    assert_eq!(missing.code, "tool_operation_intent_missing");

    let intent = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_002,
            json!("write-call-2"),
            json!({"path":"src/lib.rs","content":"replacement\n"}),
            &sha256(b"base\n"),
        )
        .unwrap();
    let error_result_body = json!({
        "content":[{"type":"text","text":"write failed"}],
        "isError":true
    });
    let false_success = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_003,
            &intent.operation_id,
            &intent.after_source_sha256,
            error_result_body,
        )
        .unwrap_err();
    assert_eq!(false_success.code, "tool_operation_response_invalid");

    let wrong_observation = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_004,
            &intent.operation_id,
            &sha256(b"third state\n"),
            result_body,
        )
        .unwrap_err();

    assert_eq!(wrong_observation.code, "tool_operation_effect_not_observed");
    assert_eq!(journal.replay().unwrap().len(), 4);
}

#[test]
fn workspace_write_intent_refuses_expired_lease_and_oversized_content() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, _allocation, _request, lease) = lease_for_workspace_write(&path, 41);
    let before_sha256 = sha256(b"base\n");
    let expired = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            lease.expires_at_unix_ms,
            json!("expired-write"),
            json!({"path":"src/lib.rs","content":"replacement\n"}),
            &before_sha256,
        )
        .unwrap_err();
    assert_eq!(expired.code, "execution_lease_expired");

    let oversized = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_001,
            json!("oversized-write"),
            json!({"path":"src/lib.rs","content":"x".repeat(65_537)}),
            &before_sha256,
        )
        .unwrap_err();
    assert_eq!(oversized.code, "tool_operation_input_invalid");
    assert_eq!(journal.replay().unwrap().len(), 3);
}

#[test]
fn workspace_write_result_can_be_completed_only_by_the_current_fence() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request, lease) = lease_for_workspace_write(&path, 41);
    let intent = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_001,
            json!("write-call-resume"),
            json!({"path":"src/lib.rs","content":"replacement\n"}),
            &sha256(b"base\n"),
        )
        .unwrap();
    let replacement_lease = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "workspace-write-lease-2",
            "worker-b",
            || lease.expires_at_unix_ms,
            4_000,
            &request,
        )
        .unwrap();
    let result_body = json!({"content":[{"type":"text","text":"written"}],"isError":false});

    let stale = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &lease,
            "worker-a",
            lease.expires_at_unix_ms,
            &intent.operation_id,
            &intent.after_source_sha256,
            result_body.clone(),
        )
        .unwrap_err();
    assert_eq!(stale.code, "execution_lease_fenced");

    let completed = journal
        .complete_workspace_write_result(
            &scope(),
            41,
            &replacement_lease,
            "worker-b",
            lease.expires_at_unix_ms + 1,
            &intent.operation_id,
            &intent.after_source_sha256,
            result_body,
        )
        .unwrap();
    assert_eq!(completed.fencing_token, replacement_lease.fencing_token);
    assert_eq!(completed.fencing_token, lease.fencing_token + 1);
}

#[test]
fn execute_workspace_write_holds_the_fence_and_replays_with_a_new_rpc_id() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, _allocation, request, lease) = lease_for_workspace_write(&path, 41);
    let source = std::sync::Arc::new(std::sync::Mutex::new("base\n".to_owned()));
    let effect_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let input = json!({"path":"src/lib.rs","content":"fixed\n"});
    let source_for_read = source.clone();
    let source_for_write = source.clone();
    let effects_for_write = effect_calls.clone();
    let journal_for_write = MissionExecutionJournalV1::open(&path);
    let requirement_sha256 = request.requirement_sha256.clone();

    let completed = journal
        .execute_workspace_write(
            &scope(),
            41,
            &lease,
            "worker-a",
            || 1_001,
            json!("write-rpc-1"),
            input.clone(),
            move || Ok(sha256(source_for_read.lock().unwrap().as_bytes())),
            move |expected_before, content| {
                let events = journal_for_write.replay().unwrap();
                assert_eq!(
                    events.last().unwrap().kind,
                    "mission_tool_operation_intent_v1"
                );
                let error = journal_for_write
                    .allocate_generation(&scope(), 41, &requirement_sha256, "write-during-effect")
                    .unwrap_err();
                assert_eq!(error.code, "execution_mission_busy");
                let mut source = source_for_write.lock().unwrap();
                assert_eq!(sha256(source.as_bytes()), expected_before);
                *source = content.to_owned();
                effects_for_write.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            },
        )
        .expect("intent, filesystem effect, readback and result should share the mission fence");

    assert!(!completed.replayed);
    assert_eq!(completed.request_id, json!("write-rpc-1"));
    assert_eq!(completed.result_body["isError"], false);
    assert_eq!(effect_calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    let source_for_replay = source.clone();
    let replay = journal
        .execute_workspace_write(
            &scope(),
            41,
            &lease,
            "worker-a",
            || 1_002,
            json!("write-rpc-retry"),
            input,
            move || Ok(sha256(source_for_replay.lock().unwrap().as_bytes())),
            |_, _| panic!("a completed write must not repeat its filesystem effect"),
        )
        .expect("the host should replay the stored MCP result for a new correlation ID");
    assert!(replay.replayed);
    assert_eq!(replay.request_id, json!("write-rpc-retry"));
    assert_eq!(replay.result_body, completed.result_body);
    assert_eq!(effect_calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(journal.replay().unwrap().len(), 5);
}

#[test]
fn execute_workspace_write_allows_the_same_content_after_an_intervening_source_state() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, _allocation, _request, lease) = lease_for_workspace_write(&path, 41);
    let source = std::sync::Arc::new(std::sync::Mutex::new("base\n".to_owned()));
    let input = json!({"path":"src/lib.rs","content":"fixed\n"});
    let first_source_for_read = source.clone();
    let first_source_for_write = source.clone();
    let first = journal
        .execute_workspace_write(
            &scope(),
            41,
            &lease,
            "worker-a",
            || 1_001,
            json!("write-first"),
            input.clone(),
            move || Ok(sha256(first_source_for_read.lock().unwrap().as_bytes())),
            move |expected_before, content| {
                let mut source = first_source_for_write.lock().unwrap();
                assert_eq!(sha256(source.as_bytes()), expected_before);
                *source = content.to_owned();
                Ok(())
            },
        )
        .unwrap();
    *source.lock().unwrap() = "intervening change\n".to_owned();

    let second_source_for_read = source.clone();
    let second_source_for_write = source.clone();
    let second = journal
        .execute_workspace_write(
            &scope(),
            41,
            &lease,
            "worker-a",
            || 1_002,
            json!("write-second"),
            input,
            move || Ok(sha256(second_source_for_read.lock().unwrap().as_bytes())),
            move |expected_before, content| {
                let mut source = second_source_for_write.lock().unwrap();
                assert_eq!(sha256(source.as_bytes()), expected_before);
                *source = content.to_owned();
                Ok(())
            },
        )
        .unwrap();

    assert_ne!(first.operation_id, second.operation_id);
    assert!(!second.replayed);
    assert_eq!(*source.lock().unwrap(), "fixed\n");
    assert_eq!(
        journal
            .replay()
            .unwrap()
            .iter()
            .filter(|event| event.kind == "mission_tool_operation_intent_v1")
            .count(),
        2
    );
}

#[test]
fn concurrent_workspace_writes_cannot_create_overlapping_effect_intents() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, _allocation, _request, lease) = lease_for_workspace_write(&path, 41);
    let source = std::sync::Arc::new(std::sync::Mutex::new("base\n".to_owned()));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let journal_path = path.clone();
    let source_for_thread = source.clone();
    let lease_for_thread = lease.clone();
    let first = thread::spawn(move || {
        let journal = MissionExecutionJournalV1::open(journal_path);
        let source_for_read = source_for_thread.clone();
        let source_for_write = source_for_thread;
        journal.execute_workspace_write(
            &scope(),
            41,
            &lease_for_thread,
            "worker-a",
            || 1_001,
            json!("concurrent-write-1"),
            json!({"path":"src/lib.rs","content":"first\n"}),
            move || Ok(sha256(source_for_read.lock().unwrap().as_bytes())),
            move |expected_before, content| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                let mut source = source_for_write.lock().unwrap();
                assert_eq!(sha256(source.as_bytes()), expected_before);
                *source = content.to_owned();
                Ok(())
            },
        )
    });

    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the first effect should be in flight under the mission fence");
    let source_for_second = source.clone();
    let second = journal
        .execute_workspace_write(
            &scope(),
            41,
            &lease,
            "worker-a",
            || 1_002,
            json!("concurrent-write-2"),
            json!({"path":"src/lib.rs","content":"second\n"}),
            move || Ok(sha256(source_for_second.lock().unwrap().as_bytes())),
            |_, _| panic!("a concurrent writer must fail before its filesystem effect"),
        )
        .unwrap_err();
    assert_eq!(second.code, "execution_mission_busy");

    release_tx.send(()).unwrap();
    first
        .join()
        .unwrap()
        .expect("the first serialized write should finish");
    assert_eq!(*source.lock().unwrap(), "first\n");
    let events = journal.replay().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "mission_tool_operation_intent_v1")
            .count(),
        1
    );
}

#[test]
fn workspace_write_reconciles_a_post_state_under_a_newer_fence() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request, lease) = lease_for_workspace_write(&path, 41);
    let input = json!({"path":"src/lib.rs","content":"recovered\n"});
    let before_sha256 = sha256(b"base\n");
    let intent = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_001,
            json!("original-rpc"),
            input.clone(),
            &before_sha256,
        )
        .unwrap();
    let source = std::sync::Arc::new(std::sync::Mutex::new("recovered\n".to_owned()));
    let replacement_lease = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "workspace-write-recovery-lease",
            "worker-b",
            || lease.expires_at_unix_ms,
            4_000,
            &request,
        )
        .unwrap();
    let source_for_read = source.clone();
    let recovered = journal
        .execute_workspace_write(
            &scope(),
            41,
            &replacement_lease,
            "worker-b",
            || lease.expires_at_unix_ms + 1,
            json!("recovery-rpc"),
            input,
            move || Ok(sha256(source_for_read.lock().unwrap().as_bytes())),
            |_, _| panic!("the exact post-state is already present; do not rewrite it"),
        )
        .expect("a fresh fence should safely promote the observed post-state to a result");

    assert_eq!(recovered.operation_id, intent.operation_id);
    assert_eq!(recovered.request_id, json!("recovery-rpc"));
    assert_eq!(recovered.result_body["isError"], false);
    let status = journal
        .workspace_write_operation_status(
            &scope(),
            41,
            &replacement_lease,
            "worker-b",
            lease.expires_at_unix_ms + 2,
            &json!({"path":"src/lib.rs","content":"recovered\n"}),
            &intent.after_source_sha256,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        status.result.unwrap().fencing_token,
        replacement_lease.fencing_token
    );
    assert_eq!(*source.lock().unwrap(), "recovered\n");
}

#[test]
fn workspace_write_recovery_preflight_requires_the_admitted_base_and_intent_state() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request, lease) = lease_for_workspace_write(&path, 41);
    let input = json!({"path":"src/lib.rs","content":"recovered\n"});
    let before_sha256 = sha256(b"base\n");
    let intent = journal
        .begin_workspace_write_intent(
            &scope(),
            41,
            &lease,
            "worker-a",
            1_001,
            json!("original-rpc"),
            input,
            &before_sha256,
        )
        .unwrap();

    let pre_state = journal
        .workspace_write_recovery_preflight(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            &before_sha256,
            &before_sha256,
        )
        .unwrap()
        .expect("an unresolved intent at its exact pre-state should be recoverable");
    assert_eq!(pre_state.operation_id, intent.operation_id);
    assert_eq!(pre_state.input, intent.input);

    let post_state = journal
        .workspace_write_recovery_preflight(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            &before_sha256,
            &intent.after_source_sha256,
        )
        .unwrap()
        .expect("an unresolved intent at its exact post-state should be recoverable");
    assert_eq!(post_state.operation_id, intent.operation_id);

    let changed_base = journal
        .workspace_write_recovery_preflight(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            &sha256(b"different base\n"),
            &intent.after_source_sha256,
        )
        .unwrap_err();
    assert_eq!(
        changed_base.code,
        "tool_operation_recovery_binding_mismatch"
    );

    let third_state = journal
        .workspace_write_recovery_preflight(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            &before_sha256,
            &sha256(b"unrelated candidate\n"),
        )
        .unwrap_err();
    assert_eq!(third_state.code, "tool_operation_recovery_binding_mismatch");
}

#[test]
fn active_execution_lease_is_exclusive_and_replayable() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);

    let first = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-01",
            "worker-a",
            || 1_000,
            100,
            &request,
        )
        .unwrap();
    let replay = MissionExecutionJournalV1::open(&path)
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-01",
            "worker-a",
            || 1_010,
            100,
            &request,
        )
        .unwrap();
    let competing = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-02",
            "worker-b",
            || 1_010,
            100,
            &request,
        )
        .unwrap_err();
    let changed_replay = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-01",
            "worker-b",
            || 1_010,
            100,
            &request,
        )
        .unwrap_err();
    let wrong_holder = journal
        .validate_lease(&scope(), 41, &first, "worker-b", 1_010)
        .unwrap_err();

    assert_eq!(first.fencing_token, 1);
    assert_eq!(first.expires_at_unix_ms, 1_100);
    assert!(replay.replayed);
    assert_eq!(replay.lease_id, first.lease_id);
    assert_eq!(competing.code, "execution_lease_active");
    assert!(competing.retryable);
    assert_eq!(changed_replay.code, "execution_lease_idempotency_conflict");
    assert_eq!(wrong_holder.code, "execution_lease_holder_mismatch");
    journal
        .validate_lease(&scope(), 41, &first, "worker-a", 1_099)
        .unwrap();
    assert_eq!(journal.replay().unwrap().len(), 3);
}

#[test]
fn expired_lease_recovers_with_a_higher_fence_and_rejects_the_old_worker() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let first = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-old",
            "worker-old",
            || 1_000,
            100,
            &request,
        )
        .unwrap();
    let clock_regressed = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-clock-regressed",
            "worker-new",
            || 999,
            100,
            &request,
        )
        .unwrap_err();
    let expired = journal
        .validate_lease(&scope(), 41, &first, "worker-old", 1_100)
        .unwrap_err();
    let mut forged_extension = first.clone();
    forged_extension.expires_at_unix_ms = 9_999;
    forged_extension.ttl_ms = 8_999;
    let forged = journal
        .validate_lease(&scope(), 41, &forged_extension, "worker-old", 1_100)
        .unwrap_err();
    let replay_after_expiry = MissionExecutionJournalV1::open(&path)
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-old",
            "worker-old",
            || 2_000,
            100,
            &request,
        )
        .unwrap();
    let recovered = MissionExecutionJournalV1::open(&path)
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-new",
            "worker-new",
            || 1_100,
            100,
            &request,
        )
        .unwrap();
    let stale = journal
        .validate_lease(&scope(), 41, &first, "worker-old", 1_101)
        .unwrap_err();

    assert_eq!(expired.code, "execution_lease_expired");
    assert_eq!(clock_regressed.code, "execution_lease_clock_regressed");
    assert_eq!(forged.code, "execution_lease_expired");
    assert!(replay_after_expiry.replayed);
    assert_eq!(replay_after_expiry.fencing_token, 1);
    assert_eq!(recovered.fencing_token, 2);
    assert!(recovered.expires_at_unix_ms > 1_100);
    assert_eq!(stale.code, "execution_lease_fenced");
    journal
        .validate_lease(&scope(), 41, &recovered, "worker-new", 1_101)
        .unwrap();
    assert_eq!(journal.replay().unwrap().len(), 4);
}

#[test]
fn execution_lease_cannot_outlive_the_runtime_timeout() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);

    let error = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "lease-too-long",
            "worker-a",
            || 1_000,
            request.capabilities.budget.timeout_ms + 1,
            &request,
        )
        .unwrap_err();

    assert_eq!(error.code, "execution_lease_ttl_invalid");
    assert_eq!(journal.replay().unwrap().len(), 2);
}

#[test]
fn newer_generation_fences_every_lease_from_older_generations() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, first_allocation, first_request) = allocation_and_claim(&path);
    let old_lease = journal
        .acquire_lease(
            &scope(),
            41,
            first_allocation.execution_generation,
            &first_allocation.allocation_id,
            "generation-one-lease",
            "worker-old",
            || 1_000,
            100,
            &first_request,
        )
        .unwrap();

    let next_request = request(2);
    let next_allocation = journal
        .allocate_generation(
            &scope(),
            41,
            &next_request.requirement_sha256,
            "generation-two-allocation",
        )
        .unwrap();
    let events_after_next_allocation = journal.replay().unwrap().len();
    let old_validation_after_allocation = journal
        .validate_lease(&scope(), 41, &old_lease, "worker-old", 1_001)
        .unwrap_err();
    let old_effect_ran = std::cell::Cell::new(false);
    let old_effect = journal
        .with_valid_lease(
            &scope(),
            41,
            &old_lease,
            "worker-old",
            || 1_001,
            || {
                old_effect_ran.set(true);
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(
        old_validation_after_allocation.code,
        "execution_lease_generation_superseded"
    );
    assert_eq!(old_effect.code, "execution_lease_generation_superseded");
    assert!(!old_effect_ran.get());
    assert_eq!(
        journal.replay().unwrap().len(),
        events_after_next_allocation
    );

    journal
        .claim_allocated(
            &scope(),
            41,
            next_allocation.execution_generation,
            &next_allocation.allocation_id,
            &next_request,
        )
        .unwrap();
    let current_lease = journal
        .acquire_lease(
            &scope(),
            41,
            next_allocation.execution_generation,
            &next_allocation.allocation_id,
            "generation-two-lease",
            "worker-new",
            || 1_001,
            100,
            &next_request,
        )
        .unwrap();

    let old_validation = journal
        .validate_lease(&scope(), 41, &old_lease, "worker-old", 1_001)
        .unwrap_err();
    let events_before_refused_reacquisitions = journal.replay().unwrap().len();
    let old_same_id_reacquisition = journal
        .acquire_lease(
            &scope(),
            41,
            first_allocation.execution_generation,
            &first_allocation.allocation_id,
            "generation-one-lease",
            "worker-old",
            || 1_001,
            100,
            &first_request,
        )
        .unwrap_err();
    let old_reacquisition = journal
        .acquire_lease(
            &scope(),
            41,
            first_allocation.execution_generation,
            &first_allocation.allocation_id,
            "generation-one-retry",
            "worker-old-retry",
            || 1_001,
            100,
            &first_request,
        )
        .unwrap_err();

    assert_eq!(old_validation.code, "execution_lease_generation_superseded");
    assert_eq!(
        old_same_id_reacquisition.code,
        "execution_lease_generation_superseded"
    );
    assert_eq!(
        old_reacquisition.code,
        "execution_lease_generation_superseded"
    );
    assert_eq!(
        journal.replay().unwrap().len(),
        events_before_refused_reacquisitions
    );
    assert_eq!(current_lease.fencing_token, old_lease.fencing_token + 1);
    journal
        .validate_lease(&scope(), 41, &current_lease, "worker-new", 1_002)
        .unwrap();
}

#[test]
fn lease_retries_share_one_absolute_runtime_deadline() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "deadline-first",
            "worker-first",
            || 1_000,
            100,
            &request,
        )
        .unwrap();

    let beyond_deadline = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "deadline-too-long",
            "worker-next",
            || 1_100,
            request.capabilities.budget.timeout_ms,
            &request,
        )
        .unwrap_err();
    let retry_lease = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "deadline-retry",
            "worker-retry",
            || 1_100,
            100,
            &request,
        )
        .unwrap();
    let final_lease = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "deadline-final",
            "worker-final",
            || retry_lease.expires_at_unix_ms,
            request.capabilities.budget.timeout_ms - 200,
            &request,
        )
        .unwrap();
    let after_deadline = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "deadline-after",
            "worker-after",
            || final_lease.expires_at_unix_ms,
            1,
            &request,
        )
        .unwrap_err();

    assert_eq!(beyond_deadline.code, "execution_lease_deadline_exceeded");
    assert_eq!(final_lease.expires_at_unix_ms, 11_000);
    assert_eq!(after_deadline.code, "execution_lease_deadline_exceeded");
}

#[test]
fn mission_effect_keeps_global_journal_available_and_other_missions_progress() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let lease = journal
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "atomic-tool-lease",
            "worker-atomic",
            || 1_000,
            1_000,
            &request,
        )
        .unwrap();

    let (effect_entered_tx, effect_entered_rx) = mpsc::channel();
    let (release_effect_tx, release_effect_rx) = mpsc::channel();
    let effect_path = path.clone();
    let effect_probe_path = path.clone();
    let effect_lease = lease.clone();
    let effect = thread::spawn(move || {
        MissionExecutionJournalV1::open(effect_path).with_valid_lease(
            &scope(),
            41,
            &effect_lease,
            "worker-atomic",
            || 1_001,
            || {
                let global_lock_was_free = !journal_lock_is_contended(&effect_probe_path);
                effect_entered_tx.send(global_lock_was_free).unwrap();
                release_effect_rx.recv().unwrap();
                Ok("tool effect committed")
            },
        )
    });
    let global_lock_was_free = effect_entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the lease-guarded effect should start");
    if !global_lock_was_free {
        release_effect_tx.send(()).unwrap();
        assert!(effect.join().unwrap().is_ok());
        panic!("lease-guarded effect retained the global journal lock");
    }
    let webhook_secret = b"mission-effect-webhook-fixture";
    let webhook_body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": "forge-admin/agent-forge-core"},
        "issue": {"number": 41, "state": "open"}
    }))
    .unwrap();
    let mut webhook_mac = HmacSha256::new_from_slice(webhook_secret).unwrap();
    webhook_mac.update(&webhook_body);
    let webhook_signature = hex::encode(webhook_mac.finalize().into_bytes());
    assert!(matches!(
        ingest_forgejo_delivery(
            &EventJournal::open(&path),
            webhook_secret,
            "webhook-during-mission-effect",
            "issues",
            &webhook_signature,
            &webhook_body,
        ),
        Ok(agent_forge_core::AppendResult::Appended(_))
    ));

    let (allocation_result_tx, allocation_result_rx) = mpsc::channel();
    let allocation_path = path.clone();
    let requirement_sha256 = request.requirement_sha256.clone();
    let allocator = thread::spawn(move || {
        let result = MissionExecutionJournalV1::open(allocation_path).allocate_generation(
            &scope(),
            41,
            &requirement_sha256,
            "next-generation-during-tool",
        );
        allocation_result_tx.send(result).unwrap();
    });

    let allocation_during_effect = allocation_result_rx.recv_timeout(Duration::from_millis(250));

    let (other_effect_entered_tx, other_effect_entered_rx) = mpsc::channel();
    let other_path = path.clone();
    let other_effect = thread::spawn(move || {
        let (other_journal, other_allocation, other_request) =
            allocation_and_claim_for_issue(&other_path, 42);
        let other_lease = other_journal
            .acquire_lease(
                &scope(),
                42,
                other_allocation.execution_generation,
                &other_allocation.allocation_id,
                "other-mission-lease",
                "worker-other",
                || 1_000,
                1_000,
                &other_request,
            )
            .unwrap();
        other_journal
            .with_valid_lease(
                &scope(),
                42,
                &other_lease,
                "worker-other",
                || 1_001,
                || {
                    other_effect_entered_tx.send(()).unwrap();
                    Ok(())
                },
            )
            .unwrap();
    });

    let other_mission_progressed = other_effect_entered_rx
        .recv_timeout(Duration::from_millis(250))
        .is_ok();

    release_effect_tx.send(()).unwrap();
    assert_eq!(effect.join().unwrap().unwrap(), "tool effect committed");
    allocator.join().unwrap();
    other_effect.join().unwrap();

    assert!(
        global_lock_was_free,
        "tool effect must not hold the global journal lock"
    );
    assert!(
        other_mission_progressed,
        "a different mission should not wait for this effect"
    );
    assert!(
        allocation_during_effect.is_ok(),
        "same-mission generation allocation should return a retryable busy error promptly"
    );
    let allocation_error = allocation_during_effect.unwrap().unwrap_err();
    assert_eq!(allocation_error.code, "execution_mission_busy");
    assert!(allocation_error.retryable);
    journal
        .validate_lease(&scope(), 41, &lease, "worker-atomic", 1_002)
        .unwrap();
}

#[test]
fn mission_operations_return_busy_without_sampling_clock_while_global_lock_is_held() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (_journal, allocation, request) = allocation_and_claim(&path);
    let (global_locked_tx, global_locked_rx) = mpsc::channel();
    let (release_global_tx, release_global_rx) = mpsc::channel();
    let blocker_path = path.clone();
    let blocker = thread::spawn(move || {
        let result = EventJournal::open(blocker_path).transact(|_| {
            global_locked_tx.send(()).unwrap();
            release_global_rx.recv().unwrap();
            Ok::<_, ()>(agent_forge_core::JournalTransaction::Return(()))
        });
        assert!(matches!(result, Ok(())));
    });
    global_locked_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    let (clock_sampled_tx, clock_sampled_rx) = mpsc::channel();
    let (lease_result_tx, lease_result_rx) = mpsc::channel();
    let acquire_path = path.clone();
    let allocation_id = allocation.allocation_id.clone();
    let acquire_request = request.clone();
    let acquire_clock_sampled_tx = clock_sampled_tx.clone();
    let acquire = thread::spawn(move || {
        let result = MissionExecutionJournalV1::open(acquire_path).acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation_id,
            "fresh-clock-lease",
            "worker-fresh-clock",
            || {
                acquire_clock_sampled_tx.send(()).unwrap();
                1_000
            },
            1_000,
            &acquire_request,
        );
        lease_result_tx.send(result).unwrap();
    });

    let first_acquire = lease_result_rx.recv_timeout(Duration::from_secs(1));
    if first_acquire.is_err() {
        release_global_tx.send(()).unwrap();
        blocker.join().unwrap();
        acquire.join().unwrap();
        panic!("typed lease acquisition waited for the global lock");
    }
    let first_acquire = first_acquire.unwrap().unwrap_err();
    assert_eq!(first_acquire.code, "execution_mission_busy");
    assert!(first_acquire.retryable);
    assert!(clock_sampled_rx.try_recv().is_err());

    release_global_tx.send(()).unwrap();
    blocker.join().unwrap();
    let lease = MissionExecutionJournalV1::open(&path)
        .acquire_lease(
            &scope(),
            41,
            allocation.execution_generation,
            &allocation.allocation_id,
            "fresh-clock-lease",
            "worker-fresh-clock",
            || {
                clock_sampled_tx.send(()).unwrap();
                1_000
            },
            1_000,
            &request,
        )
        .unwrap();
    clock_sampled_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    acquire.join().unwrap();
    assert_eq!(lease.issued_at_unix_ms, 1_000);

    let (effect_global_locked_tx, effect_global_locked_rx) = mpsc::channel();
    let (release_effect_global_tx, release_effect_global_rx) = mpsc::channel();
    let effect_blocker_path = path.clone();
    let effect_blocker = thread::spawn(move || {
        let result = EventJournal::open(effect_blocker_path).transact(|_| {
            effect_global_locked_tx.send(()).unwrap();
            release_effect_global_rx.recv().unwrap();
            Ok::<_, ()>(agent_forge_core::JournalTransaction::Return(()))
        });
        assert!(matches!(result, Ok(())));
    });
    effect_global_locked_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    let (effect_clock_tx, effect_clock_rx) = mpsc::channel();
    let (effect_entered_tx, effect_entered_rx) = mpsc::channel();
    let (effect_result_tx, effect_result_rx) = mpsc::channel();
    let effect_path = path.clone();
    let effect_lease = lease.clone();
    let contended_effect_clock_tx = effect_clock_tx.clone();
    let contended_effect_entered_tx = effect_entered_tx.clone();
    let effect = thread::spawn(move || {
        let result = MissionExecutionJournalV1::open(effect_path).with_valid_lease(
            &scope(),
            41,
            &effect_lease,
            "worker-fresh-clock",
            || {
                contended_effect_clock_tx.send(()).unwrap();
                1_001
            },
            || {
                contended_effect_entered_tx.send(()).unwrap();
                Ok(())
            },
        );
        effect_result_tx.send(result).unwrap();
    });
    let first_effect = effect_result_rx.recv_timeout(Duration::from_secs(1));
    if first_effect.is_err() {
        release_effect_global_tx.send(()).unwrap();
        effect_blocker.join().unwrap();
        effect.join().unwrap();
        panic!("lease-guarded effect waited for the global lock");
    }
    let first_effect = first_effect.unwrap().unwrap_err();
    assert_eq!(first_effect.code, "execution_mission_busy");
    assert!(first_effect.retryable);
    assert!(effect_clock_rx.try_recv().is_err());
    assert!(effect_entered_rx.try_recv().is_err());

    release_effect_global_tx.send(()).unwrap();
    effect_blocker.join().unwrap();
    MissionExecutionJournalV1::open(&path)
        .with_valid_lease(
            &scope(),
            41,
            &lease,
            "worker-fresh-clock",
            || {
                effect_clock_tx.send(()).unwrap();
                1_001
            },
            || {
                effect_entered_tx.send(()).unwrap();
                Ok(())
            },
        )
        .unwrap();
    effect_clock_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    effect_entered_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    effect.join().unwrap();
}

#[test]
fn mission_lock_is_private_and_released_after_process_interruption() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (_journal, allocation, request) = allocation_and_claim(&path);
    let lock_path = mission_lock_path_for(&path, &allocation.mission_id);
    let lock_directory = lock_path.parent().unwrap();
    assert_eq!(
        std::fs::metadata(lock_directory).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(std::fs::metadata(&lock_path).unwrap().mode() & 0o777, 0o600);

    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mission_lock_child_holds_until_parent_kills",
                "--ignored",
                "--nocapture",
            ])
            .env("AGENT_FORGE_MISSION_LOCK_CHILD", &lock_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (line_tx, line_rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                return;
            }
        }
    });
    assert_eq!(
        receive_one_of(&line_rx, &["LOCKED"], Duration::from_secs(3)),
        "LOCKED"
    );
    child.0.kill().unwrap();
    let _ = child.0.wait().unwrap();

    let next = MissionExecutionJournalV1::open(&path)
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "after-interrupted-worker",
        )
        .unwrap();
    assert_eq!(
        next.execution_generation,
        allocation.execution_generation + 1
    );
}

#[test]
#[ignore = "invoked as a subprocess by the mission lock interruption contract"]
fn mission_lock_child_holds_until_parent_kills() {
    let Ok(path) = std::env::var("AGENT_FORGE_MISSION_LOCK_CHILD") else {
        return;
    };
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    println!("LOCKED");
    std::io::stdout().flush().unwrap();
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn generic_journal_api_cannot_append_reserved_mission_transitions() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);
    let identity =
        derive_mission_identity_v1(&scope(), 41, &request.requirement_sha256, 1).unwrap();
    let payloads = [
        (
            "mission_generation_allocated_v1",
            serde_json::json!({
                "schema_version": 1,
                "mission_id": identity.mission_id,
                "allocation_id": "forged",
                "requirement_sha256": request.requirement_sha256,
                "execution_generation": 1,
            }),
        ),
        (
            "mission_execution_claim_v1",
            serde_json::json!({
                "schema_version": 1,
                "mission_id": identity.mission_id,
                "execution_generation": 1,
                "request_sha256": "a".repeat(64),
            }),
        ),
        (
            "mission_execution_lease_acquired_v1",
            serde_json::json!({"schema_version": 1, "mission_id": identity.mission_id}),
        ),
        (
            "mission_tool_operation_intent_v1",
            serde_json::json!({"schema_version": 1, "mission_id": identity.mission_id}),
        ),
        (
            "mission_tool_operation_result_v1",
            serde_json::json!({"schema_version": 1, "mission_id": identity.mission_id}),
        ),
        (
            "mission_merge_attempt_started_v1",
            serde_json::json!({"schema_version": 1, "mission_id": identity.mission_id}),
        ),
        (
            "mission_merge_attempt_resolved_v1",
            serde_json::json!({"schema_version": 1, "mission_id": identity.mission_id}),
        ),
    ];
    for (index, (kind, payload)) in payloads.into_iter().enumerate() {
        let journal = EventJournal::open(&path);
        let append_error = journal
            .append(&format!("forged-append-{index}"), kind, payload.clone())
            .unwrap_err();
        assert_eq!(append_error, agent_forge_core::JournalError::InvalidEvent);

        let transaction_result = journal.transact(|_| {
            Ok::<_, ()>(agent_forge_core::JournalTransaction::Append {
                idempotency_key: format!("forged-transaction-{index}"),
                kind: kind.to_owned(),
                payload: payload.clone(),
                result: (),
            })
        });
        assert!(matches!(
            transaction_result,
            Err(agent_forge_core::JournalTransactionError::Journal(
                agent_forge_core::JournalError::InvalidEvent
            ))
        ));
    }
    assert!(EventJournal::open(&path).replay().unwrap().is_empty());
}

#[test]
fn generic_transaction_reentry_into_mission_transition_returns_busy() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let requirement_sha256 = request(1).requirement_sha256;
    let (started_tx, started_rx) = mpsc::channel();
    let (nested_tx, nested_rx) = mpsc::channel();
    let mut nested_worker = None;
    let mut response_timed_out_while_outer_lock_held = false;
    let mut nested_response_inside_transaction = None;
    let transaction_result = EventJournal::open(&path).transact(|_| {
        let nested_path = path.clone();
        let nested_requirement_sha256 = requirement_sha256.clone();
        nested_worker = Some(thread::spawn(move || {
            started_tx.send(()).unwrap();
            let nested = MissionExecutionJournalV1::open(&nested_path).allocate_generation(
                &scope(),
                41,
                &nested_requirement_sha256,
                "nested-generic-transaction",
            );
            let _ = nested_tx.send(nested);
        }));
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("nested transition worker should start");
        match nested_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(result) => nested_response_inside_transaction = Some(result),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                response_timed_out_while_outer_lock_held = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("nested transition worker disconnected");
            }
        }
        Ok::<_, &'static str>(agent_forge_core::JournalTransaction::Return(()))
    });
    assert!(matches!(transaction_result, Ok(())));

    let nested_result = match nested_response_inside_transaction {
        Some(result) => result,
        None => nested_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("nested worker should finish after the outer transaction releases its lock"),
    };
    nested_worker.unwrap().join().unwrap();
    assert!(
        !response_timed_out_while_outer_lock_held,
        "nested transition must return while the generic transaction owns the journal lock"
    );
    let error = nested_result.expect_err("nested transition must be refused");
    assert_eq!(error.code, "execution_mission_busy");
    assert!(error.retryable);
}

#[test]
fn hard_linked_journal_and_lock_files_are_rejected() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    std::fs::write(&path, b"").unwrap();
    let journal_alias = scratch.path().join("missions-alias.jsonl");
    std::fs::hard_link(&path, &journal_alias).unwrap();
    assert_eq!(
        EventJournal::open(&journal_alias).replay().unwrap_err(),
        agent_forge_core::JournalError::InvalidEvent
    );
    std::fs::remove_file(&journal_alias).unwrap();

    EventJournal::open(&path).replay().unwrap();
    let mut global_lock = path.as_os_str().to_owned();
    global_lock.push(".lock");
    let global_lock = PathBuf::from(global_lock);
    let global_lock_alias = scratch.path().join("global-lock-alias");
    std::fs::hard_link(&global_lock, &global_lock_alias).unwrap();
    assert_eq!(
        EventJournal::open(&path).replay().unwrap_err(),
        agent_forge_core::JournalError::InvalidEvent
    );
    std::fs::remove_file(&global_lock_alias).unwrap();

    let (journal, allocation, request) = allocation_and_claim(&path);
    let mission_lock = mission_lock_path_for(&path, &allocation.mission_id);
    let mission_lock_alias = scratch.path().join("mission-lock-alias");
    std::fs::hard_link(&mission_lock, &mission_lock_alias).unwrap();
    let error = journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "reject-hard-linked-mission-lock",
        )
        .unwrap_err();
    assert_eq!(error.code, "execution_claim_invalid");
}

#[test]
fn concurrent_execution_lease_acquisition_has_one_winner() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();

    for worker in ["worker-a", "worker-b"] {
        let path = path.clone();
        let allocation_id = allocation.allocation_id.clone();
        let request = request.clone();
        let barrier = barrier.clone();
        workers.push(thread::spawn(move || {
            barrier.wait();
            retry_mission_busy(|| {
                MissionExecutionJournalV1::open(&path).acquire_lease(
                    &scope(),
                    41,
                    1,
                    &allocation_id,
                    &format!("lease-{worker}"),
                    worker,
                    || 2_000,
                    100,
                    &request,
                )
            })
        }));
    }

    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    let winners = results.iter().filter(|result| result.is_ok()).count();
    let active_refusals = results
        .iter()
        .filter(|result| {
            result
                .as_ref()
                .err()
                .is_some_and(|error| error.code == "execution_lease_active" && error.retryable)
        })
        .count();

    assert_eq!(winners, 1);
    assert_eq!(active_refusals, 1);
    assert_eq!(journal.replay().unwrap().len(), 3);
}

#[test]
fn concurrent_generation_allocations_reserve_distinct_monotonic_values() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let requirement_sha256 = request(1).requirement_sha256;
    let mut workers = Vec::new();

    for worker in 0..12 {
        let path = path.clone();
        let requirement_sha256 = requirement_sha256.clone();
        workers.push(thread::spawn(move || {
            retry_mission_busy(|| {
                MissionExecutionJournalV1::open(&path).allocate_generation(
                    &scope(),
                    41,
                    &requirement_sha256,
                    &format!("parallel-{worker}"),
                )
            })
            .unwrap()
            .execution_generation
        }));
    }

    let mut generations = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    generations.sort_unstable();

    assert_eq!(generations, (1..=12).collect::<Vec<_>>());
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        12
    );
}

#[test]
fn concurrent_retries_of_one_allocation_id_recover_one_generation() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let requirement_sha256 = request(1).requirement_sha256;
    let mut workers = Vec::new();

    for _ in 0..8 {
        let path = path.clone();
        let requirement_sha256 = requirement_sha256.clone();
        workers.push(thread::spawn(move || {
            retry_mission_busy(|| {
                MissionExecutionJournalV1::open(&path).allocate_generation(
                    &scope(),
                    41,
                    &requirement_sha256,
                    "same-request",
                )
            })
            .unwrap()
        }));
    }

    let allocations = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert!(allocations.iter().all(|allocation| {
        allocation.execution_generation == 1 && allocation.mission_id == allocations[0].mission_id
    }));
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn invalid_allocation_ids_are_refused_without_changing_the_journal() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let journal = MissionExecutionJournalV1::open(&path);
    let requirement_sha256 = request(1).requirement_sha256;

    let error = journal
        .allocate_generation(&scope(), 41, &requirement_sha256, "../outside")
        .unwrap_err();

    assert_eq!(error.code, "execution_allocation_id_invalid");
    assert!(journal.replay().unwrap().is_empty());
}

#[test]
fn malformed_generation_record_blocks_further_allocation() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);
    let idempotency_key = format!("mission-generation-v1:{}:1", request.mission_id);
    append_raw_event(
        &path,
        &idempotency_key,
        "mission_generation_allocated_v1",
        serde_json::json!({
            "schema_version": 1,
            "mission_id": request.mission_id,
            "allocation_id": "damaged-record",
            "requirement_sha256": request.requirement_sha256,
            "execution_generation": "one",
        }),
    );

    let error = MissionExecutionJournalV1::open(&path)
        .allocate_generation(&scope(), 41, &request.requirement_sha256, "damaged-record")
        .unwrap_err();

    assert_eq!(error.code, "execution_generation_journal_invalid");
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn malformed_legacy_claim_blocks_generation_allocation() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);
    let digest = request.mission_id.strip_prefix("mission-v1-").unwrap();
    let idempotency_key = format!("mission-execution-claim-v1:execution-v1-{digest}-1");
    append_raw_event(
        &path,
        &idempotency_key,
        "mission_execution_claim_v1",
        serde_json::json!({
            "schema_version": 1,
            "mission_id": request.mission_id,
            "execution_generation": "one",
            "request_sha256": "a".repeat(64),
        }),
    );

    let error = MissionExecutionJournalV1::open(&path)
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "after-damaged-claim",
        )
        .unwrap_err();

    assert_eq!(error.code, "execution_generation_journal_invalid");
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn invalid_hashes_and_generation_are_refused_before_journal_append() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let ledger = MissionExecutionJournalV1::open(&path);
    let mut bad_requirement = request(1);
    bad_requirement.requirement_sha256 = "bad".to_owned();
    let invalid_request = ledger.claim_allocated(&scope(), 41, 1, "unused-01", &bad_requirement);
    let mismatched_identity = {
        let mut request = request(1);
        request.mission_id = "mission-for-another-issue".to_owned();
        ledger.claim_allocated(&scope(), 41, 1, "unused-01", &request)
    };
    let zero_generation = ledger.claim_allocated(&scope(), 41, 0, "unused-01", &request(1));

    assert_eq!(
        invalid_request.unwrap_err().code,
        "requirement_hash_invalid"
    );
    assert_eq!(
        mismatched_identity.unwrap_err().code,
        "execution_identity_mismatch"
    );
    assert_eq!(
        zero_generation.unwrap_err().code,
        "execution_generation_invalid"
    );
    assert!(ledger.replay().unwrap().is_empty());
}

#[test]
fn corrupt_complete_journal_record_blocks_new_execution_claims() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    std::fs::write(&path, b"{\"invalid\":true}\n").unwrap();
    let ledger = MissionExecutionJournalV1::open(&path);
    let request = request(1);

    let error = ledger
        .claim_allocated(&scope(), 41, 1, "unused-01", &request)
        .unwrap_err();

    assert_eq!(error.code, "execution_journal_corrupt");
    assert_eq!(error.details["line"], 1);
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"invalid\":true}\n");
}

#[test]
fn unclassified_journal_io_failure_does_not_claim_it_is_retryable() {
    let scratch = ScratchDir::new();
    let blocker = scratch.path().join("not-a-directory");
    std::fs::write(&blocker, b"file").unwrap();
    let ledger = MissionExecutionJournalV1::open(blocker.join("missions.jsonl"));
    let request = request(1);

    let error = ledger
        .claim_allocated(&scope(), 41, 1, "unused-01", &request)
        .unwrap_err();

    assert_eq!(error.code, "execution_journal_io_failure");
    assert!(!error.retryable);
}

#[test]
fn concurrent_processes_admit_only_one_claim_for_the_same_generation() {
    let scratch = ScratchDir::new();
    let journal_path = scratch.path().join("missions.jsonl");
    let start_path = scratch.path().join("start");
    let mut children = Vec::new();
    let (sender, receiver) = mpsc::channel();

    for _ in 0..2 {
        let mut child = KillOnDrop(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "claim_child_waits_for_start_file_then_claims",
                    "--ignored",
                    "--nocapture",
                ])
                .env("AGENT_FORGE_CLAIM_CHILD_JOURNAL", &journal_path)
                .env("AGENT_FORGE_CLAIM_CHILD_START", &start_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let sender = sender.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        children.push(child);
    }
    drop(sender);

    assert_eq!(
        receive_one_of(&receiver, &["READY"], Duration::from_secs(5)),
        "READY"
    );
    assert_eq!(
        receive_one_of(&receiver, &["READY"], Duration::from_secs(5)),
        "READY"
    );
    std::fs::write(&start_path, b"go").unwrap();

    let first = receive_one_of(
        &receiver,
        &["CLAIMED", "DUPLICATE", "ERROR"],
        Duration::from_secs(5),
    );
    let second = receive_one_of(
        &receiver,
        &["CLAIMED", "DUPLICATE", "ERROR"],
        Duration::from_secs(5),
    );
    for child in &mut children {
        assert!(child.0.wait().unwrap().success());
    }

    assert_eq!(
        [first.as_str(), second.as_str()]
            .iter()
            .filter(|v| **v == "CLAIMED")
            .count(),
        1
    );
    assert_eq!(
        [first.as_str(), second.as_str()]
            .iter()
            .filter(|v| **v == "DUPLICATE")
            .count(),
        1
    );
    assert_eq!(
        MissionExecutionJournalV1::open(&journal_path)
            .replay()
            .unwrap()
            .len(),
        2
    );
}

fn merge_binding(
    issue_number: u64,
    request: &AgentExecutionRequestV1,
    pull_request_number: u64,
    base_commit: &str,
) -> MergeAttemptBindingV1 {
    let request_sha256 = sha256(&serde_json::to_vec(request).unwrap());
    MergeAttemptBindingV1::new(
        &scope(),
        issue_number,
        pull_request_number,
        base_commit,
        &"c".repeat(40),
        &request.requirement_sha256,
        &"d".repeat(64),
        &"e".repeat(64),
        &"f".repeat(64),
        &request_sha256,
        &request_sha256,
    )
    .unwrap()
}

fn append_legacy_inconclusive_merge_resolution(path: &Path) -> String {
    let events = EventJournal::open(path).replay().unwrap();
    let started = events
        .iter()
        .find(|event| event.kind == "mission_merge_attempt_started_v1")
        .expect("persisted merge intent exists");
    let mission_id = started.payload["mission_id"].as_str().unwrap().to_owned();
    let attempt_id = started.payload["attempt_id"].as_str().unwrap().to_owned();
    let binding: MergeAttemptBindingV1 =
        serde_json::from_value(started.payload["binding"].clone()).unwrap();
    let payload = json!({
        "schema_version": 1,
        "mission_id": mission_id,
        "attempt_id": attempt_id,
        "binding_sha256": sha256(&serde_json::to_vec(&binding).unwrap()),
        "resolution": {
            "schema_version": 1,
            "outcome": "inconclusive",
            // Older records could persist the terminal default even though
            // the outcome represented an ambiguous POST/readback.
            "terminal": true,
            "reason_code": "legacy_readback_unavailable",
            "merge_commit": null,
            "merge_tree_manifest_sha256": null,
            "observation_sha256": null
        }
    });
    append_raw_event(
        path,
        &format!("{attempt_id}:resolved"),
        "mission_merge_attempt_resolved_v1",
        payload,
    );
    attempt_id
}

#[test]
fn merge_attempt_intent_is_durable_before_effect() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 17, &"a".repeat(40));

    let result = journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |mode, _| {
                assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
                Ok(())
            },
            |mode, verified, ()| {
                assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
                assert_eq!(
                    verified.request_sha256,
                    sha256(&serde_json::to_vec(&request).unwrap())
                );
                let events = MissionExecutionJournalV1::open(&path).replay().unwrap();
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event.kind == "mission_merge_attempt_started_v1")
                        .count(),
                    1,
                    "the durable intent must be visible before the host effect runs"
                );
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
                        .count(),
                    0
                );
                Ok(MergeAttemptResolutionV1::inconclusive(
                    "fixture_observed_open_pr",
                ))
            },
        )
        .unwrap();

    assert_eq!(
        result,
        MergeAttemptResolutionV1::inconclusive("fixture_observed_open_pr")
    );
    let events = journal.replay().unwrap();
    let started = events
        .iter()
        .position(|event| event.kind == "mission_merge_attempt_started_v1")
        .unwrap();
    assert!(
        events[started..]
            .iter()
            .all(|event| event.kind != "mission_merge_attempt_resolved_v1")
    );
}

#[test]
fn unresolved_merge_intent_is_reused_read_only() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 18, &"a".repeat(40));

    let first = journal.with_merge_attempt(
        &scope(),
        41,
        &allocation.allocation_id,
        allocation.execution_generation,
        &request,
        binding.clone(),
        |mode, _| {
            assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
            Ok(())
        },
        |mode, _, ()| {
            assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
            Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                "fixture_transport_interrupted",
                true,
                "retry by reading the existing merge attempt; do not send another merge request",
                json!({}),
            ))
        },
    );
    assert_eq!(first.unwrap_err().code, "fixture_transport_interrupted");

    let result = journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |mode, _| {
                assert_eq!(mode, MergeAttemptModeV1::RecoverOnly);
                Ok(())
            },
            |mode, _, ()| {
                assert_eq!(mode, MergeAttemptModeV1::RecoverOnly);
                Ok(MergeAttemptResolutionV1::inconclusive(
                    "fixture_readback_unavailable",
                ))
            },
        )
        .unwrap();

    assert_eq!(
        result,
        MergeAttemptResolutionV1::inconclusive("fixture_readback_unavailable")
    );
    let events = journal.replay().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_started_v1")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
            .count(),
        0,
        "a temporary readback failure must leave the merge intent recoverable"
    );
}

#[test]
fn inconclusive_merge_readback_remains_unresolved_until_a_conclusive_readback() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 25, &"a".repeat(40));

    let first = journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding.clone(),
            |_, _| Ok(()),
            |_, _, ()| {
                Ok(MergeAttemptResolutionV1::inconclusive(
                    "fixture_readback_unavailable",
                ))
            },
        )
        .unwrap();
    assert_eq!(
        first.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Inconclusive
    );
    assert_eq!(
        journal
            .replay()
            .unwrap()
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
            .count(),
        0,
        "an inconclusive observation must not close the unresolved attempt"
    );

    let mut readback_calls = 0;
    let recovered = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |_| {
                readback_calls += 1;
                Ok(MergeAttemptResolutionV1::merged(
                    &"1".repeat(40),
                    &"2".repeat(64),
                    &"3".repeat(64),
                ))
            },
        )
        .unwrap()
        .expect("existing intent remains eligible for readback");
    assert_eq!(readback_calls, 1);
    assert_eq!(
        recovered.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Merged
    );
    assert_eq!(
        journal
            .replay()
            .unwrap()
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
            .count(),
        1
    );
}

#[test]
fn unresolved_merge_intent_blocks_a_new_execution_generation() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 26, &"a".repeat(40));
    journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |_, _| Ok(()),
            |_, _, ()| {
                Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                    "fixture_process_interrupted",
                    true,
                    "recover the unresolved merge intent by readback before a new generation",
                    json!({}),
                ))
            },
        )
        .unwrap_err();

    let error = journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "next-generation-after-unresolved-merge",
        )
        .expect_err("an unresolved intent must fence a new generation");
    assert_eq!(error.code, "execution_merge_attempt_unresolved");
}

#[test]
fn unresolved_merge_intent_blocks_another_mission_from_the_same_pull_request() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, first_allocation, first_request) = allocation_and_claim(&path);
    let pull_request_number = 27;
    journal
        .with_merge_attempt(
            &scope(),
            41,
            &first_allocation.allocation_id,
            first_allocation.execution_generation,
            &first_request,
            merge_binding(41, &first_request, pull_request_number, &"a".repeat(40)),
            |_, _| Ok(()),
            |_, _, ()| {
                Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                    "fixture_process_interrupted",
                    true,
                    "recover the open pull-request intent before another mission can merge it",
                    json!({}),
                ))
            },
        )
        .unwrap_err();

    let (_, second_allocation, second_request) = allocation_and_claim_for_issue(&path, 42);
    let binding = merge_binding(42, &second_request, pull_request_number, &"a".repeat(40));
    let recovery_error = journal
        .recover_merge_attempt_if_present(
            &scope(),
            42,
            &second_allocation.allocation_id,
            second_allocation.execution_generation,
            &second_request,
            binding.clone(),
            |_| panic!("another mission cannot resolve the first mission's intent"),
        )
        .unwrap_err();
    assert_eq!(recovery_error.code, "execution_merge_target_unresolved");

    let attempt_error = journal
        .with_merge_attempt(
            &scope(),
            42,
            &second_allocation.allocation_id,
            second_allocation.execution_generation,
            &second_request,
            binding,
            |_, _| panic!("another mission cannot prepare a merge for an unresolved PR"),
            |_, _, ()| panic!("another mission cannot post a merge for an unresolved PR"),
        )
        .unwrap_err();
    assert_eq!(attempt_error.code, "execution_merge_target_unresolved");
    assert!(attempt_error.retryable);
}

#[test]
fn unresolved_merge_intent_blocks_another_pull_request_on_the_same_base_branch() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, first_allocation, first_request) = allocation_and_claim(&path);
    let unresolved_pr = 27;
    let blocking_binding = merge_binding(41, &first_request, unresolved_pr, &"a".repeat(40));
    journal
        .with_merge_attempt(
            &scope(),
            41,
            &first_allocation.allocation_id,
            first_allocation.execution_generation,
            &first_request,
            blocking_binding.clone(),
            |_, _| Ok(()),
            |_, _, ()| {
                Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                    "fixture_process_interrupted",
                    true,
                    "recover the unresolved merge intent before starting another merge",
                    json!({}),
                ))
            },
        )
        .unwrap_err();

    let (_, second_allocation, second_request) = allocation_and_claim_for_issue(&path, 42);
    let competing_pr = 28;
    let competing_binding = merge_binding(42, &second_request, competing_pr, &"a".repeat(40));
    let prepare_ran = std::cell::Cell::new(false);
    let effect_ran = std::cell::Cell::new(false);
    let attempt_error = journal
        .with_merge_attempt(
            &scope(),
            42,
            &second_allocation.allocation_id,
            second_allocation.execution_generation,
            &second_request,
            competing_binding.clone(),
            |_, _| {
                prepare_ran.set(true);
                Ok(())
            },
            |_, _, ()| {
                effect_ran.set(true);
                Ok(MergeAttemptResolutionV1::rejected("fixture"))
            },
        )
        .expect_err("an unresolved merge must fence all PRs to its base branch");

    let recovery_error = journal
        .recover_merge_attempt_if_present(
            &scope(),
            42,
            &second_allocation.allocation_id,
            second_allocation.execution_generation,
            &second_request,
            merge_binding(42, &second_request, competing_pr, &"a".repeat(40)),
            |_| panic!("a different PR cannot recover another branch merge intent"),
        )
        .expect_err("recovery must name the mission and PR owning the unresolved intent");

    assert_eq!(attempt_error.code, "execution_merge_target_unresolved");
    assert_eq!(
        attempt_error.details["blocking_pull_request_number"],
        unresolved_pr
    );
    assert!(attempt_error.action.contains("base branch"));
    assert_eq!(recovery_error.code, "execution_merge_target_unresolved");
    assert!(recovery_error.retryable);
    assert!(
        !prepare_ran.get(),
        "no merge preflight starts while the branch is quarantined"
    );
    assert!(
        !effect_ran.get(),
        "no second merge effect may run for another PR"
    );
    assert_eq!(
        attempt_error.details["blocking_pull_request_number"],
        unresolved_pr
    );

    let inconclusive_recovery = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &first_allocation.allocation_id,
            first_allocation.execution_generation,
            &first_request,
            blocking_binding.clone(),
            |_| {
                Ok(MergeAttemptResolutionV1::inconclusive(
                    "fixture_readback_still_ambiguous",
                ))
            },
        )
        .unwrap()
        .expect("the original mission can retry authenticated readback");
    assert_eq!(
        inconclusive_recovery.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Inconclusive
    );
    let still_blocked = journal
        .with_merge_attempt(
            &scope(),
            42,
            &second_allocation.allocation_id,
            second_allocation.execution_generation,
            &second_request,
            competing_binding.clone(),
            |_, _| panic!("a nonterminal readback must keep the base branch fenced"),
            |_, _, ()| panic!("a nonterminal readback must not permit another PR merge"),
        )
        .unwrap_err();
    assert_eq!(still_blocked.code, "execution_merge_target_unresolved");
    assert!(still_blocked.retryable);

    let terminal_recovery = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &first_allocation.allocation_id,
            first_allocation.execution_generation,
            &first_request,
            blocking_binding,
            |_| {
                Ok(MergeAttemptResolutionV1::rejected(
                    "fixture_merge_not_applied",
                ))
            },
        )
        .unwrap()
        .expect("a conclusive readback resolves the original merge intent");
    assert_eq!(
        terminal_recovery.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Rejected
    );

    let permitted = journal
        .with_merge_attempt(
            &scope(),
            42,
            &second_allocation.allocation_id,
            second_allocation.execution_generation,
            &second_request,
            competing_binding,
            |mode, _| {
                assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
                prepare_ran.set(true);
                Ok(())
            },
            |mode, _, ()| {
                assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
                effect_ran.set(true);
                Ok(MergeAttemptResolutionV1::rejected("fixture_second_pr_safe"))
            },
        )
        .unwrap();
    assert_eq!(
        permitted.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Rejected
    );
    assert!(prepare_ran.get());
    assert!(effect_ran.get());

    let events = journal.replay().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_started_v1")
            .count(),
        2,
        "the other PR is admitted only after conclusive resolution of the blocker"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
            .count(),
        2
    );
}

#[test]
fn concurrent_merge_intents_for_different_prs_on_one_base_have_one_effect() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (first_journal, first_allocation, first_request) = allocation_and_claim(&path);
    let (_, second_allocation, second_request) = allocation_and_claim_for_issue(&path, 42);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let effects_started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut workers = Vec::new();

    for (issue, allocation, request, pull_request_number) in [
        (41, first_allocation, first_request, 27),
        (42, second_allocation, second_request, 28),
    ] {
        let path = path.clone();
        let barrier = barrier.clone();
        let effects_started = effects_started.clone();
        let prepare_barrier_waited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        workers.push(thread::spawn(move || {
            let journal = MissionExecutionJournalV1::open(&path);
            retry_mission_busy(|| {
                let barrier_for_prepare = barrier.clone();
                let wait_on_prepare = prepare_barrier_waited.clone();
                journal.with_merge_attempt(
                    &scope(),
                    issue,
                    &allocation.allocation_id,
                    allocation.execution_generation,
                    &request,
                    merge_binding(issue, &request, pull_request_number, &"a".repeat(40)),
                    move |_, _| {
                        // Both callers pass their initial journal snapshot before
                        // either one can append a durable intent. Retry only when
                        // the journal lock was busy; do not wait on the barrier twice.
                        if !wait_on_prepare.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            barrier_for_prepare.wait();
                        }
                        Ok(())
                    },
                    |_, _, ()| {
                        effects_started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                            "fixture_process_interrupted",
                            true,
                            "recover the unresolved merge intent before starting another merge",
                            json!({}),
                        ))
                    },
                )
            })
        }));
    }

    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    let unresolved_refusals = results
        .iter()
        .filter(|result| {
            result
                .as_ref()
                .err()
                .is_some_and(|error| error.code == "execution_merge_target_unresolved")
        })
        .count();
    let simulated_interruptions = results
        .iter()
        .filter(|result| {
            result
                .as_ref()
                .err()
                .is_some_and(|error| error.code == "fixture_process_interrupted")
        })
        .count();

    assert_eq!(effects_started.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(unresolved_refusals, 1, "results={results:?}");
    assert_eq!(simulated_interruptions, 1);
    let refusal = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .find(|error| error.code == "execution_merge_target_unresolved")
        .expect("one competing PR is refused with the blocking PR identified");
    assert!(refusal.retryable);
    let requested_pr = refusal.details["pull_request_number"]
        .as_u64()
        .expect("the refused PR number is reported");
    let blocking_pr = refusal.details["blocking_pull_request_number"]
        .as_u64()
        .expect("the unresolved PR number is reported");
    assert!(matches!((requested_pr, blocking_pr), (27, 28) | (28, 27)));
    assert_eq!(
        first_journal
            .replay()
            .unwrap()
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_started_v1")
            .count(),
        1
    );
}

#[test]
fn same_pr_with_changed_binding_is_rejected() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let first_binding = merge_binding(41, &request, 19, &"a".repeat(40));

    journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            first_binding,
            |_, _| Ok(()),
            |_, _, ()| {
                Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                    "fixture_transport_interrupted",
                    true,
                    "fixture leaves the intent unresolved",
                    json!({}),
                ))
            },
        )
        .unwrap_err();

    let changed_binding = merge_binding(41, &request, 19, &"b".repeat(40));
    let result = journal.with_merge_attempt(
        &scope(),
        41,
        &allocation.allocation_id,
        allocation.execution_generation,
        &request,
        changed_binding,
        |_, _| panic!("a changed binding must be rejected before preparation"),
        |_, _, ()| panic!("a changed binding must be rejected before readback"),
    );

    assert_eq!(result.unwrap_err().code, "merge_attempt_binding_mismatch");
}

#[test]
fn resolved_merge_attempt_replays_saved_result() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 20, &"a".repeat(40));
    let saved = MergeAttemptResolutionV1::merged(&"1".repeat(40), &"2".repeat(64), &"3".repeat(64));

    let first = journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding.clone(),
            |mode, _| {
                assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
                Ok(())
            },
            |mode, _, ()| {
                assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
                Ok(saved.clone())
            },
        )
        .unwrap();
    let replay = journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |_, _| panic!("a resolved attempt must replay without preparing a new effect"),
            |_, _, ()| panic!("a resolved attempt must return its saved result without effects"),
        )
        .unwrap();

    assert_eq!(first, saved);
    assert_eq!(replay, saved);
    let events = journal.replay().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_started_v1")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
            .count(),
        1
    );
}

#[test]
fn merge_preflight_rejection_does_not_create_an_attempt_intent() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 21, &"a".repeat(40));

    let result = journal.with_merge_attempt(
        &scope(),
        41,
        &allocation.allocation_id,
        allocation.execution_generation,
        &request,
        binding,
        |mode, _| {
            assert_eq!(mode, MergeAttemptModeV1::NewAttempt);
            Err::<(), _>(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                "fixture_preflight_rejected",
                false,
                "repair the candidate and request a new authorized generation",
                json!({}),
            ))
        },
        |_, _, _| panic!("a rejected preflight must not reach the post-intent effect"),
    );

    assert_eq!(result.unwrap_err().code, "fixture_preflight_rejected");
    let events = journal.replay().unwrap();
    assert_eq!(
        events.len(),
        2,
        "failed preparation must leave no merge intent"
    );
}

#[test]
fn rejected_merge_attempt_replays_the_same_host_decision() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 22, &"a".repeat(40));
    let rejected = MergeAttemptResolutionV1::rejected("forgejo_merge_refused").with_retryable(true);

    let first = journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding.clone(),
            |_, _| Ok(()),
            |_, _, ()| Ok(rejected.clone()),
        )
        .unwrap();
    let replay = journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |_, _| panic!("a saved resolution must not prepare another attempt"),
            |_, _, ()| panic!("a saved resolution must not repeat the effect"),
        )
        .unwrap();

    assert_eq!(first, rejected);
    assert_eq!(replay, rejected);
    assert!(
        replay.is_retryable(),
        "replay preserves the host retryability decision"
    );
}

#[test]
fn legacy_inconclusive_resolution_is_nonterminal_even_with_a_persisted_terminal_default() {
    let legacy: MergeAttemptResolutionV1 = serde_json::from_value(json!({
        "schema_version": 1,
        "outcome": "inconclusive",
        "terminal": true,
        "reason_code": "legacy_readback_unavailable",
        "merge_commit": null,
        "merge_tree_manifest_sha256": null,
        "observation_sha256": null
    }))
    .unwrap();

    assert!(!legacy.is_terminal());
    assert_eq!(legacy.retryable, None);
    assert!(legacy.is_retryable());
}

#[test]
fn legacy_inconclusive_resolution_requires_readback_before_a_new_attempt() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 27, &"a".repeat(40));

    journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding.clone(),
            |_, _| Ok(()),
            |_, _, ()| {
                Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                    "fixture_interrupted_after_intent",
                    true,
                    "the fixture leaves the durable merge intent unresolved",
                    json!({}),
                ))
            },
        )
        .unwrap_err();
    let attempt_id = append_legacy_inconclusive_merge_resolution(&path);

    let blocked = journal.allocate_generation(
        &scope(),
        41,
        &request.requirement_sha256,
        "legacy-inconclusive-must-recover-first",
    );
    assert_eq!(
        blocked.unwrap_err().code,
        "execution_merge_attempt_unresolved"
    );

    let recovered = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |_| {
                Ok(MergeAttemptResolutionV1::merged(
                    &"1".repeat(40),
                    &"2".repeat(64),
                    &"3".repeat(64),
                ))
            },
        )
        .unwrap()
        .expect("legacy inconclusive result triggers authenticated readback");
    assert_eq!(
        recovered.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Merged
    );

    let events = journal.replay().unwrap();
    let resolutions: Vec<_> = events
        .iter()
        .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
        .collect();
    assert_eq!(
        resolutions.len(),
        2,
        "legacy observation is retained then finalized"
    );
    assert_eq!(
        resolutions[1].idempotency_key,
        format!("{attempt_id}:resolved-after-recovery")
    );
    journal
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "legacy-inconclusive-recovered-generation-two",
        )
        .expect("terminal authenticated recovery releases the mission fence");
}

#[test]
fn existing_merge_intent_recovers_without_creating_a_second_intent() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 23, &"a".repeat(40));
    journal
        .with_merge_attempt(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding.clone(),
            |_, _| Ok(()),
            |_, _, ()| {
                Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                    "fixture_interrupted_after_intent",
                    true,
                    "resolve the existing intent through authenticated readback",
                    json!({}),
                ))
            },
        )
        .unwrap_err();

    let recovered = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding.clone(),
            |_| {
                let events = MissionExecutionJournalV1::open(&path).replay().unwrap();
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event.kind == "mission_merge_attempt_started_v1")
                        .count(),
                    1
                );
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
                        .count(),
                    0
                );
                Ok(MergeAttemptResolutionV1::inconclusive(
                    "fixture_pr_still_open",
                ))
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(
        recovered,
        MergeAttemptResolutionV1::inconclusive("fixture_pr_still_open")
    );
    let conclusive = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |_| {
                Ok(MergeAttemptResolutionV1::merged(
                    &"1".repeat(40),
                    &"2".repeat(64),
                    &"3".repeat(64),
                ))
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        conclusive.outcome,
        agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Merged
    );
    assert_eq!(
        journal
            .replay()
            .unwrap()
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_resolved_v1")
            .count(),
        1
    );

    let replay = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            merge_binding(41, &request, 23, &"a".repeat(40)),
            |_| panic!("a durable terminal resolution must replay without another readback"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(replay, conclusive);
    assert_eq!(
        journal
            .replay()
            .unwrap()
            .iter()
            .filter(|event| event.kind == "mission_merge_attempt_started_v1")
            .count(),
        1
    );
}

#[test]
fn merge_recovery_without_intent_is_a_no_op() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let (journal, allocation, request) = allocation_and_claim(&path);
    let binding = merge_binding(41, &request, 24, &"a".repeat(40));

    let recovered = journal
        .recover_merge_attempt_if_present(
            &scope(),
            41,
            &allocation.allocation_id,
            allocation.execution_generation,
            &request,
            binding,
            |_| panic!("no intent means no recovery readback callback"),
        )
        .unwrap();

    assert!(recovered.is_none());
    assert_eq!(journal.replay().unwrap().len(), 2);
}

#[test]
fn malformed_merge_transition_blocks_future_admission() {
    let scratch = ScratchDir::new();
    let path = scratch.path().join("missions.jsonl");
    let request = request(1);
    let identity =
        derive_mission_identity_v1(&scope(), 41, &request.requirement_sha256, 1).unwrap();
    append_raw_event(
        &path,
        "mission-merge-attempt-v1:malformed",
        "mission_merge_attempt_started_v1",
        json!({
            "schema_version": 1,
            "mission_id": identity.mission_id,
            "attempt_id": "missing-bindings",
        }),
    );

    let error = MissionExecutionJournalV1::open(&path)
        .allocate_generation(
            &scope(),
            41,
            &request.requirement_sha256,
            "after-malformed-merge",
        )
        .unwrap_err();

    assert_eq!(error.code, "merge_attempt_journal_invalid");
    assert_eq!(
        MissionExecutionJournalV1::open(&path)
            .replay()
            .unwrap()
            .len(),
        1
    );
}

#[test]
#[ignore = "invoked as a subprocess by the concurrent claim contract"]
fn claim_child_waits_for_start_file_then_claims() {
    let (Ok(journal), Ok(start)) = (
        std::env::var("AGENT_FORGE_CLAIM_CHILD_JOURNAL"),
        std::env::var("AGENT_FORGE_CLAIM_CHILD_START"),
    ) else {
        return;
    };
    println!("READY");
    std::io::stdout().flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !Path::new(&start).exists() {
        if Instant::now() >= deadline {
            panic!("parent did not release concurrent claim children");
        }
        thread::sleep(Duration::from_millis(5));
    }
    let marker = match claim(Path::new(&journal), 1, &request(1)).unwrap() {
        ExecutionClaimResultV1::Claimed(_) => "CLAIMED",
        ExecutionClaimResultV1::AlreadyClaimed(_) => "DUPLICATE",
    };
    println!("{marker}");
    std::io::stdout().flush().unwrap();
}
