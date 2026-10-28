use agent_forge_core::{
    ContextManifest, ContextSource, EventJournal,
    agent_runtime::{
        AgentExecutionRequestV1, ContextCitationV1, ContextPacketV1, MissionExecutionJournalV1,
        MissionExecutionLeaseV1, RuntimeBudgetV1, RuntimeCapabilitiesV1, WorkspaceRefV1,
        derive_mission_identity_v1,
    },
    forgejo::BaseBranchScope,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mission-verification-cpu-budget-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn file(&self) -> PathBuf {
        self.0.join("mission.jsonl")
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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

fn request(generation: u64) -> AgentExecutionRequestV1 {
    let requirements_json = serde_json::to_string(&json!({
        "schema_version":2,
        "id":"REQ-CPU-BUDGET-001",
        "revision":1,
        "source_issue_number":41,
        "request":"Run the pinned acceptance verifier.",
        "acceptance":["The verifier result is bound to this mission."],
        "ambiguities":[]
    }))
    .unwrap();
    let requirement_sha256 = hex::encode(Sha256::digest(requirements_json.as_bytes()));
    let identity =
        derive_mission_identity_v1(&scope(), 41, &requirement_sha256, generation).unwrap();
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
            id: "cpu-budget-workspace".to_owned(),
            base_commit,
        },
        capabilities: RuntimeCapabilitiesV1 {
            read_paths: vec!["src/lib.rs".to_owned()],
            write_paths: vec!["src/lib.rs".to_owned()],
            tools: vec!["cargo.test".to_owned()],
            budget: RuntimeBudgetV1 {
                max_tool_calls: 12,
                timeout_ms: 10_000,
                max_output_tokens: 512,
            },
        },
    }
}

fn acquire_lease(
    journal_path: &Path,
    generation: u64,
    lease_id: &str,
    now_unix_ms: u64,
) -> MissionExecutionLeaseV1 {
    let journal = MissionExecutionJournalV1::open(journal_path);
    let request = request(generation);
    let allocation_id = format!("cpu-budget-allocation-{generation}");
    let allocation = journal
        .allocate_generation(&scope(), 41, &request.requirement_sha256, &allocation_id)
        .unwrap();
    assert_eq!(allocation.execution_generation, generation);
    journal
        .claim_allocated(&scope(), 41, generation, &allocation_id, &request)
        .unwrap();
    journal
        .acquire_lease(
            &scope(),
            41,
            generation,
            &allocation_id,
            lease_id,
            "cpu-budget-host",
            || now_unix_ms,
            1_000,
            &request,
        )
        .unwrap()
}

fn policy_sha256() -> String {
    "a".repeat(64)
}

#[test]
fn current_lease_lookup_replays_and_validates_the_durable_lease() {
    let scratch = ScratchDir::new();
    let path = scratch.file();
    let lease = acquire_lease(&path, 1, "cpu-budget-lookup-lease", 1_000);
    let journal = MissionExecutionJournalV1::open(&path);

    let loaded = journal
        .current_lease_by_id(&scope(), 41, &lease.lease_id, "cpu-budget-host", 1_500)
        .unwrap();
    assert_eq!(loaded, lease);

    let wrong_holder = journal
        .current_lease_by_id(&scope(), 41, &lease.lease_id, "other-host", 1_500)
        .unwrap_err();
    assert_eq!(wrong_holder.code, "execution_lease_holder_mismatch");

    let expired = journal
        .current_lease_by_id(&scope(), 41, &lease.lease_id, "cpu-budget-host", 2_000)
        .unwrap_err();
    assert_eq!(expired.code, "execution_lease_expired");
}

#[test]
fn ledger_summary_reports_unresolved_reservations_at_full_cost_and_resolved_usage() {
    let scratch = ScratchDir::new();
    let path = scratch.file();
    let lease = acquire_lease(&path, 1, "cpu-budget-summary-lease", 1_000);
    let journal = MissionExecutionJournalV1::open(&path);
    journal
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease,
            "cpu-budget-host",
            1_100,
            &policy_sha256(),
            100,
            "cpu-budget-summary-operation",
            60,
        )
        .unwrap();

    let pending = journal.verification_cpu_ledger_summary(&lease).unwrap();
    assert_eq!(pending.charged_cpu_ms, 60);
    assert_eq!(pending.remaining_cpu_ms, 40);
    assert_eq!(pending.reservations.len(), 1);
    assert_eq!(pending.reservations[0].charged_cpu_ms, 60);
    assert!(!pending.reservations[0].resolved);

    journal
        .resolve_mission_verification_cpu(&lease, "cpu-budget-summary-operation", 20)
        .unwrap();
    let resolved = journal.verification_cpu_ledger_summary(&lease).unwrap();
    assert_eq!(resolved.charged_cpu_ms, 20);
    assert_eq!(resolved.remaining_cpu_ms, 80);
    assert_eq!(resolved.reservations[0].charged_cpu_ms, 20);
    assert!(resolved.reservations[0].resolved);
}

#[test]
fn completed_cpu_usage_is_cumulative_across_generations_and_journal_reopen() {
    let scratch = ScratchDir::new();
    let path = scratch.file();
    let lease1 = acquire_lease(&path, 1, "cpu-budget-lease-1", 1_000);
    let journal = MissionExecutionJournalV1::open(&path);

    let reservation1 = journal
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease1,
            "cpu-budget-host",
            1_100,
            &policy_sha256(),
            100,
            "cpu-budget-operation-1",
            60,
        )
        .unwrap();
    assert!(!reservation1.replayed);
    journal
        .resolve_mission_verification_cpu(&lease1, "cpu-budget-operation-1", 20)
        .unwrap();

    let lease2 = acquire_lease(&path, 2, "cpu-budget-lease-2", 3_000);
    let reopened = MissionExecutionJournalV1::open(&path);
    let reservation2 = reopened
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease2,
            "cpu-budget-host",
            3_100,
            &policy_sha256(),
            100,
            "cpu-budget-operation-2",
            80,
        )
        .unwrap();
    assert!(!reservation2.replayed);
    reopened
        .resolve_mission_verification_cpu(&lease2, "cpu-budget-operation-2", 80)
        .unwrap();

    let error = reopened
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease2,
            "cpu-budget-host",
            3_200,
            &policy_sha256(),
            100,
            "cpu-budget-operation-3",
            1,
        )
        .unwrap_err();
    assert_eq!(error.code, "mission_verification_cpu_budget_exhausted");
    assert_eq!(error.details["charged_cpu_ms"], 100);
    assert_eq!(error.details["mission_cpu_budget_ms"], 100);
}

#[test]
fn unresolved_reservation_is_charged_at_its_full_bound_after_restart() {
    let scratch = ScratchDir::new();
    let path = scratch.file();
    let lease1 = acquire_lease(&path, 1, "cpu-budget-lease-1", 1_000);
    MissionExecutionJournalV1::open(&path)
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease1,
            "cpu-budget-host",
            1_100,
            &policy_sha256(),
            60,
            "cpu-budget-interrupted-operation",
            40,
        )
        .unwrap();

    let lease2 = acquire_lease(&path, 2, "cpu-budget-lease-2", 3_000);
    let reopened = MissionExecutionJournalV1::open(&path);
    let error = reopened
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease2,
            "cpu-budget-host",
            3_100,
            &policy_sha256(),
            60,
            "cpu-budget-after-restart",
            21,
        )
        .unwrap_err();
    assert_eq!(error.code, "mission_verification_cpu_budget_exhausted");
    assert_eq!(error.details["charged_cpu_ms"], 40);
    assert_eq!(error.details["requested_cpu_ms"], 21);
}

#[test]
fn duplicate_operation_id_cannot_replay_a_verification_effect() {
    let scratch = ScratchDir::new();
    let path = scratch.file();
    let lease = acquire_lease(&path, 1, "cpu-budget-lease-1", 1_000);
    let journal = MissionExecutionJournalV1::open(&path);

    journal
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease,
            "cpu-budget-host",
            1_100,
            &policy_sha256(),
            100,
            "cpu-budget-repeat",
            60,
        )
        .unwrap();
    let error = journal
        .reserve_mission_verification_cpu(
            &scope(),
            41,
            &lease,
            "cpu-budget-host",
            1_200,
            &policy_sha256(),
            100,
            "cpu-budget-repeat",
            60,
        )
        .unwrap_err();
    assert_eq!(error.code, "mission_verification_cpu_reservation_replayed");
}

#[test]
fn concurrent_reservations_cannot_exceed_the_shared_mission_budget() {
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::{Duration, Instant},
    };

    let scratch = ScratchDir::new();
    let path = Arc::new(scratch.file());
    let lease = acquire_lease(&path, 1, "cpu-budget-lease-1", 1_000);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for sequence in 1..=2 {
        let path = Arc::clone(&path);
        let lease = lease.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            let journal = MissionExecutionJournalV1::open(path.as_path());
            barrier.wait();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let result = journal.reserve_mission_verification_cpu(
                    &scope(),
                    41,
                    &lease,
                    "cpu-budget-host",
                    1_100,
                    &policy_sha256(),
                    100,
                    &format!("cpu-budget-concurrent-{sequence}"),
                    60,
                );
                match result {
                    Err(error)
                        if error.code == "execution_mission_busy" && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(2));
                    }
                    result => break result,
                }
            }
        }));
    }
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .map(|error| error.code.as_str())
            .collect::<Vec<_>>(),
        ["mission_verification_cpu_budget_exhausted"]
    );
}

#[test]
fn generic_journal_append_cannot_forge_cpu_accounting_events() {
    let scratch = ScratchDir::new();
    let error = EventJournal::open(scratch.file())
        .append(
            "untrusted-cpu-reservation",
            "mission_verification_cpu_reserved_v1",
            json!({"mission_id":"forged"}),
        )
        .unwrap_err();
    assert_eq!(error, agent_forge_core::JournalError::InvalidEvent);
}
