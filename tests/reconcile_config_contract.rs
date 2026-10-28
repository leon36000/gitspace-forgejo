use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "forgejo-reconcile-config-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated config fixture directory");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_with_config_and_delivery_ids(
    config: &str,
    config_dir: &TempDirectory,
    delivery_ids: &[&str],
    credential_fd: Option<i32>,
) -> Output {
    let config_path = config_dir.path().join("adapter.json");
    let inbox_path = config_dir.path().join("no-inbox.jsonl");
    fs::write(&config_path, config).expect("write config fixture");
    fs::write(&inbox_path, "").expect("write empty inbox fixture");
    fs::set_permissions(&inbox_path, fs::Permissions::from_mode(0o600))
        .expect("restrict empty inbox fixture permissions");
    let mut command = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"));
    command.arg("--config").arg(config_path);
    if let Some(fd) = credential_fd {
        command.arg("--host-credential-fd").arg(fd.to_string());
    }
    command.arg(inbox_path).args(delivery_ids);
    command.output().expect("run local reconciler")
}

fn run_with_config(config: &str, config_dir: &TempDirectory) -> Output {
    run_with_config_and_delivery_ids(config, config_dir, &["fixture-delivery"], None)
}

const CANONICAL_REPOSITORY: &str = r#""schema_version":1,"origin":"http://127.0.0.1:3130","owner":"forge-admin","repository":"agent-forge-core""#;

#[test]
fn canonical_config_is_loaded_before_the_inbox() {
    let temp = TempDirectory::new();
    let output = run_with_config(&format!("{{{CANONICAL_REPOSITORY}}}"), &temp);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("missing delivery returns structured JSON");
    assert_eq!(value["schema_version"], 19);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(
        value["source_event_provenance_contract"],
        serde_json::json!({
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
    assert_eq!(value["delivery_id"], "fixture-delivery");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "delivery_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert!(!value["error"]["action"].as_str().unwrap().is_empty());
}

#[test]
fn batch_with_unknown_delivery_returns_structured_error_before_credential_read() {
    let temp = TempDirectory::new();
    let output = run_with_config_and_delivery_ids(
        &format!("{{{CANONICAL_REPOSITORY}}}"),
        &temp,
        &["missing-delivery-a", "missing-delivery-b"],
        Some(i32::MAX),
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("batch lookup failure returns structured JSON");
    assert_eq!(value["schema_version"], 23);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(
        value["source_event_provenance_contract"]["replay_hmac_verification"],
        "not_performed"
    );
    assert_eq!(value["requested_delivery_count"], 2);
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "delivery_not_found");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["delivery_id"],
        "missing-delivery-a"
    );
    assert!(!value["error"]["action"].as_str().unwrap().is_empty());
}

#[test]
fn config_cannot_redirect_the_local_adapter() {
    let temp = TempDirectory::new();
    let origins = [
        "https://127.0.0.1:3130",
        "http://localhost:3130",
        "http://127.0.0.1:3131",
        "http://user@127.0.0.1:3130",
        "http://127.0.0.1:3130/api/v1",
        "http://127.0.0.1:3130?query=1",
        "http://127.0.0.1:3130#fragment",
    ];

    for origin in origins {
        let config = format!(
            r#"{{"schema_version":1,"origin":"{origin}","owner":"forge-admin","repository":"agent-forge-core"}}"#
        );
        let output = run_with_config(&config, &temp);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains("origin must be exactly"),
            "origin {origin:?} was not rejected by config validation: {stderr}"
        );
    }
}

#[test]
fn config_rejects_other_repositories_versions_and_unknown_credentials() {
    let temp = TempDirectory::new();
    let invalid_configs = [
        (
            r#"{"schema_version":2,"origin":"http://127.0.0.1:3130","owner":"forge-admin","repository":"agent-forge-core"}"#,
            "schema_version must be 1",
        ),
        (
            r#"{"schema_version":1,"origin":"http://127.0.0.1:3130","owner":"other-owner","repository":"agent-forge-core"}"#,
            "repository must be forge-admin/agent-forge-core",
        ),
        (
            r#"{"schema_version":1,"origin":"http://127.0.0.1:3130","owner":"forge-admin","repository":"other-repo"}"#,
            "repository must be forge-admin/agent-forge-core",
        ),
        (
            r#"{"schema_version":1,"origin":"http://127.0.0.1:3130","owner":"forge-admin","repository":"agent-forge-core","token":"secret"}"#,
            "unknown field",
        ),
    ];

    for (config, expected_error) in invalid_configs {
        let output = run_with_config(config, &temp);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains(expected_error),
            "invalid config did not fail as expected ({expected_error}): {stderr}"
        );
    }
}
