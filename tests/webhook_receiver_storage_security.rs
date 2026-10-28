#![cfg(unix)]

use std::{
    fs::{self, OpenOptions},
    io::Write,
    net::TcpListener,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "webhook-storage-security-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create test directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("restrict test directory");
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_secret(path: &std::path::Path) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create synthetic secret");
    file.write_all(b"synthetic-only-secret")
        .expect("write synthetic secret");
    file.sync_all().expect("flush synthetic secret");
}

fn write_empty_journal(path: &std::path::Path, mode: u32) {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .expect("create test inbox");
    file.sync_all().expect("flush test inbox");
}

fn invoke_receiver_with_reserved_address(
    secret: &std::path::Path,
    journal: &std::path::Path,
) -> (Output, TcpListener) {
    let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve fixture port");
    let address = reservation.local_addr().expect("read fixture port");
    let output = Command::new(env!("CARGO_BIN_EXE_forgejo-webhook-local"))
        .env_clear()
        .arg(address.to_string())
        .arg(secret)
        .arg(journal)
        .stdin(Stdio::null())
        .output()
        .expect("run actual receiver startup");
    (output, reservation)
}

fn assert_storage_refusal(output: &Output, expected: &str) {
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "expected startup refusal {expected:?}, got {stderr:?}"
    );
    assert!(!stderr.contains("could not bind loopback listener"));
}

#[test]
fn receiver_refuses_inbox_directory_accessible_to_other_users() {
    let scratch = ScratchDir::new();
    let secret = scratch.0.join("secret");
    let journal = scratch.0.join("inbox.jsonl");
    write_secret(&secret);
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o755))
        .expect("make test directory group and world traversable");

    let (output, _reservation) = invoke_receiver_with_reserved_address(&secret, &journal);

    assert_storage_refusal(
        &output,
        "journal directory must be owned by the current user with mode 0700",
    );
}

#[test]
fn receiver_refuses_group_accessible_existing_inbox() {
    let scratch = ScratchDir::new();
    let secret = scratch.0.join("secret");
    let journal = scratch.0.join("inbox.jsonl");
    write_secret(&secret);
    write_empty_journal(&journal, 0o640);

    let (output, _reservation) = invoke_receiver_with_reserved_address(&secret, &journal);

    assert_storage_refusal(
        &output,
        "journal file must be a private owner-owned regular file with one link and mode 0600",
    );
}

#[test]
fn receiver_refuses_hard_linked_existing_inbox() {
    let scratch = ScratchDir::new();
    let secret = scratch.0.join("secret");
    let journal = scratch.0.join("inbox.jsonl");
    write_secret(&secret);
    write_empty_journal(&journal, 0o600);
    fs::hard_link(&journal, scratch.0.join("inbox-link.jsonl")).expect("create second inbox name");

    let (output, _reservation) = invoke_receiver_with_reserved_address(&secret, &journal);

    assert_storage_refusal(
        &output,
        "journal file must be a private owner-owned regular file with one link and mode 0600",
    );
}
