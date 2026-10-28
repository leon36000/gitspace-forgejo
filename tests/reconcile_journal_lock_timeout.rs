use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    os::fd::AsRawFd,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn scratch_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "forgejo-reconcile-lock-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("create private test directory");
    path
}

fn journal_lock_path(journal_path: &Path) -> PathBuf {
    let mut path = journal_path.as_os_str().to_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

fn hold_exclusive_lock(path: &Path) -> File {
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .expect("create journal lock fixture");
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(result, 0, "hold the journal lock in this process");
    file
}

#[test]
fn actual_cli_discloses_the_bounded_inbox_lock_wait() {
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .arg("--capabilities")
        .stdin(Stdio::null())
        .output()
        .expect("run the actual reconciliation CLI");

    assert!(
        output.status.success(),
        "capability discovery failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("capability JSON");
    let operation = value["operations"]
        .as_array()
        .expect("operations array")
        .iter()
        .find(|operation| operation["id"] == "reconcile_webhook_deliveries")
        .expect("reconciliation operation");
    assert_eq!(operation["inbox_journal_lock"]["wait"], "bounded");
    assert_eq!(operation["inbox_journal_lock"]["wait_timeout_ms"], 5_000);
}

#[test]
fn actual_cli_returns_journal_busy_before_the_held_lock_is_released() {
    let root = scratch_dir();
    let journal = root.join("inbox.jsonl");
    let held_lock = hold_exclusive_lock(&journal_lock_path(&journal));
    let mut child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal)
        .arg("delivery-not-present")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the actual reconciliation CLI");

    let started = Instant::now();
    let mut elapsed_when_exit = None;
    while started.elapsed() < Duration::from_secs(6) {
        if child.try_wait().expect("poll reconciliation CLI").is_some() {
            elapsed_when_exit = Some(started.elapsed());
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    drop(held_lock);
    let output = child
        .wait_with_output()
        .expect("collect reconciliation CLI output");

    assert!(
        elapsed_when_exit.is_some(),
        "CLI stayed blocked until the lock was released"
    );
    assert!(
        elapsed_when_exit.expect("the CLI exited while the lock was held")
            >= Duration::from_millis(4_500),
        "CLI returned journal busy before the advertised wait window"
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("inbox journal is busy"),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    fs::remove_dir_all(root).expect("remove private test fixture");
}

#[test]
fn actual_cli_retries_when_the_lock_is_released_inside_the_wait_window() {
    let root = scratch_dir();
    let journal = root.join("inbox.jsonl");
    let held_lock = hold_exclusive_lock(&journal_lock_path(&journal));
    let mut child = Command::new(env!("CARGO_BIN_EXE_forgejo-reconcile-local"))
        .env_clear()
        .arg(&journal)
        .arg("delivery-not-present")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the actual reconciliation CLI");

    thread::sleep(Duration::from_millis(250));
    let early_exit = child.try_wait().expect("poll reconciliation CLI");
    drop(held_lock);
    let output = child
        .wait_with_output()
        .expect("collect reconciliation CLI output");

    assert!(
        early_exit.is_none(),
        "CLI returned before the held lock was released: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stderr.is_empty(),
        "structured reconciliation refusal must not be mixed with stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .expect("released lock lets the CLI return its structured selection refusal");
    assert_eq!(value["schema_version"], 19);
    assert_eq!(value["verdict"], "inconclusive");
    assert_eq!(value["delivery_id"], "delivery-not-present");
    assert_eq!(value["upstream_read_attempts"], 0);
    assert_eq!(value["error"]["code"], "delivery_not_found");
    fs::remove_dir_all(root).expect("remove private test fixture");
}
