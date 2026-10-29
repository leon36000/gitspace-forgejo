use agent_forge_core::EventJournal;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_INBOX_BYTES: u64 = 64 * 1024 * 1024;

fn scratch_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "forgejo-webhook-snapshot-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("create private snapshot fixture directory");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("make snapshot fixture directory private");
    path
}

fn create_journal(path: &std::path::Path) -> Vec<u8> {
    let journal = EventJournal::open_webhook_inbox(path);
    journal
        .append(
            "snapshot-event-1",
            "snapshot-test",
            serde_json::json!({"value": 1}),
        )
        .expect("append first valid record");
    journal
        .append(
            "snapshot-event-2",
            "snapshot-test",
            serde_json::json!({"value": 2}),
        )
        .expect("append second valid record");
    fs::read(path).expect("read valid fixture journal")
}

fn snapshot_command(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
        .env_clear()
        .arg("--snapshot-inbox")
        .arg(source)
        .arg(destination)
        .stdin(Stdio::null())
        .output()
        .expect("run local inbox snapshot command")
}

fn restore_command(
    snapshot: &std::path::Path,
    destination: &std::path::Path,
    expected_sha256: &str,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
        .env_clear()
        .arg("--stage-restore-inbox")
        .arg(snapshot)
        .arg(destination)
        .arg(expected_sha256)
        .stdin(Stdio::null())
        .output()
        .expect("run local inbox restore staging command")
}

#[cfg(all(feature = "test-crash-injection", target_os = "linux"))]
struct FeatureChildGuard(Option<std::process::Child>);

#[cfg(all(feature = "test-crash-injection", target_os = "linux"))]
impl FeatureChildGuard {
    fn new(child: std::process::Child) -> Self {
        Self(Some(child))
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.0.as_mut().expect("child guard is active").try_wait()
    }

    fn kill(&mut self) -> std::io::Result<()> {
        self.0.as_mut().expect("child guard is active").kill()
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.0.as_mut().expect("child guard is active").wait()
    }

    fn wait_with_output(&mut self) -> std::io::Result<std::process::Output> {
        self.0
            .take()
            .expect("child guard is active")
            .wait_with_output()
    }
}

#[cfg(all(feature = "test-crash-injection", target_os = "linux"))]
impl Drop for FeatureChildGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

#[test]
fn cli_exports_the_validated_bytes_and_receipt_without_mutating_the_inbox() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let destination = root.join("snapshot.jsonl");
    let original = create_journal(&source);
    let expected_last_hash = EventJournal::open_webhook_inbox(&source)
        .replay_read_only()
        .unwrap()
        .last()
        .unwrap()
        .hash
        .clone();

    let output = snapshot_command(&source, &destination);

    assert!(
        output.status.success(),
        "snapshot failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(&source).unwrap(),
        original,
        "source must stay byte-for-byte unchanged"
    );
    assert_eq!(
        fs::read(&destination).unwrap(),
        original,
        "export must preserve exact JSONL bytes"
    );
    let metadata = fs::symlink_metadata(&destination).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

    let receipt: Value = serde_json::from_slice(&output.stdout).expect("snapshot receipt JSON");
    assert_eq!(
        receipt["schema_version"],
        "forgejo-webhook-inbox-snapshot-v1"
    );
    assert_eq!(receipt["byte_length"], original.len() as u64);
    assert_eq!(receipt["event_count"], 2);
    assert_eq!(receipt["last_sequence"], 2);
    assert_eq!(receipt["last_event_hash"], expected_last_hash);
    assert_eq!(receipt["sha256"], hex::encode(Sha256::digest(&original)));
    assert_eq!(receipt["restore_policy"], "reconcile_or_quiesce_and_replay");
    assert_eq!(receipt["quiescent_restore_point"], false);

    let retry = snapshot_command(&source, &destination);
    assert!(
        !retry.status.success(),
        "existing destination must never be replaced"
    );
    assert_eq!(fs::read(&destination).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_an_unterminated_journal_without_repairing_or_creating_a_snapshot() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let destination = root.join("snapshot.jsonl");
    let _ = create_journal(&source);
    let length = fs::metadata(&source).unwrap().len();
    OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_len(length - 1)
        .expect("remove final newline to model interrupted append");
    let torn_source = fs::read(&source).unwrap();

    let output = snapshot_command(&source, &destination);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("snapshot_refused"),
        "failure should identify an actionable snapshot refusal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !destination.exists(),
        "invalid history must not produce an export"
    );
    assert_eq!(
        fs::read(&source).unwrap(),
        torn_source,
        "read-only refusal must preserve the torn tail"
    );
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_a_preexisting_destination_without_overwriting_it() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let destination = root.join("snapshot.jsonl");
    let _ = create_journal(&source);
    let original_destination = b"preserve existing export\n";
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&destination)
        .expect("create preexisting private destination");
    std::io::Write::write_all(&mut file, original_destination).unwrap();
    file.sync_all().unwrap();

    let output = snapshot_command(&source, &destination);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(&destination).unwrap(), original_destination);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_a_symlink_destination_and_preserves_its_target() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let destination = root.join("snapshot.jsonl");
    let target = root.join("existing.jsonl");
    let _ = create_journal(&source);
    fs::write(&target, b"preserve symlink target\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&target, &destination).unwrap();

    let output = snapshot_command(&source, &destination);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&target).unwrap(), b"preserve symlink target\n");
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_an_oversized_sparse_inbox_before_reading_or_exporting_it() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let destination = root.join("snapshot.jsonl");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&source)
        .expect("create sparse inbox fixture");
    file.set_len(MAX_INBOX_BYTES + 1)
        .expect("make inbox exceed the configured bound");
    file.sync_all().unwrap();

    let output = snapshot_command(&source, &destination);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("exceeds the 67108864-byte limit"),
        "oversized history should produce an actionable capacity refusal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::metadata(&source).unwrap().len(), MAX_INBOX_BYTES + 1);
    assert!(!destination.exists());
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_stages_verified_snapshot_and_resumes_the_journal_sequence() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let snapshot = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let original = create_journal(&source);

    let exported = snapshot_command(&source, &snapshot);
    assert!(exported.status.success());
    let export_receipt: Value = serde_json::from_slice(&exported.stdout).unwrap();
    let expected_sha256 = export_receipt["sha256"].as_str().unwrap();
    let snapshot_bytes = fs::read(&snapshot).unwrap();

    let output = restore_command(&snapshot, &restored, expected_sha256);

    assert!(
        output.status.success(),
        "restore staging failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let receipt: Value = serde_json::from_slice(&output.stdout).expect("restore receipt JSON");
    assert_eq!(
        receipt["schema_version"],
        "forgejo-webhook-inbox-restore-v1"
    );
    assert_eq!(receipt["operation"], "stage_validated_snapshot");
    assert_eq!(receipt["snapshot_sha256"], expected_sha256);
    assert_eq!(receipt["staged_sha256"], expected_sha256);
    assert_eq!(
        receipt["expected_sha256_source"],
        "operator_supplied_argument"
    );
    assert_eq!(receipt["event_count"], 2);
    assert_eq!(receipt["last_sequence"], 2);
    assert_eq!(
        receipt["last_event_hash"],
        export_receipt["last_event_hash"]
    );
    assert_eq!(
        receipt["source_validation"],
        "event_chain_and_idempotency_keys"
    );
    assert_eq!(
        receipt["destination_validation"],
        "read_back_event_chain_and_hash"
    );
    assert_eq!(receipt["source_snapshot_bytes_modified"], false);
    assert_eq!(receipt["existing_inbox_replaced"], false);
    assert_eq!(receipt["quiescent_restore_point"], false);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&snapshot).unwrap(), snapshot_bytes);
    assert_eq!(fs::read(&restored).unwrap(), original);

    let metadata = fs::symlink_metadata(&restored).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

    let journal = EventJournal::open_webhook_inbox(&restored);
    let recovered = journal.replay_read_only().expect("validate staged journal");
    assert_eq!(recovered.len(), 2);
    assert_eq!(recovered[1].sequence, 2);
    assert!(matches!(
        journal.append(
            "snapshot-event-2",
            "snapshot-test",
            serde_json::json!({"value": 2})
        ),
        Ok(agent_forge_core::AppendResult::Duplicate(event)) if event.sequence == 2
    ));
    assert_eq!(journal.replay_read_only().unwrap().len(), 2);
    journal
        .append(
            "snapshot-event-3",
            "snapshot-test",
            serde_json::json!({"value": 3}),
        )
        .expect("continue appending to staged journal");
    let resumed = journal.replay_read_only().expect("replay resumed journal");
    assert_eq!(resumed.len(), 3);
    assert_eq!(resumed[2].sequence, 3);
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_restore_digest_mismatch_without_creating_destination() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let snapshot = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let original = create_journal(&source);
    assert!(snapshot_command(&source, &snapshot).status.success());
    let snapshot_bytes = fs::read(&snapshot).unwrap();

    let output = restore_command(&snapshot, &restored, &"0".repeat(64));

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore_refused"));
    assert!(!restored.exists());
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(fs::read(&snapshot).unwrap(), snapshot_bytes);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_malformed_restore_digest_without_reading_or_staging() {
    let root = scratch_dir();
    let source = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let original = create_journal(&source);

    let output = restore_command(&source, &restored, "NOT-A-SHA256");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected snapshot SHA-256"));
    assert!(!restored.exists());
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_corrupt_snapshot_even_when_expected_digest_matches() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let snapshot = root.join("corrupt-snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let _ = create_journal(&source);
    let mut corrupt = fs::read(&source).unwrap();
    corrupt[0] ^= 1;
    fs::write(&snapshot, &corrupt).unwrap();
    fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o600)).unwrap();
    let expected_sha256 = hex::encode(Sha256::digest(&corrupt));

    let output = restore_command(&snapshot, &restored, &expected_sha256);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore_refused"));
    assert!(!restored.exists());
    assert_eq!(fs::read(&snapshot).unwrap(), corrupt);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_hard_linked_restore_source_without_staging_target() {
    let root = scratch_dir();
    let original_source = root.join("inbox.jsonl");
    let snapshot = root.join("snapshot-hardlink.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let original = create_journal(&original_source);
    fs::hard_link(&original_source, &snapshot).expect("create linked snapshot fixture");
    let expected_sha256 = hex::encode(Sha256::digest(&original));

    let output = restore_command(&snapshot, &restored, &expected_sha256);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore_refused"));
    assert!(!restored.exists());
    assert_eq!(fs::read(&original_source).unwrap(), original);
    assert_eq!(fs::read(&snapshot).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_missing_restore_source_before_creating_any_path() {
    let root = scratch_dir();
    let missing = root.join("missing-snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");

    let output = restore_command(&missing, &restored, &"0".repeat(64));

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore_refused"));
    assert!(!missing.exists());
    assert!(!root.join("missing-snapshot.jsonl.lock").exists());
    assert!(!restored.exists());
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_existing_restore_destination_without_overwriting_it() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let snapshot = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let _ = create_journal(&source);
    let export = snapshot_command(&source, &snapshot);
    assert!(export.status.success());
    let receipt: Value = serde_json::from_slice(&export.stdout).expect("snapshot receipt");
    let sentinel = b"preserve existing restore target\n";
    fs::write(&restored, sentinel).unwrap();
    fs::set_permissions(&restored, fs::Permissions::from_mode(0o600)).unwrap();

    let output = restore_command(&snapshot, &restored, receipt["sha256"].as_str().unwrap());

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore_refused"));
    assert_eq!(fs::read(&restored).unwrap(), sentinel);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn cli_refuses_symlink_restore_destination_without_touching_its_target() {
    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let snapshot = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let target = root.join("destination-target.jsonl");
    let original = create_journal(&source);
    let export = snapshot_command(&source, &snapshot);
    assert!(export.status.success());
    let receipt: Value = serde_json::from_slice(&export.stdout).expect("snapshot receipt");
    let sentinel = b"preserve symlink restore target\n";
    fs::write(&target, sentinel).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&target, &restored).unwrap();

    let output = restore_command(&snapshot, &restored, receipt["sha256"].as_str().unwrap());

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore_refused"));
    assert!(
        fs::symlink_metadata(&restored)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&target).unwrap(), sentinel);
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[cfg(all(feature = "test-crash-injection", target_os = "linux"))]
#[test]
fn interrupted_restore_never_exposes_a_partial_destination() {
    use std::time::{Duration, Instant};

    let root = scratch_dir();
    let source = root.join("inbox.jsonl");
    let snapshot = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let barrier = root.join("restore-ready");
    let original = create_journal(&source);
    let exported = snapshot_command(&source, &snapshot);
    assert!(exported.status.success());
    let receipt: Value = serde_json::from_slice(&exported.stdout).unwrap();
    let expected_sha256 = receipt["sha256"].as_str().unwrap();

    let mut child = FeatureChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
            .env_clear()
            .env("AGENT_FORGE_TEST_RESTORE_PAUSE_BEFORE_PUBLISH", &barrier)
            .arg("--stage-restore-inbox")
            .arg(&snapshot)
            .arg(&restored)
            .arg(expected_sha256)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start restore process with the feature-gated crash barrier"),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while !barrier.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !barrier.exists() {
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            let _ = child.wait();
        }
        panic!("restore process exited without reaching the pre-publication barrier");
    }
    assert_eq!(
        fs::read(&barrier).unwrap(),
        b"anonymous staged file is synced; final name is not published\n"
    );

    let status_before_kill = child.try_wait().unwrap();
    if status_before_kill.is_some() || restored.exists() {
        if status_before_kill.is_none() {
            child.kill().unwrap();
            let _ = child.wait();
        }
        panic!("restore completed or exposed its destination before publication was allowed");
    }

    child.kill().unwrap();
    let killed = child.wait().unwrap();
    assert!(
        !killed.success(),
        "the child must be interrupted at the barrier"
    );
    assert!(
        !restored.exists(),
        "an interrupted restore must leave no final path that could be activated"
    );
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private crash-test fixture");
}

#[cfg(all(feature = "test-crash-injection", target_os = "linux"))]
#[test]
fn interrupted_after_restore_publish_leaves_a_complete_private_destination() {
    use std::time::{Duration, Instant};

    let root = scratch_dir();
    let source = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let barrier = root.join("restore-published");
    let original = create_journal(&source);
    let expected_sha256 = hex::encode(Sha256::digest(&original));

    let mut child = FeatureChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
            .env_clear()
            .env("AGENT_FORGE_TEST_RESTORE_PAUSE_AFTER_PUBLISH", &barrier)
            .arg("--stage-restore-inbox")
            .arg(&source)
            .arg(&restored)
            .arg(expected_sha256)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start restore process with the post-publication crash barrier"),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while !barrier.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !barrier.exists() {
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            let _ = child.wait();
        }
        panic!("restore process exited without reaching the post-publication barrier");
    }
    let marker_bytes = fs::read(&barrier).unwrap();
    let published_bytes = fs::read(&restored).ok();
    let published_metadata = fs::symlink_metadata(&restored).ok();
    let status_before_kill = child.try_wait().unwrap();
    if status_before_kill.is_none() {
        child.kill().unwrap();
    }
    let killed = child.wait().unwrap();
    assert!(
        status_before_kill.is_none() && !killed.success(),
        "the process must be interrupted while directory sync is pending"
    );
    assert_eq!(
        marker_bytes,
        b"complete destination linked; directory sync is pending\n"
    );
    assert_eq!(published_bytes.as_deref(), Some(original.as_slice()));
    let metadata = published_metadata.expect("published destination metadata");
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let staged = EventJournal::open_webhook_inbox(&restored)
        .snapshot_read_only()
        .expect("destination remains a complete validated journal after process termination");
    assert_eq!(staged.bytes, original);
    fs::remove_dir_all(root).expect("remove private crash-test fixture");
}

#[cfg(all(feature = "test-crash-injection", target_os = "linux"))]
#[test]
fn restore_refuses_a_destination_created_at_the_publish_boundary() {
    use std::{
        io::Write,
        time::{Duration, Instant},
    };

    let root = scratch_dir();
    let source = root.join("snapshot.jsonl");
    let restored = root.join("restored-inbox.jsonl");
    let barrier = root.join("restore-ready");
    let resume = root.join("restore-resume");
    let original = create_journal(&source);
    let expected_sha256 = hex::encode(Sha256::digest(&original));

    let mut child = FeatureChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
            .env_clear()
            .env("AGENT_FORGE_TEST_RESTORE_PAUSE_BEFORE_PUBLISH", &barrier)
            .env("AGENT_FORGE_TEST_RESTORE_CONTINUE_AFTER_BARRIER", &resume)
            .arg("--stage-restore-inbox")
            .arg(&source)
            .arg(&restored)
            .arg(expected_sha256)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start restore process with the pre-publication barrier"),
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while !barrier.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !barrier.exists() {
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            let _ = child.wait();
        }
        panic!("restore process exited without reaching the pre-publication barrier");
    }
    if restored.exists() {
        child.kill().unwrap();
        let _ = child.wait();
        panic!("destination appeared before the test writer reached the publish boundary");
    }

    let sentinel = b"concurrent destination must remain unchanged\n";
    let existing_result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&restored);
    if existing_result.is_err() {
        child.kill().unwrap();
        let _ = child.wait();
        panic!("concurrent test writer could not create the final destination");
    }
    let mut existing = existing_result.unwrap();
    existing.write_all(sentinel).unwrap();
    existing.sync_all().unwrap();
    let release = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&resume)
        .expect("release the pre-publication barrier");
    release.sync_all().unwrap();

    let exit_deadline = Instant::now() + Duration::from_secs(5);
    let mut exited = false;
    while Instant::now() < exit_deadline {
        if child.try_wait().unwrap().is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !exited {
        child.kill().unwrap();
        let _ = child.wait();
        panic!("restore did not resolve the destination race within the bound");
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("restore_refused: destination already exists")
    );
    assert_eq!(fs::read(&restored).unwrap(), sentinel);
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::remove_dir_all(root).expect("remove private race-test fixture");
}
