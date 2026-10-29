use agent_forge_core::{
    EventJournal, JournalError,
    webhook::{
        MAX_WEBHOOK_BODY_BYTES,
        http::{
            HttpRequestError, HttpResponse, SERVER_BUSY_RETRY_AFTER_SECONDS,
            handle_webhook_request, parse_http_request,
        },
        is_supported_journal_kind,
    },
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{
    env, fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::ExitCode,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

#[cfg(all(feature = "test-crash-injection", not(debug_assertions)))]
compile_error!("test-crash-injection is restricted to debug builds");

#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_CONNECTIONS: usize = 16;
const REJECTION_QUEUE_CAPACITY: usize = 1;
const WORKERS: usize = 4;
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_REQUEST_BYTES: usize = MAX_WEBHOOK_BODY_BYTES + 16 * 1024 + 4;

#[cfg(unix)]
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn shutdown_signal_handler(_: libc::c_int) {
    // Keep signal handling async-signal-safe; normal threads perform all I/O.
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

#[cfg(unix)]
fn install_shutdown_handlers() -> Result<(), String> {
    unsafe {
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::signal(
                signal,
                shutdown_signal_handler as *const () as libc::sighandler_t,
            ) == libc::SIG_ERR
            {
                return Err("could not install webhook shutdown handler".into());
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_shutdown_handlers() -> Result<(), String> {
    Ok(())
}

fn shutdown_requested() -> bool {
    #[cfg(unix)]
    {
        SHUTDOWN_REQUESTED.load(Ordering::Relaxed)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() == 2 && args[0] == "--print-attestation-public-key" {
        let signing_key = read_attestation_signing_key(Path::new(&args[1]))?;
        println!("{}", hex::encode(signing_key.verifying_key().as_bytes()));
        return Ok(());
    }
    if args.len() == 3 && args[0] == "--snapshot-inbox" {
        let source = Path::new(&args[1]);
        let destination = Path::new(&args[2]);
        validate_private_journal_path(source).map_err(|_| {
            "snapshot_refused: source inbox must be in an owner-owned 0700 directory and be an owner-owned 0600 regular file with one link".to_owned()
        })?;
        validate_private_journal_path(destination).map_err(|_| {
            "snapshot_refused: destination parent must be an owner-owned 0700 directory; destination must be absent or a private regular file".to_owned()
        })?;
        let snapshot = EventJournal::open_webhook_inbox(source)
            .snapshot_read_only()
            .map_err(snapshot_refusal)?;
        write_private_snapshot(destination, &snapshot.bytes)?;
        println!(
            "{}",
            serde_json::json!({
                "schema_version": "forgejo-webhook-inbox-snapshot-v1",
                "byte_length": snapshot.bytes.len(),
                "event_count": snapshot.event_count,
                "last_sequence": snapshot.last_sequence,
                "last_event_hash": snapshot.last_event_hash,
                "sha256": snapshot.sha256,
                "source_validation": "event_chain_and_idempotency_keys",
                "source_lock": "exclusive_journal_flock",
                "restore_policy": "reconcile_or_quiesce_and_replay",
                "quiescent_restore_point": false
            })
        );
        return Ok(());
    }
    if args.len() == 4 && args[0] == "--stage-restore-inbox" {
        let source = Path::new(&args[1]);
        let destination = Path::new(&args[2]);
        let expected_sha256 = &args[3];
        if !is_canonical_sha256(expected_sha256) {
            return Err(
                "restore_refused: expected snapshot SHA-256 must be 64 lowercase hexadecimal characters"
                    .to_owned(),
            );
        }
        validate_existing_private_journal_path(source)?;
        validate_private_journal_path(destination).map_err(|_| {
            "restore_refused: destination parent must be an owner-owned 0700 directory; destination must be absent or a private regular file".to_owned()
        })?;

        let snapshot = EventJournal::open_webhook_inbox(source)
            .snapshot_read_only()
            .map_err(restore_refusal)?;
        if snapshot.sha256 != *expected_sha256 {
            return Err(
                "restore_refused: snapshot SHA-256 differs from the independently supplied expected digest; destination was not created"
                    .to_owned(),
            );
        }

        write_private_journal_file(destination, &snapshot.bytes, "restore")?;
        let staged = EventJournal::open_webhook_inbox(destination)
            .snapshot_read_only()
            .map_err(|_| {
                "restore_write_incomplete: staged journal failed read-back validation; preserve both files and do not activate the staged journal".to_owned()
            })?;
        if staged.sha256 != snapshot.sha256 || staged.bytes != snapshot.bytes {
            return Err(
                "restore_write_incomplete: staged journal differs from the validated snapshot; preserve both files and do not activate the staged journal"
                    .to_owned(),
            );
        }
        println!(
            "{}",
            serde_json::json!({
                "schema_version": "forgejo-webhook-inbox-restore-v1",
                "operation": "stage_validated_snapshot",
                "snapshot_sha256": snapshot.sha256,
                "staged_sha256": staged.sha256,
                "expected_sha256_source": "operator_supplied_argument",
                "event_count": snapshot.event_count,
                "last_sequence": snapshot.last_sequence,
                "last_event_hash": snapshot.last_event_hash,
                "source_validation": "event_chain_and_idempotency_keys",
                "destination_validation": "read_back_event_chain_and_hash",
                "source_snapshot_bytes_modified": false,
                "existing_inbox_replaced": false,
                "quiescent_restore_point": false,
                "post_snapshot_delivery_replay": "operator_required",
                "signature_authenticity": "not_established"
            })
        );
        return Ok(());
    }
    if args.len() != 3 && args.len() != 4 {
        return Err(
            "usage: forgejo-webhook-local <127.0.0.1:port> <secret-file> <journal.jsonl> [<receiver-signing-seed-file>] | forgejo-webhook-local --print-attestation-public-key <receiver-signing-seed-file> | forgejo-webhook-local --snapshot-inbox <inbox.jsonl> <new-snapshot.jsonl> | forgejo-webhook-local --stage-restore-inbox <snapshot.jsonl> <new-inbox.jsonl> <expected-sha256>".into(),
        );
    }
    let address = args[0]
        .parse::<SocketAddr>()
        .map_err(|_| "invalid listen address; use a loopback IP and port".to_owned())?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("receiver binds only to a loopback IP and a non-zero port".into());
    }
    install_shutdown_handlers()?;
    let secret = read_secret(Path::new(&args[1]))?;
    let attestation_signing_key = args
        .get(3)
        .map(|path| read_attestation_signing_key(Path::new(path)).map(Arc::new))
        .transpose()?;
    validate_private_journal_path(Path::new(&args[2]))?;
    let journal = Arc::new(EventJournal::open_webhook_inbox(&args[2]));
    let recovered = journal
        .replay()
        .map_err(|error| match error {
            JournalError::InboxTooLarge { maximum_bytes } => format!(
                "webhook inbox exceeds its {maximum_bytes}-byte limit; startup stopped without changing the inbox. Preserve a private copy; this build has no verified compaction or rotation command; do not truncate or edit the inbox"
            ),
            _ => "journal recovery failed".to_owned(),
        })?;
    println!(
        "recovered {} webhook inbox events",
        recovered
            .iter()
            .filter(|event| is_supported_journal_kind(&event.kind))
            .count()
    );
    let listener =
        TcpListener::bind(address).map_err(|_| "could not bind loopback listener".to_owned())?;
    serve(listener, secret, journal, attestation_signing_key)?;
    Ok(())
}

fn snapshot_refusal(error: JournalError) -> String {
    match error {
        JournalError::Corrupt { line } => format!(
            "snapshot_refused: journal integrity validation failed at line {line}; preserve the source and use an operator-approved recovery path; no repair was attempted"
        ),
        JournalError::InboxTooLarge { maximum_bytes } => format!(
            "snapshot_refused: inbox exceeds the {maximum_bytes}-byte limit; preserve the source. This build has no verified compaction or rotation command; do not truncate or edit the inbox"
        ),
        JournalError::LockBusy => {
            "snapshot_refused: inbox lock is busy; retry after the current journal operation completes".to_owned()
        }
        JournalError::InvalidEvent => {
            "snapshot_refused: journal path or metadata failed its private-file checks".to_owned()
        }
        JournalError::Io(_) => {
            "snapshot_refused: journal could not be read safely; preserve the source and inspect the local filesystem".to_owned()
        }
        JournalError::IdempotencyConflict => {
            "snapshot_refused: journal integrity validation failed; preserve the source and inspect it before retrying".to_owned()
        }
    }
}

fn restore_refusal(error: JournalError) -> String {
    match error {
        JournalError::Corrupt { line } => format!(
            "restore_refused: snapshot journal integrity validation failed at line {line}; preserve the source; no repair was attempted"
        ),
        JournalError::InboxTooLarge { maximum_bytes } => format!(
            "restore_refused: snapshot exceeds the {maximum_bytes}-byte limit; preserve the source snapshot. This build has no verified recovery path for an oversized snapshot; do not truncate or edit the snapshot"
        ),
        JournalError::LockBusy => {
            "restore_refused: snapshot is in use; retry after the current journal operation completes"
                .to_owned()
        }
        JournalError::InvalidEvent => {
            "restore_refused: snapshot path or metadata failed its private-file checks".to_owned()
        }
        JournalError::Io(_) => {
            "restore_refused: snapshot could not be read safely; preserve the source and inspect the local filesystem".to_owned()
        }
        JournalError::IdempotencyConflict => {
            "restore_refused: snapshot has conflicting idempotency keys; preserve the source and inspect it before retrying".to_owned()
        }
    }
}

fn is_canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_existing_private_journal_path(path: &Path) -> Result<(), String> {
    validate_private_journal_path(path).map_err(|_| {
        "restore_refused: source snapshot must be in an owner-owned 0700 directory and be an owner-owned 0600 regular file with one link".to_owned()
    })?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(
            "restore_refused: source snapshot must be an existing private regular file".to_owned(),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err("restore_refused: source snapshot does not exist; nothing was staged".to_owned())
        }
        Err(_) => Err(
            "restore_refused: source snapshot metadata is unavailable; nothing was staged"
                .to_owned(),
        ),
    }
}

fn write_private_snapshot(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    write_private_journal_file(destination, bytes, "snapshot")
}

fn write_private_journal_file(
    destination: &Path,
    bytes: &[u8],
    operation: &str,
) -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (destination, bytes);
        Err(format!(
            "{operation}_refused: atomic journal publication requires Linux O_TMPFILE and /proc/self/fd; no destination was created"
        ))
    }

    #[cfg(target_os = "linux")]
    {
        use std::{
            ffi::CString,
            os::{
                fd::AsRawFd,
                fd::FromRawFd,
                unix::ffi::OsStrExt,
                unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
            },
        };

        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let filename = destination.file_name().ok_or_else(|| {
            format!("{operation}_refused: destination must name a file in a private directory")
        })?;
        let filename = CString::new(filename.as_bytes()).map_err(|_| {
            format!("{operation}_refused: destination file name contains an invalid NUL byte")
        })?;
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent)
            .map_err(|_| {
                format!("{operation}_refused: destination parent could not be opened safely")
            })?;
        let directory_metadata = directory.metadata().map_err(|_| {
            format!("{operation}_refused: destination parent metadata is unavailable")
        })?;
        if !directory_metadata.is_dir()
            || directory_metadata.uid() != unsafe { libc::geteuid() }
            || directory_metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err(format!(
                "{operation}_refused: destination parent must be an owner-owned 0700 directory"
            ));
        }

        let dot = c".";
        let staging_fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                dot.as_ptr(),
                libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
                0o600,
            )
        };
        if staging_fd < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::EOPNOTSUPP || code == libc::EINVAL
            ) {
                return Err(format!(
                    "{operation}_refused: destination filesystem does not support anonymous atomic staging; choose a private local filesystem that supports O_TMPFILE"
                ));
            }
            return Err(format!(
                "{operation}_refused: anonymous staging file could not be created; no destination was published, preserve the source and inspect the local filesystem"
            ));
        }
        let mut file = unsafe { fs::File::from_raw_fd(staging_fd) };
        let metadata = file
        .metadata()
        .map_err(|_| format!("{operation}_write_incomplete: preserve the source; anonymous staging metadata is unavailable"))?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 0
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(format!(
                "{operation}_write_incomplete: preserve the source; anonymous staging metadata is unsafe"
            ));
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| {
            format!("{operation}_write_incomplete: preserve the source; private staging permissions could not be established")
        })?;
        file.write_all(bytes).map_err(|_| {
        format!("{operation}_write_incomplete: preserve the source; anonymous staged bytes are incomplete and were not published")
    })?;
        file.sync_all().map_err(|_| {
        format!("{operation}_write_incomplete: preserve the source; anonymous staged bytes are not confirmed durable and were not published")
    })?;

        pause_before_atomic_journal_publish();

        let proc_fd_path = CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))
            .expect("numeric file descriptor path contains no NUL");
        let publish_result = unsafe {
            libc::linkat(
                libc::AT_FDCWD,
                proc_fd_path.as_ptr(),
                directory.as_raw_fd(),
                filename.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        };
        if publish_result != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(format!(
                    "{operation}_refused: destination already exists; choose a new path, no existing file was replaced"
                ));
            }
            return Err(format!(
                "{operation}_write_incomplete: preserve the source; atomic publication failed and no partial destination was published"
            ));
        }
        let published_metadata = file.metadata().map_err(|_| {
        format!("{operation}_write_incomplete: preserve the source; published destination metadata is unavailable")
    })?;
        if !published_metadata.is_file()
            || published_metadata.uid() != unsafe { libc::geteuid() }
            || published_metadata.nlink() != 1
            || published_metadata.permissions().mode() & 0o777 != 0o600
        {
            return Err(format!(
                "{operation}_write_incomplete: preserve the source; published destination metadata is unsafe and must not be activated"
            ));
        }
        pause_after_atomic_journal_publish();
        directory.sync_all().map_err(|_| {
        format!("{operation}_write_incomplete: destination contains complete bytes but directory durability is unconfirmed; preserve the source and verify the staged copy before activation")
    })?;
        Ok(())
    }
}

#[cfg(all(target_os = "linux", feature = "test-crash-injection"))]
fn pause_after_atomic_journal_publish() {
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(marker_path) = env::var_os("AGENT_FORGE_TEST_RESTORE_PAUSE_AFTER_PUBLISH") {
        let mut marker = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(marker_path)
            .expect("create post-publication crash-test barrier");
        marker
            .write_all(b"complete destination linked; directory sync is pending\n")
            .expect("write post-publication crash-test barrier");
        marker
            .sync_all()
            .expect("sync post-publication crash-test barrier");
        loop {
            thread::park_timeout(Duration::from_secs(60));
        }
    }
}

#[cfg(all(target_os = "linux", not(feature = "test-crash-injection")))]
fn pause_after_atomic_journal_publish() {}

#[cfg(all(target_os = "linux", feature = "test-crash-injection"))]
fn pause_before_atomic_journal_publish() {
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(marker_path) = env::var_os("AGENT_FORGE_TEST_RESTORE_PAUSE_BEFORE_PUBLISH") {
        let mut marker = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(marker_path)
            .expect("create crash-test publication barrier");
        marker
            .write_all(b"anonymous staged file is synced; final name is not published\n")
            .expect("write crash-test publication barrier");
        marker
            .sync_all()
            .expect("sync crash-test publication barrier");
        if let Some(resume_path) = env::var_os("AGENT_FORGE_TEST_RESTORE_CONTINUE_AFTER_BARRIER") {
            let resume_path = Path::new(&resume_path);
            while !resume_path.exists() {
                thread::park_timeout(Duration::from_millis(5));
            }
            return;
        }
        loop {
            thread::park_timeout(Duration::from_secs(60));
        }
    }
}

#[cfg(all(target_os = "linux", not(feature = "test-crash-injection")))]
fn pause_before_atomic_journal_publish() {}

fn read_secret(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .map_err(|_| "secret file is unavailable".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "secret file metadata is unavailable".to_owned())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > 4096 {
        return Err("secret file must be a non-empty regular file of at most 4096 bytes".into());
    }
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
        return Err("secret file must be owned by the current user and have one link".into());
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err("secret file permissions must not grant group or other access".into());
    }
    let mut secret = Vec::with_capacity(metadata.len() as usize);
    file.take(4097)
        .read_to_end(&mut secret)
        .map_err(|_| "secret file could not be read".to_owned())?;
    if secret.is_empty() || secret.len() > 4096 {
        return Err("secret file must be a non-empty regular file of at most 4096 bytes".into());
    }
    Ok(secret)
}

fn read_attestation_signing_key(path: &Path) -> Result<SigningKey, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .map_err(|_| "receiver signing seed file is unavailable".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "receiver signing seed metadata is unavailable".to_owned())?;
    if !metadata.file_type().is_file()
        || metadata.len() != 32
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(
            "receiver signing seed must be a 32-byte owner-owned regular file with one link and no group or other access".into(),
        );
    }
    let mut seed = [0u8; 32];
    let mut file = file;
    file.read_exact(&mut seed)
        .map_err(|_| "receiver signing seed could not be read".to_owned())?;
    let signing_key = SigningKey::from_bytes(&seed);
    seed.zeroize();
    Ok(signing_key)
}

fn validate_private_journal_path(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let directory = fs::symlink_metadata(parent)
        .map_err(|_| "journal directory must be owned by the current user with mode 0700")?;
    if !directory.file_type().is_dir()
        || directory.uid() != unsafe { libc::geteuid() }
        || directory.permissions().mode() & 0o777 != 0o700
    {
        return Err("journal directory must be owned by the current user with mode 0700".into());
    }

    match fs::symlink_metadata(path) {
        Ok(file)
            if file.file_type().is_file()
                && file.uid() == unsafe { libc::geteuid() }
                && file.nlink() == 1
                && file.permissions().mode() & 0o777 == 0o600 =>
        {
            Ok(())
        }
        Ok(_) => Err(
            "journal file must be a private owner-owned regular file with one link and mode 0600"
                .into(),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("journal file metadata is unavailable".into()),
    }
}

fn serve(
    listener: TcpListener,
    secret: Vec<u8>,
    journal: Arc<EventJournal>,
    attestation_signing_key: Option<Arc<SigningKey>>,
) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|_| "could not configure loopback listener".to_owned())?;
    let (sender, receiver) = sync_channel::<TcpStream>(MAX_CONNECTIONS);
    let receiver = Arc::new(Mutex::new(receiver));
    let (rejection_sender, rejection_receiver) =
        sync_channel::<TcpStream>(REJECTION_QUEUE_CAPACITY);
    let rejection_worker = thread::Builder::new()
        .name("forgejo-webhook-rejector".to_owned())
        .spawn(move || rejection_worker(rejection_receiver))
        .map_err(|_| "could not start webhook rejection worker".to_owned())?;
    let secret = Arc::new(secret);
    let mut workers = Vec::with_capacity(WORKERS);
    for worker_id in 0..WORKERS {
        let receiver = Arc::clone(&receiver);
        let secret = Arc::clone(&secret);
        let journal = Arc::clone(&journal);
        let attestation_signing_key = attestation_signing_key.clone();
        let worker = thread::Builder::new()
            .name(format!("forgejo-webhook-{worker_id}"))
            .spawn(move || worker(receiver, secret, journal, attestation_signing_key))
            .map_err(|_| "could not start webhook worker".to_owned())?;
        workers.push(worker);
    }
    println!(
        "listening on {}",
        listener
            .local_addr()
            .map_err(|_| "listener address unavailable")?
    );
    let mut serve_error = None;
    loop {
        if shutdown_requested() {
            println!("shutdown requested; draining accepted webhook requests");
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => match sender.try_send(stream) {
                Ok(()) => {}
                Err(TrySendError::Full(stream)) => match rejection_sender.try_send(stream) {
                    Ok(()) => {}
                    // Keep overload handling bounded if even the rejection worker is full.
                    Err(TrySendError::Full(stream)) => drop(stream),
                    Err(TrySendError::Disconnected(_)) => {
                        serve_error = Some("webhook rejection worker stopped".to_owned());
                        break;
                    }
                },
                Err(TrySendError::Disconnected(_)) => {
                    serve_error = Some("webhook worker queue stopped".to_owned());
                    break;
                }
            },
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => {
                serve_error = Some("loopback listener failed".into());
                break;
            }
        }
    }
    drop(sender);
    drop(rejection_sender);
    let mut worker_panicked = false;
    for worker in workers {
        if worker.join().is_err() {
            worker_panicked = true;
        }
    }
    if rejection_worker.join().is_err() {
        worker_panicked = true;
    }
    if worker_panicked {
        return Err("webhook worker panicked during shutdown".to_owned());
    }
    match serve_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn rejection_worker(receiver: Receiver<TcpStream>) {
    while let Ok(stream) = receiver.recv() {
        let _ = reject_overloaded(stream);
    }
}

fn reject_overloaded(mut stream: TcpStream) -> io::Result<()> {
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let response = match read_request(&mut stream) {
        Ok(_) => {
            let mut response = error_response(
                503,
                "server_busy",
                true,
                "retry the exact signed delivery after the receiver has drained its bounded request queue",
            );
            response.retry_after_seconds = Some(SERVER_BUSY_RETRY_AFTER_SECONDS);
            response
        }
        Err(error) => response_for_read_error(error),
    };
    write_response(&mut stream, &response)
}

fn worker(
    receiver: Arc<Mutex<Receiver<TcpStream>>>,
    secret: Arc<Vec<u8>>,
    journal: Arc<EventJournal>,
    attestation_signing_key: Option<Arc<SigningKey>>,
) {
    loop {
        let stream = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(stream) = stream else {
            return;
        };
        let _ = handle_connection(
            stream,
            &secret,
            &journal,
            attestation_signing_key.as_deref(),
        );
    }
}

fn handle_connection(
    mut stream: TcpStream,
    secret: &[u8],
    journal: &EventJournal,
    attestation_signing_key: Option<&SigningKey>,
) -> io::Result<()> {
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let response = match read_request(&mut stream) {
        Ok(request) => match attestation_signing_key {
            Some(signing_key) => agent_forge_core::webhook::http::handle_webhook_request_attested(
                &request,
                secret,
                journal,
                signing_key,
            ),
            None => handle_webhook_request(&request, secret, journal),
        },
        Err(error) => response_for_read_error(error),
    };
    #[cfg(feature = "test-crash-injection")]
    crash_after_selected_webhook_append(&response);
    write_response(&mut stream, &response)
}

#[cfg(feature = "test-crash-injection")]
fn crash_after_selected_webhook_append(response: &HttpResponse) {
    let Ok(selected_delivery_id) = env::var("AGENT_FORGE_TEST_CRASH_AFTER_WEBHOOK_APPEND") else {
        return;
    };
    let response_delivery_id = response.body["delivery_id"].as_str();
    let newly_appended = response.status == 202
        && response.body["accepted"].as_bool() == Some(true)
        && response.body["duplicate"].as_bool() == Some(false);
    if !newly_appended || response_delivery_id != Some(selected_delivery_id.as_str()) {
        return;
    }

    // This failpoint exists only in debug builds with an explicit test feature.
    // It models process death after append/sync and before the HTTP acknowledgement.
    unsafe {
        libc::kill(libc::getpid(), libc::SIGKILL);
    }
    std::process::abort();
}

fn response_for_read_error(error: ReadError) -> HttpResponse {
    match error {
        ReadError::Http(HttpRequestError::HeaderTooLarge) => error_response(
            431,
            "headers_too_large",
            false,
            "keep request headers below 16 KiB",
        ),
        ReadError::Http(HttpRequestError::BodyTooLarge) => error_response(
            413,
            "body_too_large",
            false,
            "keep webhook bodies below 1 MiB",
        ),
        ReadError::Http(HttpRequestError::Incomplete | HttpRequestError::Malformed) => {
            error_response(
                400,
                "request_malformed",
                false,
                "send a complete, well-formed request with at most one Content-Length, no Transfer-Encoding, and one Host for HTTP/1.1",
            )
        }
        ReadError::Timeout => {
            error_response(408, "request_timeout", true, "retry the webhook delivery")
        }
        ReadError::Incomplete => error_response(
            408,
            "request_incomplete",
            true,
            "retry the webhook delivery with its complete original request",
        ),
        ReadError::Io => error_response(
            400,
            "request_unreadable",
            true,
            "retry the webhook delivery",
        ),
    }
}

enum ReadError {
    Http(HttpRequestError),
    Incomplete,
    Timeout,
    Io,
}

fn read_request(
    stream: &mut TcpStream,
) -> Result<agent_forge_core::webhook::http::HttpRequest, ReadError> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(ReadError::Timeout);
        }
        match parse_http_request(&bytes) {
            Ok(request) => return Ok(request),
            Err(HttpRequestError::Incomplete) => {}
            Err(error) => return Err(ReadError::Http(error)),
        }
        if bytes.len() >= MAX_REQUEST_BYTES {
            return Err(ReadError::Http(HttpRequestError::BodyTooLarge));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ReadError::Timeout);
        }
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| ReadError::Io)?;
        let count = match stream.read(&mut chunk) {
            Ok(0) => return Err(ReadError::Incomplete),
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(ReadError::Timeout);
            }
            Err(_) => return Err(ReadError::Io),
        };
        if bytes.len().saturating_add(count) > MAX_REQUEST_BYTES {
            return Err(ReadError::Http(HttpRequestError::BodyTooLarge));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

fn write_response(stream: &mut TcpStream, response: &HttpResponse) -> io::Result<()> {
    let body = serde_json::to_vec(&response.body).map_err(io::Error::other)?;
    let reason = match response.status {
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        431 => "Request Header Fields Too Large",
        409 => "Conflict",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\n",
        response.status,
        reason,
        body.len()
    )?;
    if let Some(seconds) = response.retry_after_seconds {
        write!(stream, "Retry-After: {seconds}\r\n")?;
    }
    write!(stream, "Connection: close\r\n\r\n")?;
    stream.write_all(&body)?;
    stream.flush()
}

fn error_response(status: u16, code: &str, retryable: bool, action: &str) -> HttpResponse {
    HttpResponse {
        status,
        retry_after_seconds: None,
        body: json!({
            "schema_version": 1,
            "accepted": false,
            "error": {"code": code, "retryable": retryable, "action": action},
        }),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::{
        io::{Read, Write},
        net::Shutdown,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    type HmacSha256 = Hmac<Sha256>;

    struct ScratchDir(std::path::PathBuf);

    impl ScratchDir {
        fn new(prefix: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn signed_delivery_request(delivery_id: &str, body: &[u8]) -> Vec<u8> {
        let secret = b"test-only-secret";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());
        let headers = format!(
            "POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Forgejo-Delivery: {delivery_id}\r\nX-Forgejo-Event: issues\r\nX-Forgejo-Signature: {signature}\r\n\r\n",
            body.len()
        );
        let mut request = headers.into_bytes();
        request.extend_from_slice(body);
        request
    }

    #[test]
    fn snapshot_and_restore_capacity_refusals_do_not_claim_a_migration_exists() {
        let inbox_refusal = snapshot_refusal(JournalError::InboxTooLarge { maximum_bytes: 64 });
        assert!(inbox_refusal.contains("no verified compaction or rotation command"));
        assert!(inbox_refusal.contains("do not truncate or edit the inbox"));
        assert!(!inbox_refusal.contains("approved migration"));

        let snapshot_refusal = restore_refusal(JournalError::InboxTooLarge { maximum_bytes: 64 });
        assert!(snapshot_refusal.contains("no verified recovery path for an oversized snapshot"));
        assert!(snapshot_refusal.contains("do not truncate or edit the snapshot"));
        assert!(!snapshot_refusal.contains("approved migration"));
    }

    fn send_request(address: SocketAddr, request: &[u8]) -> (u16, serde_json::Value) {
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(request).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        let response = String::from_utf8(response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    #[test]
    fn slow_drip_request_hits_total_read_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let journal_path = env::temp_dir().join(format!(
            "webhook-timeout-{}-{nonce}.jsonl",
            std::process::id()
        ));
        let journal = Arc::new(EventJournal::open(&journal_path));
        let server_journal = Arc::clone(&journal);
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, b"test-only-secret", &server_journal, None).unwrap();
        });

        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(1)))
            .unwrap();
        client
            .write_all(b"POST /forgejo/webhook HTTP/1.1\r\nHost:")
            .unwrap();
        for _ in 0..12 {
            thread::sleep(Duration::from_millis(400));
            if client.write_all(b"x").is_err() {
                break;
            }
            let mut response_probe = [0_u8; 1];
            match client.peek(&mut response_probe) {
                Ok(_) => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => break,
            }
        }
        let _ = client.shutdown(Shutdown::Write);
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        server.join().unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 408 "),
            "received {response:?}"
        );
        drop(journal);
        if journal_path.exists() {
            fs::remove_file(&journal_path).unwrap();
        }
        let lock_path = journal_path.with_extension("jsonl.lock");
        if lock_path.exists() {
            fs::remove_file(lock_path).unwrap();
        }
    }

    #[test]
    fn truncated_half_closed_request_gets_a_retryable_timeout_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let journal_path = env::temp_dir().join(format!(
            "webhook-truncated-{}-{nonce}.jsonl",
            std::process::id()
        ));
        let journal = Arc::new(EventJournal::open(&journal_path));
        let server_journal = Arc::clone(&journal);
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, b"test-only-secret", &server_journal, None).unwrap();
        });

        let mut client = TcpStream::connect(address).unwrap();
        client
            .write_all(b"POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 10\r\n\r\nshort")
            .unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        server.join().unwrap();

        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 408 "),
            "received {response:?}"
        );
        let body = response.split_once("\r\n\r\n").unwrap().1;
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["error"]["code"], "request_incomplete");
        assert_eq!(body["error"]["retryable"], true);
        assert_eq!(
            body["error"]["action"],
            "retry the webhook delivery with its complete original request"
        );
        assert!(journal.replay().unwrap().is_empty());
        drop(journal);
        if journal_path.exists() {
            fs::remove_file(&journal_path).unwrap();
        }
        let lock_path = journal_path.with_extension("jsonl.lock");
        if lock_path.exists() {
            fs::remove_file(lock_path).unwrap();
        }
    }

    #[test]
    fn complete_redelivery_after_truncation_is_accepted_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let journal_path = env::temp_dir().join(format!(
            "webhook-retry-{}-{nonce}.jsonl",
            std::process::id()
        ));
        let journal = Arc::new(EventJournal::open(&journal_path));
        let server_journal = Arc::clone(&journal);
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                handle_connection(stream, b"test-only-secret", &server_journal, None).unwrap();
            }
        });

        let body = br#"{"action":"opened","repository":{"full_name":"forge-admin/agent-forge-core"},"issue":{"number":1,"state":"open"}}"#;
        let full_request = signed_delivery_request("delivery-retry-1", body);
        let body_start = full_request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let (status, response) = send_request(address, &full_request[..body_start + 5]);
        assert_eq!(status, 408);
        assert_eq!(response["error"]["code"], "request_incomplete");
        assert!(journal.replay().unwrap().is_empty());

        let (status, response) = send_request(address, &full_request);
        assert_eq!(status, 202);
        assert_eq!(response["duplicate"], false);
        assert_eq!(journal.replay().unwrap().len(), 1);

        let (status, response) = send_request(address, &full_request);
        assert_eq!(status, 202);
        assert_eq!(response["duplicate"], true);
        server.join().unwrap();
        assert_eq!(journal.replay().unwrap().len(), 1);

        drop(journal);
        if journal_path.exists() {
            fs::remove_file(&journal_path).unwrap();
        }
        let lock_path = journal_path.with_extension("jsonl.lock");
        if lock_path.exists() {
            fs::remove_file(lock_path).unwrap();
        }
    }

    #[test]
    fn secret_with_multiple_hard_links_is_rejected() {
        use std::os::unix::fs::OpenOptionsExt;

        let directory = ScratchDir::new("webhook-secret-hardlink");
        let secret_path = directory.0.join("secret");
        let linked_path = directory.0.join("secret-link");
        let mut secret = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&secret_path)
            .unwrap();
        secret.write_all(b"synthetic-only-secret").unwrap();
        secret.sync_all().unwrap();
        fs::hard_link(&secret_path, &linked_path).unwrap();

        assert!(
            read_secret(&secret_path).is_err(),
            "the receiver must refuse a secret with another filesystem name"
        );
    }
}
