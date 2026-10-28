use agent_forge_core::webhook::{
    WEBHOOK_EVENT_CONTRACTS_V1, WEBHOOK_EVENT_CONTRACTS_V2, WEBHOOK_EVENT_CONTRACTS_V3,
    WEBHOOK_EVENT_CONTRACTS_V4, WEBHOOK_EVENT_CONTRACTS_V5, WEBHOOK_EVENT_CONTRACTS_V6,
    WEBHOOK_EVENT_CONTRACTS_V7, WEBHOOK_EVENT_CONTRACTS_V8, WEBHOOK_EVENT_CONTRACTS_V9,
};
use serde_json::{Value, json};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::ffi::OsStrExt,
    process::Child,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
struct ScratchDir(std::path::PathBuf);

#[cfg(unix)]
impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "forgejo-capabilities-fifo-e2e-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create private FIFO test directory");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg(unix)]
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn wait_bounded(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll CLI process") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop CLI that exceeded the FIFO bound");
            child.wait().expect("reap timed-out CLI");
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn spawn_commit_status_http_fixture(sha: String) -> thread::JoinHandle<Vec<String>> {
    let listener = TcpListener::bind(("127.0.0.1", 3130))
        .expect("bind the adapter's fixed loopback origin for this isolated process test");
    listener
        .set_nonblocking(true)
        .expect("make fixture accept bounded");
    thread::spawn(move || {
        let mut first_lines = Vec::new();
        for request_number in 0..4 {
            let accept_deadline = Instant::now() + Duration::from_secs(15);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < accept_deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept bounded loopback request: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("bound request read time");
            let mut request_bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream
                    .read(&mut chunk)
                    .expect("read bounded HTTP request headers");
                if read == 0 {
                    break;
                }
                request_bytes.extend_from_slice(&chunk[..read]);
                assert!(request_bytes.len() <= 16 * 1024, "bounded HTTP headers");
            }
            let request = String::from_utf8(request_bytes).expect("fixture request is ASCII");
            let first_line = request
                .lines()
                .next()
                .expect("HTTP request line")
                .to_owned();
            let page = if request_number % 2 == 0 { 1 } else { 2 };
            let expected_path = format!(
                "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page={page} HTTP/1.1"
            );
            assert_eq!(first_line, expected_path);
            assert!(
                !request.to_ascii_lowercase().contains("\r\nauthorization:"),
                "the no-credential fixture request must stay anonymous"
            );
            first_lines.push(first_line);

            let statuses = if page == 1 {
                (0..50)
                    .map(|index| json!({"status":"success","context":format!("ci/{index}")}))
                    .collect::<Vec<_>>()
            } else {
                vec![json!({"status":"failure","context":"ci/security"})]
            };
            let page_state = if page == 1 { "success" } else { "failure" };
            let x_total_count = if request_number == 3 { 52 } else { 51 };
            let body = serde_json::to_vec(&json!({
                "state": page_state,
                "sha": sha,
                "total_count": statuses.len(),
                "statuses": statuses,
                "repository": {"full_name":"forge-admin/agent-forge-core"},
            }))
            .expect("serialize fixture status page");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Total-Count: {x_total_count}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write fixture response headers");
            stream
                .write_all(&body)
                .expect("write fixture response body");
            stream
                .shutdown(std::net::Shutdown::Write)
                .expect("finish fixture response");
        }
        first_lines
    })
}

#[cfg(unix)]
fn spawn_pull_request_status_http_fixture(
    initial_head_sha: String,
    head_sha_after_statuses: String,
    statuses_present: bool,
) -> thread::JoinHandle<Vec<String>> {
    let listener = TcpListener::bind(("127.0.0.1", 3130))
        .expect("bind the adapter's fixed loopback origin for this isolated process test");
    listener
        .set_nonblocking(true)
        .expect("make fixture accept bounded");
    thread::spawn(move || {
        let mut requests = Vec::new();
        for request_number in 0..3 {
            let accept_deadline = Instant::now() + Duration::from_secs(15);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < accept_deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept bounded pull request status request: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("bound pull request status fixture read time");
            let mut request_bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream
                    .read(&mut chunk)
                    .expect("read bounded pull request status HTTP headers");
                if read == 0 {
                    break;
                }
                request_bytes.extend_from_slice(&chunk[..read]);
                assert!(request_bytes.len() <= 16 * 1024, "bounded HTTP headers");
            }
            let request = String::from_utf8(request_bytes).expect("fixture request is ASCII");
            let first_line = request
                .lines()
                .next()
                .expect("HTTP request line")
                .to_owned();
            let status_path = format!(
                "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{initial_head_sha}/status?limit=50&page=1 HTTP/1.1"
            );
            let expected_path = match request_number {
                0 | 2 => "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1",
                1 => status_path.as_str(),
                _ => unreachable!(),
            };
            assert_eq!(first_line, expected_path);
            assert!(
                !request.to_ascii_lowercase().contains("\r\nauthorization:"),
                "the anonymous fixture request must not carry a credential"
            );
            requests.push(first_line);

            let body = if request_number == 1 && statuses_present {
                json!({
                    "state":"success",
                    "sha":initial_head_sha,
                    "total_count":1,
                    "statuses":[{"status":"success","context":"ci/test"}],
                    "repository":{"full_name":"forge-admin/agent-forge-core"},
                })
            } else if request_number == 1 {
                json!({
                    "state":"",
                    "sha":"",
                    "total_count":0,
                    "statuses":null,
                    "repository":null,
                })
            } else {
                let head_sha = if request_number == 0 {
                    &initial_head_sha
                } else {
                    &head_sha_after_statuses
                };
                json!({
                    "number":23,
                    "html_url":"http://127.0.0.1:3130/forge-admin/agent-forge-core/pulls/23",
                    "state":"open",
                    "merged":false,
                    "draft":false,
                    "mergeable":true,
                    "base":{
                        "ref":"main",
                        "sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "repo":{"full_name":"forge-admin/agent-forge-core"},
                    },
                    "head":{"sha":head_sha},
                })
            };
            let body = serde_json::to_vec(&body).expect("serialize PR status fixture body");
            let total_count_header = if request_number == 1 && statuses_present {
                "X-Total-Count: 1\r\n"
            } else if request_number == 1 {
                "X-Total-Count: 0\r\n"
            } else {
                ""
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{total_count_header}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write fixture response headers");
            stream
                .write_all(&body)
                .expect("write fixture response body");
            stream
                .shutdown(std::net::Shutdown::Write)
                .expect("finish fixture response");
        }
        requests
    })
}

#[test]
fn actual_cli_discloses_versioned_local_test_capabilities() {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .arg("--capabilities")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual local Forgejo adapter");

    assert!(
        output.status.success(),
        "capability discovery failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("capability discovery returns structured JSON");
    assert_eq!(value["schema_version"], 28);
    assert_eq!(value["api_version"], "forgejo-reconcile-capabilities-v28");
    assert_eq!(value["adapter"], "forgejo-reconcile-local");
    let status_snapshot = value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["id"] == "read_commit_status_snapshot")
        .unwrap();
    assert_eq!(
        status_snapshot["accepted_status_values"],
        json!([
            "pending", "success", "failure", "error", "warning", "skipped"
        ])
    );
    assert_eq!(status_snapshot["independent_verification"], false);
    assert_eq!(status_snapshot["merge_authorizing"], false);
    let reconcile = value["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["id"] == "reconcile_webhook_deliveries")
        .unwrap();
    assert_eq!(
        reconcile["reconcile_only_profile_selector"],
        "--reconcile-target-profile"
    );
    assert_eq!(
        reconcile["reconcile_only_profiles"],
        json!([{
            "selector": "org-team-review-e2e",
            "origin": "http://127.0.0.1:3130",
            "owner": "forge-team-org",
            "repository": "team-review-readback",
            "merge_target": false,
            "forgejo_writes": false,
        }])
    );
    assert_eq!(
        reconcile["current_api_identity_contract"],
        json!({
            "issue_repository": {
                "field": "repository",
                "accepted_identity_fields": ["full_name", "name", "owner"],
                "when_object_present": "require_at_least_one_identity_field_and_validate_every_supplied_value",
                "missing_or_null": "allowed_as_partial_api_response",
            },
            "pull_request_base_repository": {
                "field": "base.repo",
                "accepted_identity_fields": ["full_name", "name", "owner.login"],
                "when_object_present": "require_at_least_one_identity_field_and_validate_every_supplied_value",
                "missing_or_null": "allowed_as_partial_api_response",
            },
            "pull_request_head_repository": {
                "target_repository_must_match": false,
                "foreign_forks_allowed": true,
            },
        })
    );
    assert_eq!(
        reconcile["work_item_assignees_projection"]["source"],
        "current_forgejo_resource_api"
    );
    assert_eq!(
        reconcile["work_item_assignees_projection"]["ordering"],
        "ascending_user_id"
    );
    assert_eq!(
        reconcile["work_item_assignees_projection"]["webhook_assignees"],
        "never_used_as_canonical_context"
    );
    assert_eq!(
        reconcile["work_item_assignees_projection"]["missing_field"],
        "null_unknown"
    );
    assert_eq!(
        reconcile["work_item_assignees_projection"]["null_by_resource"],
        json!({
            "issue": "known_empty",
            "issue_comment_parent": "known_empty",
            "pull_request": "null_unknown",
            "pull_request_comment_parent": "null_unknown",
        })
    );
    assert_eq!(
        value["process_lifecycle"]["curl_child_parent_exit"],
        if cfg!(target_os = "linux") {
            "sigkill_via_linux_pr_set_pdeathsig"
        } else {
            "not_guaranteed"
        }
    );
    assert_eq!(
        value["credential_boundary"],
        json!({
            "secret_bytes_in_argv_environment_journal_or_response": false,
            "secret_material_resides_in": [
                "adapter_process_memory",
                "curl_child_stdin_and_memory",
            ],
            "credential_fd_validation": {
                "descriptor_shape": "anonymous_pipe_checked",
                "content": ["bounded_read_to_eof", "token_syntax_validated"],
                "maximum_input_bytes": 4096,
                "read_timeout_ms": 2000,
                "producer_identity": "not_authenticated_by_cli",
                "exclusive_ownership": "not_verified_by_cli",
            },
            "pipe_read_failure": {
                "structured_refusal_code": "host_credential_pipe_read_failed",
                "result": "inconclusive_without_upstream_read",
                "source_binding": "journal_source_for_single_or_selected_event_sources_for_batch",
                "credential_in_response": false,
            },
            "agent_process_isolation": "not_enforced_by_cli",
            "required_caller_boundary": "invoke_credential_bearing_adapter_in_trusted_host_process_outside_agent_execution_scope",
        })
    );
    assert_eq!(value["configuration"]["schema_version"], 1);
    assert_eq!(
        value["configuration"]["allowed_origins"],
        json!(["http://127.0.0.1:3130"])
    );
    assert_eq!(value["configuration"]["repository"]["owner"], "forge-admin");
    assert_eq!(
        value["configuration"]["repository"]["name"],
        "agent-forge-core"
    );
    assert_eq!(
        value["configuration"]["authentication"]["reconcile_webhook_deliveries"],
        "optional_inherited_anonymous_pipe_fd"
    );
    assert_eq!(
        value["configuration"]["authentication"]["read_only_merge_preflight"],
        "anonymous_only"
    );
    assert_eq!(
        value["configuration"]["authentication"]["local_test_merge_authority"],
        "inherited_anonymous_pipe_fd"
    );
    assert_eq!(
        value["configuration"]["authentication"]["local_test_merge_policy_reader"],
        "optional_exact_route_inherited_anonymous_pipe_fd"
    );

    let operations = value["operations"].as_array().expect("operations array");
    let pull_request_status_snapshot = operations
        .iter()
        .find(|operation| operation["id"] == "read_pull_request_status_snapshot")
        .expect("pull request status snapshot capability");
    assert_eq!(
        pull_request_status_snapshot["schema_version"],
        "forgejo-pull-request-status-snapshot-v1"
    );
    assert_eq!(
        pull_request_status_snapshot["status_pagination"]["complete_multi_page_recheck"],
        "bounded_second_pass_rejects_changed_status_pages_or_combined_state"
    );
    assert_eq!(
        pull_request_status_snapshot["pull_request_head_binding"],
        "server_read_before_and_after_status_pages;changed_binding_is_inconclusive"
    );
    assert_eq!(
        pull_request_status_snapshot["pull_request_head_fields_rechecked"],
        json!([
            "number",
            "state",
            "merged",
            "base_branch",
            "base_sha",
            "head_sha"
        ])
    );
    assert_eq!(
        pull_request_status_snapshot["binding_change_fields"],
        json!([
            "number",
            "state",
            "merged",
            "base_branch",
            "base_sha",
            "head_sha"
        ])
    );
    assert_eq!(
        pull_request_status_snapshot["binding_stability_semantics"],
        "all_listed_fields_must_match;head_sha_stability_is_reported_separately"
    );
    assert_eq!(
        pull_request_status_snapshot["pull_request_identity_scope"],
        "configured_repository_and_canonical_pull_request_url"
    );
    assert_eq!(
        pull_request_status_snapshot["possible_observations"],
        json!(["complete", "no_statuses", "inconclusive"])
    );
    assert_eq!(
        pull_request_status_snapshot["empty_status_policy"],
        "no_statuses_is_not_verification_success_requires_independent_verifier"
    );
    assert_eq!(
        pull_request_status_snapshot["incomplete_status_policy"],
        "inconclusive_requires_complete_reread"
    );
    assert_eq!(
        pull_request_status_snapshot["independent_verification"],
        false
    );
    assert_eq!(pull_request_status_snapshot["merge_authorizing"], false);
    let reconcile = operations
        .iter()
        .find(|operation| operation["id"] == "reconcile_webhook_deliveries")
        .expect("webhook reconciliation capability");
    assert_eq!(
        reconcile["supported_events"],
        json!([
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
        ])
    );
    assert_eq!(reconcile["event_contracts"]["schema_version"], 1);
    assert_eq!(
        reconcile["event_contracts"]["webhook_payload_is_authoritative"],
        false
    );
    assert_eq!(
        reconcile["event_contracts"]["contracts"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V1.contracts).unwrap()
    );
    assert_eq!(
        reconcile["event_contracts_v2"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V2).unwrap()
    );
    assert_eq!(
        reconcile["event_contracts_v2"]["workflow_statuses"]
            .as_array()
            .unwrap()
            .len(),
        14
    );
    assert_eq!(
        reconcile["event_contracts_v3"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V3).unwrap()
    );
    assert_eq!(
        reconcile["event_contracts_v4"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V4).unwrap()
    );
    assert_eq!(
        reconcile["event_contracts_v5"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V5).unwrap()
    );
    assert_eq!(reconcile["event_contracts_v6"]["schema_version"], 6);
    assert_eq!(
        reconcile["event_contracts_v6"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V6).unwrap()
    );
    assert_eq!(
        reconcile["event_contracts_v6"]["event_actions"],
        json!([
            {"event":"issues","payload_field":"action","allowed_actions":["opened","closed","reopened","edited","assigned","unassigned","label_updated","label_cleared","milestoned","demilestoned"]},
            {"event":"issue_comment","payload_field":"action","allowed_actions":["created","edited","deleted"]}
        ])
    );
    assert_eq!(
        reconcile["event_contracts_v7"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V7).unwrap()
    );
    assert_eq!(
        reconcile["event_contracts_v8"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V8).unwrap()
    );
    assert_eq!(reconcile["event_contracts_v8"]["schema_version"], 8);
    assert_eq!(
        reconcile["event_contracts_v9"],
        serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V9).unwrap()
    );
    assert_eq!(reconcile["event_contracts_v9"]["schema_version"], 9);
    assert_eq!(
        reconcile["event_contracts_v7"]["event_actions"][2],
        json!({"event":"pull_request","payload_field":"action","allowed_actions":["opened","closed","reopened","edited","assigned","unassigned","label_updated","label_cleared","milestoned","demilestoned","synchronized","review_requested","review_request_removed"]})
    );
    assert_eq!(reconcile["maximum_events"], 100);
    assert_eq!(reconcile["maximum_cached_json_bytes"], 8 * 1024 * 1024);
    assert_eq!(
        reconcile["cache_budget_measurement"],
        "combined_uncompressed_serialized_json"
    );
    assert_eq!(reconcile["forgejo_writes"], false);
    assert_eq!(reconcile["canonical_state_writes"], false);
    assert_eq!(
        reconcile["credential_source"],
        "optional_inherited_anonymous_pipe_fd_bound_to_configured_origin"
    );
    assert_eq!(
        reconcile["credential_contract"]["credential_boundary_reference"],
        "credential_boundary"
    );
    assert!(
        reconcile
            .get("credential_in_argv_environment_journal_or_agent")
            .is_none()
    );
    assert_eq!(
        reconcile["credential_contract"],
        json!({
            "credential_flag": "--host-credential-fd",
            "source": "optional_inherited_anonymous_pipe_fd_bound_to_configured_origin",
            "trusted_attestation_public_key_flag": "--trusted-webhook-attestation-public-key-hex",
            "maximum_trusted_attestation_public_keys": 8,
            "attestation_required_before_credential_read_for_in_scope_delivery": true,
            "attestation_verification_scope": "all_non_quarantined_selected_inbox_delivery_rows_before_credential_read",
            "uncredentialed_legacy_replay_without_attestation_policy": "compatibility_only",
            "credential_fd_validation_reference": "credential_boundary.credential_fd_validation",
            "credential_boundary_reference": "credential_boundary",
            "private_resource_http_404": {
                "observed_in": "FORGEJO-V17-LIVE-PRIVATE-PR-READBACK-001",
                "result": "inconclusive_no_canonical_state",
                "retryable": false,
                "interpretation": "resource_absent_or_private_visibility_not_granted",
            },
        })
    );
    assert_eq!(reconcile["output_schema_versions"]["single"], 19);
    assert_eq!(reconcile["output_schema_versions"]["batch"], 23);
    assert_eq!(
        reconcile["release_projection"],
        json!({
            "source": "current_forgejo_resource_api",
            "canonical_read": "/api/v1/repos/forge-admin/agent-forge-core/releases/{release_id}",
            "projected_fields": ["id", "tag_name", "draft", "prerelease"],
            "maximum_tag_name_bytes": 255,
            "release_notes_included": false,
            "assets_included": false,
            "release_html_url": "derived_from_configured_origin_repository_and_current_tag_name",
            "missing_release": "inconclusive_resource_not_found",
            "updated_at": "unavailable_in_forgejo_v17_release_response",
            "independent_verification": false,
            "merge_authorizing": false,
        })
    );
    assert_eq!(
        reconcile["work_item_milestone_projection"],
        json!({
            "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
            "source": "current_forgejo_resource_api",
            "observations": {
                "missing_field": "unknown",
                "null": "unassigned",
                "object": "assigned",
            },
            "projected_fields": ["id", "title", "state"],
            "maximum_title_bytes": 1024,
            "malformed_or_over_limit": "inconclusive_invalid_current_state",
            "freshness": "current_api_read",
            "trust": "untrusted_user_content",
            "webhook_milestone": "never_used_as_canonical_context",
        })
    );
    assert_eq!(
        reconcile["pull_request_projection"]["draft"],
        json!({
            "source": "current_forgejo_pull_request_api",
            "projected_fields": ["draft"],
            "required_type": "boolean",
            "missing_or_malformed": "inconclusive_invalid_current_state",
            "freshness": "current_api_read",
            "webhook_draft": "never_used_as_canonical_state",
            "merge_authorizing": false,
        })
    );
    assert_eq!(
        reconcile["workflow_job_parent_run_projection"],
        json!({
            "resource": "workflow_job",
            "source": "second_current_forgejo_api_read",
            "projected_fields": ["run_id", "index_in_repo", "state", "updated_at", "html_url", "head_sha"],
            "state_semantics": "canonical_state.state_is_job_state; canonical_state.parent_workflow_run.state_is_parent_run_state",
            "read_failure": "inconclusive_without_canonical_state",
            "independent_verification": false,
            "merge_authorizing": false,
        })
    );
    assert_eq!(
        reconcile["source_event_provenance_contract"],
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
            "current_state_read_means": "canonical_api_read_only",
        })
    );
    assert_eq!(
        reconcile["review_notification_projection"],
        json!({
            "source": "forgejo_webhook_delivery",
            "projected_fields": [
                "review_type",
                "body",
                "body_truncated",
                "body_byte_length",
                "body_sha256",
                "pull_request_head_sha_at_event",
            ],
            "maximum_body_bytes": 8192,
            "oversized_body": "truncate_at_utf8_boundary",
            "body_byte_length_and_sha256": "complete_source_body_before_truncation",
            "body_sha256_semantics": "identity_only_not_correctness_or_approval",
            "trust": "untrusted_user_content",
            "freshness": "event_snapshot_not_canonical",
            "review_id_binding": "unavailable_in_forgejo_v17_webhook",
            "authorizing": false,
        })
    );
    assert_eq!(
        reconcile["pull_request_projection"]["requested_reviewers"],
        json!({
            "source": "current_forgejo_pull_request_api",
            "projected_fields": ["id", "login"],
            "ordering": "ascending_user_id",
            "maximum_items": 1024,
            "missing_or_null": "null_unknown",
            "empty_array": "known_empty",
            "malformed_or_duplicate_id": "inconclusive_invalid_current_state",
        })
    );
    assert_eq!(
        reconcile["pull_request_projection"]["requested_reviewers_teams"],
        json!({
            "source": "current_forgejo_pull_request_api",
            "projected_fields": ["id", "name"],
            "ordering": "ascending_team_id",
            "maximum_items": 1024,
            "maximum_name_characters": 255,
            "missing_or_null": "null_unknown",
            "empty_array": "known_empty",
            "malformed_or_duplicate_id": "inconclusive_invalid_current_state",
        })
    );
    assert_eq!(
        reconcile["work_item_content_projection"],
        json!({
            "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
            "source": "current_forgejo_resource_api",
            "projected_fields": ["title", "body"],
            "maximum_title_bytes": 1024,
            "maximum_body_bytes": 65_536,
            "missing_title_or_body": "null_unknown",
            "explicit_null_body": "known_null",
            "over_limit_content": "omit_content_and_keep_canonical_state",
            "malformed_field_type_or_title": "inconclusive_invalid_current_state",
            "trust": "untrusted_user_content",
            "webhook_content": "never_used_as_canonical_context",
        })
    );
    assert_eq!(
        reconcile["work_item_labels_projection"],
        json!({
            "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
            "source": "current_forgejo_resource_api",
            "projected_fields": ["id", "name"],
            "ordering": "ascending_label_id",
            "maximum_items": 256,
            "maximum_name_bytes": 255,
            "missing_or_null": "null_unknown",
            "empty_array": "known_empty",
            "malformed_duplicate_or_over_limit": "inconclusive_invalid_current_state",
            "freshness": "current_api_read",
            "trust": "untrusted_user_content",
            "webhook_labels": "never_used_as_canonical_context",
        })
    );

    let preflight = operations
        .iter()
        .find(|operation| operation["id"] == "read_only_merge_preflight")
        .expect("merge preflight capability");
    assert_eq!(preflight["merge_authorized"], false);
    assert_eq!(preflight["merge_attempted"], false);

    let merge = operations
        .iter()
        .find(|operation| operation["id"] == "local_test_merge_authority")
        .expect("host-only local merge capability");
    assert_eq!(merge["profile"], "fixed_loopback_test_only");
    assert_eq!(
        merge["credential_boundary_reference"],
        "credential_boundary"
    );
    assert_eq!(
        merge["credential_source"],
        "inherited_anonymous_pipe_fd_bound_to_fixed_loopback_origin"
    );
    assert_eq!(
        merge["credential_roles"]["merge_actor"],
        "required_inherited_anonymous_pipe_fd"
    );
    assert_eq!(
        merge["credential_fd_validation_reference"],
        "credential_boundary.credential_fd_validation"
    );
    assert!(
        merge
            .get("credential_in_argv_environment_journal_or_agent")
            .is_none()
    );
    assert_eq!(merge["prevents_external_forgejo_writers"], false);
    assert_eq!(merge["post_is_retried"], false);
    assert_eq!(
        merge["merge_attempt_scope_fence"],
        json!({
            "scope": "normalized_origin_owner_repository_base_branch",
            "unresolved_intent_refusal": {
                "error_code": "execution_merge_target_unresolved",
                "verdict": "inconclusive",
                "retryable": true,
                "details_fields": ["mission_id", "pull_request_number", "blocking_pull_request_number"],
                "known_blocker_before_preflight": "refuse_before_forgejo_requests",
                "blocker_detected_before_append": "refuse_before_new_merge_intent_append",
                "action_before_preflight": "recover the blocking pull request through authenticated readback before starting another merge attempt on this base branch",
                "action_before_append": "recover the blocking pull request through authenticated readback before appending another merge attempt on this base branch",
                "attempt_recovery": {
                    "refusal_stage": "before_forgejo_readback",
                    "action": "recover the blocking pull request through authenticated readback under its admitted mission before continuing on this base branch",
                },
            },
            "external_writer_exclusion": "not_provided_by_cooperative_journal_fence",
        })
    );
}

#[test]
fn merge_cli_rejects_reconciliation_only_profile_before_any_merge_input() {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args([
            "--merge-local-test",
            "--reconcile-target-profile",
            "org-team-review-e2e",
        ])
        .stdin(Stdio::null())
        .output()
        .expect("run the actual local Forgejo merge adapter");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "unknown --merge-local-test option; select a target profile only with --target-profile <profile>\n"
    );
}

#[test]
fn merge_cli_rejects_reconciliation_only_value_under_merge_selector() {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args([
            "--merge-local-test",
            "--target-profile",
            "org-team-review-e2e",
        ])
        .stdin(Stdio::null())
        .output()
        .expect("run the actual local Forgejo merge adapter");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "unknown Forgejo target profile; expected merge-014-agent-forge-core or mission-012-clamp\n"
    );
}

#[test]
fn capability_discovery_rejects_extra_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .arg("--capabilities")
        .arg("unexpected")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual local Forgejo adapter");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--capabilities accepts no additional arguments")
    );
}

#[test]
#[cfg(unix)]
#[ignore = "binds the adapter's fixed loopback port 3130; run explicitly and serially"]
fn actual_cli_reads_multiple_status_pages_and_refuses_changed_total() {
    let sha = "a".repeat(40);
    let server = spawn_commit_status_http_fixture(sha.clone());

    let complete = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args(["--commit-status-snapshot", &sha])
        .stdin(Stdio::null())
        .output()
        .expect("run the actual status snapshot CLI process");
    assert!(
        complete.status.success(),
        "the complete two-page snapshot should succeed: {}",
        String::from_utf8_lossy(&complete.stderr)
    );
    let snapshot: Value =
        serde_json::from_slice(&complete.stdout).expect("the CLI emits a structured snapshot");
    assert_eq!(snapshot["observation"], "complete");
    assert_eq!(snapshot["status_count"], 51);
    assert_eq!(snapshot["total_count"], 51);
    assert_eq!(snapshot["pages_read"], 2);
    assert_eq!(snapshot["pagination_complete"], true);
    assert_eq!(snapshot["snapshot_consistency"], "sequential_non_atomic");
    assert!(snapshot["forgejo_reported_combined_state"].is_null());
    assert_eq!(
        snapshot["forgejo_reported_page_states"],
        json!(["success", "failure"])
    );
    assert_eq!(snapshot["independent_verification"], false);
    assert_eq!(snapshot["merge_authorizing"], false);

    let inconsistent = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args(["--commit-status-snapshot", &sha])
        .stdin(Stdio::null())
        .output()
        .expect("run the actual status snapshot CLI against changed totals");
    assert!(!inconsistent.status.success());
    assert!(inconsistent.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&inconsistent.stderr)
            .contains("Forgejo status counts are inconsistent; reread the complete snapshot")
    );

    let requests = server.join().expect("fixture served all process requests");
    assert_eq!(requests.len(), 4);
    assert!(requests[0].contains("page=1 HTTP/1.1"));
    assert!(requests[1].contains("page=2 HTTP/1.1"));
    assert!(requests[2].contains("page=1 HTTP/1.1"));
    assert!(requests[3].contains("page=2 HTTP/1.1"));
}

#[test]
#[cfg(unix)]
#[ignore = "binds the adapter's fixed loopback port 3130; run explicitly and serially"]
fn actual_cli_binds_status_snapshot_to_pr_head_and_refuses_head_drift() {
    let base_sha = "b".repeat(40);
    let initial_head_sha = "a".repeat(40);

    let stable_server = spawn_pull_request_status_http_fixture(
        initial_head_sha.clone(),
        initial_head_sha.clone(),
        true,
    );
    let mut stable_child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args(["--pull-request-status-snapshot", "23"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the actual PR-bound status snapshot CLI");
    let stable_status = wait_bounded(&mut stable_child, Duration::from_secs(5));
    assert!(
        stable_status.is_some(),
        "stable PR snapshot CLI must be bounded"
    );
    let stable = stable_child
        .wait_with_output()
        .expect("collect the bounded stable PR snapshot output");
    assert!(
        stable.status.success(),
        "stable PR head snapshot should be complete: {}",
        String::from_utf8_lossy(&stable.stderr)
    );
    let stable_snapshot: Value =
        serde_json::from_slice(&stable.stdout).expect("stable PR snapshot is structured JSON");
    assert_eq!(
        stable_snapshot["schema_version"],
        "forgejo-pull-request-status-snapshot-v1"
    );
    assert_eq!(stable_snapshot["observation"], "complete");
    assert_eq!(stable_snapshot["current_pr_head_checked"], true);
    assert_eq!(
        stable_snapshot["pull_request_head_stable_between_reads"],
        true
    );
    assert_eq!(
        stable_snapshot["pull_request_binding_stable_between_reads"],
        true
    );
    assert_eq!(
        stable_snapshot["pull_request_binding_changed_fields"],
        json!([])
    );
    assert_eq!(
        stable_snapshot["pull_request_head_before"]["head_sha"],
        initial_head_sha
    );
    assert_eq!(
        stable_snapshot["pull_request_head_after"]["head_sha"],
        initial_head_sha
    );
    assert_eq!(
        stable_snapshot["pull_request_head_before"]["base_branch"],
        "main"
    );
    assert_eq!(
        stable_snapshot["pull_request_head_before"]["base_sha"],
        base_sha
    );
    assert_eq!(stable_snapshot["pull_request_head_before"]["state"], "open");
    assert_eq!(stable_snapshot["pull_request_head_before"]["merged"], false);
    assert_eq!(
        stable_snapshot["status_snapshot"]["commit_sha"],
        initial_head_sha
    );
    assert_eq!(stable_snapshot["independent_verification"], false);
    assert_eq!(stable_snapshot["merge_authorizing"], false);
    assert_eq!(
        stable_server.join().expect("serve stable PR status reads"),
        [
            "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1".to_owned(),
            format!(
                "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{initial_head_sha}/status?limit=50&page=1 HTTP/1.1"
            ),
            "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1".to_owned(),
        ]
    );

    let changed_head_sha = "c".repeat(40);
    let drift_server = spawn_pull_request_status_http_fixture(
        initial_head_sha.clone(),
        changed_head_sha.clone(),
        true,
    );
    let mut drift_child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args(["--pull-request-status-snapshot", "23"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the actual PR status snapshot against head drift");
    let drift_status = wait_bounded(&mut drift_child, Duration::from_secs(5));
    assert!(
        drift_status.is_some(),
        "drifting PR snapshot CLI must be bounded"
    );
    let drift = drift_child
        .wait_with_output()
        .expect("collect the bounded drifting PR snapshot output");
    assert_eq!(drift.status.code(), Some(1));
    assert!(drift.stderr.is_empty());
    let drift_snapshot: Value =
        serde_json::from_slice(&drift.stdout).expect("drift refusal is structured JSON");
    assert_eq!(drift_snapshot["observation"], "inconclusive");
    assert_eq!(
        drift_snapshot["reason"],
        "pull_request_changed_during_snapshot"
    );
    assert_eq!(drift_snapshot["current_pr_head_checked"], false);
    assert_eq!(
        drift_snapshot["pull_request_binding_stable_between_reads"],
        false
    );
    assert_eq!(
        drift_snapshot["pull_request_binding_changed_fields"],
        json!(["head_sha"])
    );
    assert_eq!(
        drift_snapshot["pull_request_head_stable_between_reads"],
        false
    );
    assert_eq!(
        drift_snapshot["pull_request_head_before"]["head_sha"],
        initial_head_sha
    );
    assert_eq!(
        drift_snapshot["pull_request_head_after"]["head_sha"],
        changed_head_sha
    );
    assert_eq!(
        drift_snapshot["pull_request_head_before"]["base_sha"],
        base_sha
    );
    assert_eq!(
        drift_snapshot["status_snapshot"]["commit_sha"],
        initial_head_sha
    );
    assert_eq!(drift_snapshot["independent_verification"], false);
    assert_eq!(drift_snapshot["merge_authorizing"], false);
    assert_eq!(
        drift_server.join().expect("serve changed PR status reads"),
        [
            "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1".to_owned(),
            format!(
                "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{initial_head_sha}/status?limit=50&page=1 HTTP/1.1"
            ),
            "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1".to_owned(),
        ]
    );

    let no_status_server = spawn_pull_request_status_http_fixture(
        initial_head_sha.clone(),
        initial_head_sha.clone(),
        false,
    );
    let mut no_status_child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args(["--pull-request-status-snapshot", "23"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the actual PR status snapshot with no reported statuses");
    let no_status_exit = wait_bounded(&mut no_status_child, Duration::from_secs(5));
    assert!(
        no_status_exit.is_some(),
        "no-status PR snapshot CLI must be bounded"
    );
    let no_status = no_status_child
        .wait_with_output()
        .expect("collect the bounded no-status PR snapshot output");
    assert!(no_status.status.success());
    let no_status_snapshot: Value = serde_json::from_slice(&no_status.stdout)
        .expect("no-status response remains structured JSON");
    assert_eq!(no_status_snapshot["observation"], "no_statuses");
    assert_eq!(no_status_snapshot["reason"], "no_statuses");
    assert_eq!(
        no_status_snapshot["action"],
        "no Forgejo status evidence exists for this PR head; run an independent verifier and do not report verification success"
    );
    assert_eq!(no_status_snapshot["current_pr_head_checked"], true);
    assert_eq!(
        no_status_snapshot["pull_request_binding_stable_between_reads"],
        true
    );
    assert_eq!(
        no_status_snapshot["pull_request_binding_changed_fields"],
        json!([])
    );
    assert_eq!(no_status_snapshot["status_snapshot"]["status_count"], 0);
    assert_eq!(no_status_snapshot["independent_verification"], false);
    assert_eq!(no_status_snapshot["merge_authorizing"], false);
    assert_eq!(
        no_status_server
            .join()
            .expect("serve no-status PR reads")
            .len(),
        3
    );
}

#[test]
#[cfg(unix)]
fn preflight_rejects_fifo_input_without_blocking() {
    let scratch = ScratchDir::new();
    let fifo = scratch.path().join("expected.json");
    let fifo_c = std::ffi::CString::new(fifo.as_os_str().as_bytes())
        .expect("temporary FIFO path contains no NUL byte");
    assert_eq!(
        unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) },
        0,
        "create the hostile named-pipe input"
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args(["--preflight", "1", "-"])
        .arg(&fifo)
        .arg(scratch.path().join("missing-key"))
        .arg("fixture-agent")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the actual preflight CLI against a FIFO");

    let status = wait_bounded(&mut child, Duration::from_secs(2));
    assert!(
        status.is_some(),
        "CLI blocked opening a FIFO instead of refusing it"
    );
    let output = child
        .wait_with_output()
        .expect("collect the bounded CLI refusal");
    assert!(!output.status.success(), "a FIFO must never be accepted");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("regular file"),
        "the CLI should explain that the input must be a regular file"
    );
}
