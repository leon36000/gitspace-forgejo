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
    if args.len() != 3 && args.len() != 4 {
        return Err(
            "usage: forgejo-webhook-local <127.0.0.1:port> <secret-file> <journal.jsonl> [<receiver-signing-seed-file>] | forgejo-webhook-local --print-attestation-public-key <receiver-signing-seed-file>".into(),
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
                "webhook inbox exceeds its {maximum_bytes}-byte limit; preserve a backup and apply a verified compaction or migration before restart"
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
