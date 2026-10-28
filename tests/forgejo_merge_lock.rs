use agent_forge_core::forgejo::{BaseBranchMergeLock, BaseBranchScope, MergeLockError};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn fresh_lock_root() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "agent-forge-merge-lock-{}-{nonce}",
        std::process::id()
    ))
}

fn scope(branch: &str) -> BaseBranchScope {
    BaseBranchScope::new(
        "http://127.0.0.1:3130",
        "forge-admin",
        "agent-forge-core",
        branch,
    )
    .unwrap()
}

fn receive_marker(receiver: &mpsc::Receiver<String>, expected: &str, timeout: Duration) -> String {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let line = receiver.recv_timeout(remaining).unwrap();
        if line == expected {
            return line;
        }
    }
}

#[test]
fn same_base_branch_lock_serializes_across_processes() {
    let root = fresh_lock_root();
    let current = BaseBranchMergeLock::acquire(&root, &scope("main"), Duration::ZERO).unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "lock_child_waits_for_parent_release",
            "--nocapture",
        ])
        .env("AGENT_FORGE_LOCK_CHILD_ROOT", &root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(Ok(line)) = lines.next() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    assert_eq!(
        receive_marker(&receiver, "READY", Duration::from_secs(3)),
        "READY"
    );
    assert!(matches!(
        BaseBranchMergeLock::acquire(&root, &scope("main"), Duration::from_millis(30)),
        Err(MergeLockError::TimedOut)
    ));

    drop(current);
    assert_eq!(
        receive_marker(&receiver, "ACQUIRED", Duration::from_secs(3)),
        "ACQUIRED"
    );
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lock_child_waits_for_parent_release() {
    let Ok(root) = std::env::var("AGENT_FORGE_LOCK_CHILD_ROOT") else {
        return;
    };
    println!("READY");
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    let _lock = BaseBranchMergeLock::acquire(
        std::path::Path::new(&root),
        &scope("main"),
        Duration::from_secs(3),
    )
    .unwrap();
    println!("ACQUIRED");
    std::io::stdout().flush().unwrap();
}

#[test]
fn different_base_branches_do_not_share_a_lock() {
    let root = fresh_lock_root();
    let _main = BaseBranchMergeLock::acquire(&root, &scope("main"), Duration::ZERO).unwrap();
    let release = BaseBranchMergeLock::acquire(&root, &scope("release/1.x"), Duration::ZERO);
    assert!(release.is_ok());
    drop(release);
    drop(_main);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_scope_is_rejected_before_lock_creation() {
    assert!(matches!(
        BaseBranchScope::new("http://127.0.0.1:3130", "..", "agent-forge-core", "main"),
        Err(MergeLockError::InvalidScope)
    ));
    assert!(matches!(
        BaseBranchScope::new(
            "http://user@127.0.0.1:3130",
            "forge-admin",
            "agent-forge-core",
            "main"
        ),
        Err(MergeLockError::InvalidScope)
    ));
}

#[cfg(unix)]
#[test]
fn symbolic_link_lock_directory_is_rejected() {
    use std::os::unix::fs::symlink;

    let real = fresh_lock_root();
    let alias = fresh_lock_root();
    std::fs::create_dir(&real).unwrap();
    symlink(&real, &alias).unwrap();
    assert!(matches!(
        BaseBranchMergeLock::acquire(&alias, &scope("main"), Duration::ZERO),
        Err(MergeLockError::UntrustedLockDirectory)
    ));
    std::fs::remove_file(alias).unwrap();
    std::fs::remove_dir_all(real).unwrap();
}

#[cfg(unix)]
#[test]
fn symbolic_link_lock_file_is_rejected() {
    use std::os::unix::fs::symlink;

    let root = fresh_lock_root();
    let lock = BaseBranchMergeLock::acquire(&root, &scope("main"), Duration::ZERO).unwrap();
    drop(lock);
    let lock_path = std::fs::read_dir(&root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let target = root.with_extension("target");
    std::fs::write(&target, b"not a lock").unwrap();
    std::fs::remove_file(&lock_path).unwrap();
    symlink(&target, &lock_path).unwrap();

    assert!(matches!(
        BaseBranchMergeLock::acquire(&root, &scope("main"), Duration::ZERO),
        Err(MergeLockError::UntrustedLockFile)
    ));

    std::fs::remove_file(lock_path).unwrap();
    std::fs::remove_file(target).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
