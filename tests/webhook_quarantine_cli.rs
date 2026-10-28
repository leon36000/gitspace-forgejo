use agent_forge_core::{EventJournal, sha256};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::fd::FromRawFd,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agent-forge-webhook-quarantine-cli-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create scratch directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("restrict scratch directory");
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_source(
    path: &std::path::Path,
    delivery_id: &str,
    event_name: &str,
    body: Value,
) -> String {
    let fields = json!({
        "sequence": 1,
        "event_id": "evt-1",
        "idempotency_key": format!("forgejo-delivery-v1:{delivery_id}"),
        "kind": format!("forgejo.webhook.v1.{event_name}"),
        "payload": {
            "schema_version": 1,
            "provider": "forgejo",
            "delivery_id": delivery_id,
            "event": event_name,
            "payload": body,
        },
        "previous_hash": "0".repeat(64),
    });
    let hash = sha256(&serde_json::to_vec(&fields).expect("serialize event hash material"));
    let mut row = fields;
    row["hash"] = Value::String(hash.clone());
    let mut bytes = serde_json::to_vec(&row).expect("serialize source event");
    bytes.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create source journal");
    file.write_all(&bytes).expect("write source event");
    file.sync_all().expect("sync source event");
    hash
}

fn append_quarantine_disposition(
    path: &std::path::Path,
    delivery_id: &str,
    source_hash: &str,
    malformed_reason_field: bool,
    rebound_source_hash: bool,
) -> String {
    let disposition_hash = if rebound_source_hash {
        let replacement = if source_hash.starts_with('0') {
            '1'
        } else {
            '0'
        };
        format!("{replacement}{}", &source_hash[1..])
    } else {
        source_hash.to_owned()
    };
    let reason_field = if malformed_reason_field {
        Value::Number(serde_json::Number::from(7))
    } else {
        Value::String("action".to_owned())
    };
    let fields = json!({
        "sequence": 2,
        "event_id": "evt-2",
        "idempotency_key": format!("forgejo-webhook-quarantine-v1:{delivery_id}:{disposition_hash}"),
        "kind": "forgejo.webhook.quarantine.v1",
        "payload": {
            "schema_version": 1,
            "delivery_id": delivery_id,
            "source_sequence": 1,
            "source_event_id": "evt-1",
            "source_event_hash": disposition_hash,
            "reason_code": "invalid_event_payload_field",
            "reason_field": reason_field,
        },
        "previous_hash": source_hash,
    });
    let hash = sha256(&serde_json::to_vec(&fields).expect("serialize disposition hash material"));
    let mut row = fields;
    row["hash"] = Value::String(hash.clone());
    let mut bytes = serde_json::to_vec(&row).expect("serialize disposition row");
    bytes.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open journal to append test disposition");
    file.write_all(&bytes).expect("append disposition row");
    file.sync_all().expect("sync disposition row");
    hash
}

fn append_active_issue_delivery(
    path: &std::path::Path,
    delivery_id: &str,
    previous_hash: &str,
    sequence: u64,
    issue_number: u64,
) -> String {
    let fields = json!({
        "sequence": sequence,
        "event_id": format!("evt-{sequence}"),
        "idempotency_key": format!("forgejo-delivery-v1:{delivery_id}"),
        "kind": "forgejo.webhook.v1.issues",
        "payload": {
            "schema_version": 1,
            "provider": "forgejo",
            "delivery_id": delivery_id,
            "event": "issues",
            "payload": {
                "action":"opened",
                "repository":{"full_name":"forge-admin/agent-forge-core"},
                "issue":{"number":issue_number}
            }
        },
        "previous_hash": previous_hash,
    });
    let hash = sha256(&serde_json::to_vec(&fields).expect("serialize active event hash material"));
    let mut row = fields;
    row["hash"] = Value::String(hash.clone());
    let mut bytes = serde_json::to_vec(&row).expect("serialize active event row");
    bytes.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open journal to append active event");
    file.write_all(&bytes).expect("append active event");
    file.sync_all().expect("sync active event");
    hash
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .args(args)
        .env_clear()
        .output()
        .expect("run real reconciliation CLI process")
}

fn run_cli_without_network(writable_dir: &std::path::Path, args: &[&str]) -> Output {
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    let sandbox_executable = writable_dir.join("forgejo-reconcile-local");
    if !sandbox_executable.exists() {
        fs::copy(executable, &sandbox_executable)
            .expect("copy CLI into the writable sandbox directory");
        let mut permissions = fs::metadata(&sandbox_executable)
            .expect("read sandbox CLI permissions")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&sandbox_executable, permissions).expect("make sandbox CLI read-only");
    }

    let mut command = Command::new("bwrap");
    command
        .args([
            "--die-with-parent",
            "--unshare-net",
            "--ro-bind",
            "/",
            "/",
            "--bind",
        ])
        .arg(writable_dir)
        .arg(writable_dir)
        .args(["--proc", "/proc", "--dev", "/dev", "--"])
        .arg(&sandbox_executable)
        .args(args)
        .env_clear();
    command
        .output()
        .expect("run real reconciliation CLI in a network namespace")
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "CLI stdout was not JSON: status={:?} stdout={:?} stderr={:?}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn real_cli_quarantines_only_the_exact_invalid_row_and_never_reports_success() {
    let scratch = ScratchDir::new();
    let inbox = scratch.0.join("inbox.jsonl");
    let delivery_id = "cli-quarantine-review-17";
    let source_body = json!({
        "repository":{"full_name":"forge-admin/agent-forge-core"},
        "pull_request":{"number":17},
        "review":{"type":"pull_request_review_approved"}
    });
    let source_hash = write_source(&inbox, delivery_id, "pull_request_approved", source_body);
    let original_bytes = fs::read(&inbox).expect("read original source row");
    let inbox_text = inbox.to_str().expect("scratch path is UTF-8");

    let stale = run_cli(&[
        "--quarantine-invalid-delivery",
        inbox_text,
        delivery_id,
        "1",
        &"f".repeat(64),
    ]);
    assert_eq!(stale.status.code(), Some(1));
    assert_eq!(
        EventJournal::open_webhook_inbox(&inbox)
            .replay()
            .unwrap()
            .len(),
        1
    );

    let first = run_cli(&[
        "--quarantine-invalid-delivery",
        inbox_text,
        delivery_id,
        "1",
        &source_hash,
    ]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_json = stdout_json(&first);
    assert_eq!(first_json["verdict"], "delivery_quarantined");
    assert_eq!(first_json["canonical_state_created"], false);
    assert_eq!(first_json["source_hmac_persisted"], false);
    assert_eq!(first_json["source_hmac_reverified"], false);
    assert_eq!(first_json["actor_identity_recorded"], false);
    assert_eq!(first_json["upstream_read_attempts"], 0);
    assert_eq!(first_json["reason_code"], "invalid_event_payload_field");
    assert_eq!(first_json["reason_field"], "action");
    assert_eq!(first_json["source_event_hash"], source_hash);
    assert_eq!(first_json["disposition_sequence"], 2);
    assert_eq!(first_json["duplicate"], false);

    let duplicate = run_cli(&[
        "--quarantine-invalid-delivery",
        inbox_text,
        delivery_id,
        "1",
        &source_hash,
    ]);
    assert!(duplicate.status.success());
    assert_eq!(stdout_json(&duplicate)["duplicate"], true);
    assert_eq!(
        EventJournal::open_webhook_inbox(&inbox)
            .replay()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        fs::read(&inbox)
            .expect("read journal after quarantine")
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap(),
        original_bytes.split(|byte| *byte == b'\n').next().unwrap(),
        "the operator operation appends and does not rewrite the source"
    );

    let reconcile = run_cli(&[inbox_text, delivery_id]);
    assert_eq!(reconcile.status.code(), Some(1));
    let reconcile_json = stdout_json(&reconcile);
    assert_eq!(reconcile_json["verdict"], "inconclusive");
    assert_eq!(reconcile_json["error"]["code"], "delivery_quarantined");
    assert_eq!(reconcile_json["error"]["retryable"], false);
    assert_eq!(
        reconcile_json["error"]["details"]["canonical_state_created"],
        false
    );
    assert_eq!(reconcile_json["upstream_read_attempts"], 0);

    let valid_inbox = scratch.0.join("valid-inbox.jsonl");
    let valid_id = "cli-quarantine-valid-issue-17";
    let valid_hash = write_source(
        &valid_inbox,
        valid_id,
        "issues",
        json!({
            "action":"opened",
            "repository":{"full_name":"forge-admin/agent-forge-core"},
            "issue":{"number":17}
        }),
    );
    let valid_quarantine = run_cli(&[
        "--quarantine-invalid-delivery",
        valid_inbox.to_str().unwrap(),
        valid_id,
        "1",
        &valid_hash,
    ]);
    assert_eq!(valid_quarantine.status.code(), Some(1));
    assert_eq!(
        EventJournal::open_webhook_inbox(&valid_inbox)
            .replay()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn real_cli_quarantines_persisted_unsupported_tag_with_field_reason() {
    let scratch = ScratchDir::new();
    let inbox = scratch.0.join("inbox.jsonl");
    let delivery_id = "cli-quarantine-tag-ref-17";
    let source_body = json!({
        "ref": "v1.0.0",
        "ref_type": "tag",
        "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "repository": {"full_name":"forge-admin/agent-forge-core"}
    });
    let source_hash = write_source(&inbox, delivery_id, "create", source_body);
    let inbox_text = inbox.to_str().expect("scratch path is UTF-8");

    let output = run_cli(&[
        "--quarantine-invalid-delivery",
        inbox_text,
        delivery_id,
        "1",
        &source_hash,
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = stdout_json(&output);
    assert_eq!(result["verdict"], "delivery_quarantined");
    assert_eq!(result["reason_code"], "unsupported_event_payload_field");
    assert_eq!(result["reason_field"], "ref_type");
    assert_eq!(result["canonical_state_created"], false);
    assert_eq!(result["upstream_read_attempts"], 0);
    assert_eq!(
        EventJournal::open_webhook_inbox(&inbox)
            .replay()
            .expect("replay tag quarantine journal")
            .len(),
        2
    );
}

#[test]
fn real_cli_quarantines_persisted_unsupported_tag_delete_with_field_reason() {
    let scratch = ScratchDir::new();
    let inbox = scratch.0.join("inbox.jsonl");
    let delivery_id = "cli-quarantine-tag-delete-ref-17";
    let source_body = json!({
        "ref": "refs/tags/v1.0.0",
        "ref_type": "tag",
        "repository": {"full_name":"forge-admin/agent-forge-core"}
    });
    let source_hash = write_source(&inbox, delivery_id, "delete", source_body);
    let inbox_text = inbox.to_str().expect("scratch path is UTF-8");

    let output = run_cli(&[
        "--quarantine-invalid-delivery",
        inbox_text,
        delivery_id,
        "1",
        &source_hash,
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = stdout_json(&output);
    assert_eq!(result["verdict"], "delivery_quarantined");
    assert_eq!(result["reason_code"], "unsupported_event_payload_field");
    assert_eq!(result["reason_field"], "ref_type");
    assert_eq!(result["canonical_state_created"], false);
    assert_eq!(result["upstream_read_attempts"], 0);
    assert_eq!(
        EventJournal::open_webhook_inbox(&inbox)
            .replay()
            .expect("replay tag-delete quarantine journal")
            .len(),
        2
    );
}

#[test]
fn real_cli_fails_closed_on_malformed_or_rebound_quarantine_dispositions() {
    let scratch = ScratchDir::new();
    for (suffix, malformed_reason_field, rebound_source_hash, use_batch) in [
        ("malformed", true, false, false),
        ("rebound", false, true, true),
    ] {
        let inbox = scratch.0.join(format!("{suffix}-inbox.jsonl"));
        let delivery_id = format!("cli-quarantine-{suffix}-17");
        let source_hash = write_source(
            &inbox,
            &delivery_id,
            "pull_request_approved",
            json!({
                "repository":{"full_name":"forge-admin/agent-forge-core"},
                "pull_request":{"number":17},
                "review":{"type":"pull_request_review_approved"}
            }),
        );
        let disposition_hash = append_quarantine_disposition(
            &inbox,
            &delivery_id,
            &source_hash,
            malformed_reason_field,
            rebound_source_hash,
        );
        let active_delivery_id = format!("cli-active-{suffix}-41");
        let active_hash =
            append_active_issue_delivery(&inbox, &active_delivery_id, &disposition_hash, 3, 41);
        let second_active_delivery_id = format!("cli-active-{suffix}-42");
        append_active_issue_delivery(&inbox, &second_active_delivery_id, &active_hash, 4, 42);

        let events = EventJournal::open_webhook_inbox(&inbox)
            .replay()
            .expect("the outer hash chain remains valid");
        assert_eq!(events.len(), 4);
        let before = fs::read(&inbox).expect("read journal before CLI replay");
        let mut pipe_fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
        let credential_probe = b"quarantine-refusal-keeps-credential";
        let mut pipe_writer = unsafe { fs::File::from_raw_fd(pipe_fds[1]) };
        pipe_writer
            .write_all(credential_probe)
            .expect("write synthetic inherited credential");
        drop(pipe_writer);
        let descriptor = pipe_fds[0].to_string();
        let mut arguments = vec![
            "--host-credential-fd".to_owned(),
            descriptor,
            inbox.to_string_lossy().into_owned(),
            active_delivery_id.clone(),
        ];
        if use_batch {
            arguments.push(second_active_delivery_id.clone());
        }
        let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        let output = run_cli(&arguments);

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "",
            "structured refusal belongs on stdout"
        );
        let result = stdout_json(&output);
        let rendered = result.to_string();
        assert!(!rendered.contains("reason_field"));
        assert!(!rendered.contains(&source_hash));
        assert!(!rendered.contains(&disposition_hash));
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(
            result["schema_version"],
            if use_batch { 23 } else { 19 },
            "the current single/batch response schema is retained"
        );
        assert_eq!(result["api_version"], "forgejo-rest-v1");
        if use_batch {
            assert_eq!(result["requested_delivery_count"], 2);
            let selected_sources = result["selected_event_sources"]
                .as_array()
                .expect("batch quarantine refusal retains source identities");
            assert_eq!(selected_sources.len(), 2);
            for ((source, delivery_id), event) in selected_sources
                .iter()
                .zip([
                    active_delivery_id.as_str(),
                    second_active_delivery_id.as_str(),
                ])
                .zip(&events[2..])
            {
                assert_eq!(source["delivery_id"].as_str(), Some(delivery_id));
                assert_eq!(source["journal_source"]["sequence"], event.sequence);
                assert_eq!(source["journal_source"]["event_sha256"], event.hash);
                assert_eq!(
                    source["journal_source"]["receiver_attestation"]["status"],
                    "missing"
                );
            }
        } else {
            assert_eq!(result["delivery_id"], active_delivery_id);
        }
        assert_eq!(result["upstream_read_attempts"], 0);
        assert_eq!(result["error"]["code"], "invalid_quarantine_disposition");
        assert_eq!(result["error"]["retryable"], false);
        assert_eq!(result["error"]["details"]["canonical_state_created"], false);
        assert_eq!(result["error"]["details"]["host_credential_read"], false);
        assert_eq!(
            result["error"]["details"]["upstream_calls_attempted"],
            false
        );
        assert_eq!(
            result["error"]["details"]["validation_scope"],
            "all_quarantine_dispositions_in_journal"
        );
        let action = result["error"]["action"].as_str().unwrap();
        assert!(action.contains("preserve"), "{action}");
        assert!(action.contains("trusted offline"), "{action}");
        assert_eq!(
            fs::read(&inbox).expect("read journal after refused replay"),
            before,
            "refusal must not rewrite a hash-chain-valid journal"
        );
        let mut pipe_reader = unsafe { fs::File::from_raw_fd(pipe_fds[0]) };
        let mut remaining = Vec::new();
        pipe_reader
            .read_to_end(&mut remaining)
            .expect("read unused credential probe");
        assert_eq!(
            remaining, credential_probe,
            "refusal must precede credential reads"
        );
    }
}

#[test]
fn real_cli_preserves_an_incomplete_tail_before_quarantine_validation() {
    let scratch = ScratchDir::new();
    let inbox = scratch.0.join("torn-quarantine-inbox.jsonl");
    let quarantined_delivery_id = "cli-torn-quarantine-17";
    let source_hash = write_source(
        &inbox,
        quarantined_delivery_id,
        "pull_request_approved",
        json!({
            "repository":{"full_name":"forge-admin/agent-forge-core"},
            "pull_request":{"number":17},
            "review":{"type":"pull_request_review_approved"}
        }),
    );
    let disposition_hash =
        append_quarantine_disposition(&inbox, quarantined_delivery_id, &source_hash, true, false);
    let active_delivery_id = "cli-torn-active-41";
    append_active_issue_delivery(&inbox, active_delivery_id, &disposition_hash, 3, 41);
    let complete_events = EventJournal::open_webhook_inbox(&inbox)
        .replay()
        .expect("the complete prefix has a valid hash chain");
    assert_eq!(complete_events.len(), 3);

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&inbox)
        .expect("open inbox to model an interrupted append");
    file.write_all(b"{\"sequence\":4,\"payload\":")
        .expect("append an incomplete record prefix");
    file.sync_all().expect("persist the incomplete tail");
    drop(file);
    let before = fs::read(&inbox).expect("read inbox before CLI replay");

    let mut pipe_fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let credential_probe = b"torn-inbox-refusal-keeps-credential";
    let mut pipe_writer = unsafe { fs::File::from_raw_fd(pipe_fds[1]) };
    pipe_writer
        .write_all(credential_probe)
        .expect("write synthetic inherited credential");
    drop(pipe_writer);
    let descriptor = pipe_fds[0].to_string();
    let output = run_cli(&[
        "--host-credential-fd",
        &descriptor,
        inbox.to_str().unwrap(),
        active_delivery_id,
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "an incomplete inbox must produce a structured refusal"
    );
    assert_eq!(
        fs::read(&inbox).expect("read inbox after refused replay"),
        before,
        "the CLI must not repair or otherwise mutate an incomplete inbox"
    );
    let result = stdout_json(&output);
    let rendered = result.to_string();
    assert!(!rendered.contains("reason_field"));
    assert!(!rendered.contains(&source_hash));
    assert!(!rendered.contains(&disposition_hash));
    assert_eq!(result["schema_version"], 19);
    assert_eq!(result["api_version"], "forgejo-rest-v1");
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["error"]["code"], "inbox_journal_invalid");
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(result["error"]["details"]["canonical_state_created"], false);
    assert_eq!(result["error"]["details"]["host_credential_read"], false);
    assert_eq!(
        result["error"]["details"]["upstream_calls_attempted"],
        false
    );
    assert_eq!(
        result["error"]["details"]["journal_repair_attempted"],
        false
    );
    let action = result["error"]["action"].as_str().unwrap();
    assert!(action.contains("preserve"), "{action}");
    let mut pipe_reader = unsafe { fs::File::from_raw_fd(pipe_fds[0]) };
    let mut remaining = Vec::new();
    pipe_reader
        .read_to_end(&mut remaining)
        .expect("read unused credential probe");
    assert_eq!(remaining, credential_probe);
}

#[test]
fn real_cli_preserves_an_incomplete_tail_for_an_unresolved_batch_selection() {
    let scratch = ScratchDir::new();
    let inbox = scratch.0.join("torn-batch-inbox.jsonl");
    let complete_delivery_id = "cli-torn-batch-41";
    write_source(
        &inbox,
        complete_delivery_id,
        "issues",
        json!({
            "action":"opened",
            "repository":{"full_name":"forge-admin/agent-forge-core"},
            "issue":{"number":41}
        }),
    );
    let complete_events = EventJournal::open_webhook_inbox(&inbox)
        .replay()
        .expect("the complete batch prefix has a valid hash chain");
    assert_eq!(complete_events.len(), 1);

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&inbox)
        .expect("open inbox to model an interrupted append");
    file.write_all(b"{\"sequence\":2,\"payload\":")
        .expect("append an incomplete record prefix");
    file.sync_all().expect("persist the incomplete tail");
    drop(file);
    let before = fs::read(&inbox).expect("read inbox before batch replay");

    let mut pipe_fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let credential_probe = b"torn-batch-refusal-keeps-credential";
    let mut pipe_writer = unsafe { fs::File::from_raw_fd(pipe_fds[1]) };
    pipe_writer
        .write_all(credential_probe)
        .expect("write synthetic inherited credential");
    drop(pipe_writer);
    let descriptor = pipe_fds[0].to_string();
    let output = run_cli(&[
        "--host-credential-fd",
        &descriptor,
        inbox.to_str().unwrap(),
        complete_delivery_id,
        "missing-batch-delivery",
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stderr.is_empty(),
        "CLI stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&inbox).expect("read inbox after refused batch replay"),
        before,
        "an incomplete inbox must be refused before batch selection can repair it"
    );
    let result = stdout_json(&output);
    assert_eq!(result["schema_version"], 23);
    assert_eq!(result["api_version"], "forgejo-rest-v1");
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["requested_delivery_count"], 2);
    assert_eq!(result["upstream_read_attempts"], 0);
    assert_eq!(result["error"]["code"], "inbox_journal_invalid");
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(result["error"]["details"]["canonical_state_created"], false);
    assert_eq!(result["error"]["details"]["host_credential_read"], false);
    assert_eq!(
        result["error"]["details"]["upstream_calls_attempted"],
        false
    );
    assert_eq!(
        result["error"]["details"]["journal_repair_attempted"],
        false
    );
    let mut pipe_reader = unsafe { fs::File::from_raw_fd(pipe_fds[0]) };
    let mut remaining = Vec::new();
    pipe_reader
        .read_to_end(&mut remaining)
        .expect("read unused credential probe");
    assert_eq!(remaining, credential_probe);
}

#[test]
fn real_cli_refuses_a_torn_batch_when_every_selected_delivery_exists() {
    let scratch = ScratchDir::new();
    let inbox = scratch.0.join("torn-complete-batch-inbox.jsonl");
    let first_delivery_id = "cli-torn-complete-batch-41";
    let first_hash = write_source(
        &inbox,
        first_delivery_id,
        "issues",
        json!({
            "action":"opened",
            "repository":{"full_name":"forge-admin/agent-forge-core"},
            "issue":{"number":41}
        }),
    );
    let second_delivery_id = "cli-torn-complete-batch-42";
    append_active_issue_delivery(&inbox, second_delivery_id, &first_hash, 2, 42);
    let complete_events = EventJournal::open_webhook_inbox(&inbox)
        .replay()
        .expect("the complete batch prefix has a valid hash chain");
    assert_eq!(complete_events.len(), 2);

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&inbox)
        .expect("open inbox to model an interrupted append");
    file.write_all(b"{\"sequence\":3,\"payload\":")
        .expect("append an incomplete record prefix");
    file.sync_all().expect("persist the incomplete tail");
    drop(file);
    let before = fs::read(&inbox).expect("read inbox before batch replay");

    let mut pipe_fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let credential_probe = b"torn-complete-batch-refusal-keeps-credential";
    let mut pipe_writer = unsafe { fs::File::from_raw_fd(pipe_fds[1]) };
    pipe_writer
        .write_all(credential_probe)
        .expect("write synthetic inherited credential");
    drop(pipe_writer);
    let descriptor = pipe_fds[0].to_string();
    let output = run_cli_without_network(
        &scratch.0,
        &[
            "--host-credential-fd",
            &descriptor,
            inbox.to_str().unwrap(),
            first_delivery_id,
            second_delivery_id,
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stderr.is_empty(),
        "CLI stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&inbox).expect("read inbox after refused batch replay"),
        before,
        "all selected IDs being present must not let replay repair a torn inbox"
    );
    let result = stdout_json(&output);
    assert_eq!(result["schema_version"], 23);
    assert_eq!(result["verdict"], "inconclusive");
    assert_eq!(result["requested_delivery_count"], 2);
    assert_eq!(result["upstream_read_attempts"], 0);
    assert_eq!(result["error"]["code"], "inbox_journal_invalid");
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(result["error"]["details"]["host_credential_read"], false);
    assert_eq!(
        result["error"]["details"]["upstream_calls_attempted"],
        false
    );
    assert_eq!(
        result["error"]["details"]["journal_repair_attempted"],
        false
    );
    let mut pipe_reader = unsafe { fs::File::from_raw_fd(pipe_fds[0]) };
    let mut remaining = Vec::new();
    pipe_reader
        .read_to_end(&mut remaining)
        .expect("read unused credential probe");
    assert_eq!(remaining, credential_probe);
}
