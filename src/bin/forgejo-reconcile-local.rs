#![recursion_limit = "256"]

use agent_forge_core::{
    EVENT_JOURNAL_LOCK_WAIT_MS, EventJournal, ExpectedBinding, GateError, JournalAttestationError,
    JournalError, JournalEvent, MAX_WEBHOOK_INBOX_BYTES, Policy, ProofTargetV2, SignedProof,
    TargetBoundProofV2,
    forgejo::{
        BaseBranchMergeLock, BaseBranchScope, MergeApprovalError, MergeLockError,
        MergePreflightContextV1, MergePreflightError, MergeReadinessError,
        MergeWriterAdmissionErrorV1, MergeWriterAdmissionProfileV1, PullRequestReviewBaseBindingV1,
        PullRequestReviewBaseBindingsSnapshotV1, PullRequestReviewPageBodyV1,
        PullRequestReviewScopeV1, PullRequestReviewsSnapshotV1, PullRequestSnapshotV1,
        collect_pull_request_review_json_pages, encode_url_path_segment,
        evaluate_target_bound_merge_readiness_v2, valid_branch, validate_merge_preflight,
        validate_merge_writer_admission_v1,
    },
    supervisor_capability::{
        MergeCapabilityTargetV1, SupervisorCapabilityErrorV1, VerifiedSupervisorMergeCapabilityV1,
    },
    verify_journal_event_attestation,
    webhook::{
        SUPPORTED_EVENTS, WEBHOOK_EVENT_CONTRACTS_V1, WEBHOOK_EVENT_CONTRACTS_V2,
        WEBHOOK_EVENT_CONTRACTS_V3, WEBHOOK_EVENT_CONTRACTS_V4, WEBHOOK_EVENT_CONTRACTS_V5,
        WEBHOOK_EVENT_CONTRACTS_V6, WEBHOOK_EVENT_CONTRACTS_V7, WEBHOOK_EVENT_CONTRACTS_V8,
        WEBHOOK_EVENT_CONTRACTS_V9, WebhookError, quarantine_for_source,
        quarantine_invalid_webhook_delivery, reconcile::*,
    },
};
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    env,
    fs::{self, File, OpenOptions},
    io::{Cursor, Read},
    os::fd::{AsRawFd, FromRawFd, RawFd},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LOCAL_REPOSITORY_OWNER: &str = "forge-admin";
const LOCAL_REPOSITORY_NAME: &str = "agent-forge-core";
const LOCAL_FORGEJO_ORIGIN: &str = "http://127.0.0.1:3130";
const MISSION_012_REPOSITORY_OWNER: &str = "forge-admin";
const MISSION_012_REPOSITORY_NAME: &str = "mission-012-clamp";
const MISSION_012_FORGEJO_ORIGIN: &str = "http://127.0.0.1:3132";
const MERGE_TARGET_PROFILE_FLAG: &str = "--target-profile";
const RECONCILE_ONLY_TARGET_PROFILE_FLAG: &str = "--reconcile-target-profile";
const MERGE_WRITER_ADMISSION_PROFILE_FLAG: &str = "--writer-admission-profile";
const SUPERVISOR_CAPABILITY_FD_FLAG: &str = "--supervisor-capability-fd";
const MAX_CONFIG_BYTES: usize = 4096;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CURL_HEADER_JSON_BYTES: usize = 64 * 1024;
const HTTP_STATUS_TRAILER_BYTES: usize = 256;
const CURL_STATUS_MARKER: &[u8] = b"\n__AGENT_FORGE_STATUS__";
const CURL_HEADERS_MARKER: &[u8] = b"\n__AGENT_FORGE_HEADERS__";
const CURL_END_MARKER: &[u8] = b"\n__AGENT_FORGE_END__";
const CURL_EXIT_FILE_TOO_LARGE: i32 = 63;
const CHILD_DEADLINE_POLL_INTERVAL: Duration = Duration::from_millis(1);
const MAX_EXPECTED_BINDING_BYTES: usize = 64 * 1024;
const MAX_PROOF_BYTES: usize = 1024 * 1024;
const MAX_VERIFIER_KEY_BYTES: usize = 4096;
const FORGEJO_REST_API_VERSION: &str = "forgejo-rest-v1";
const RECONCILE_SINGLE_SCHEMA_VERSION: u32 = 19;
const RECONCILE_BATCH_SCHEMA_VERSION: u32 = 23;
const MERGE_PREFLIGHT_SCHEMA_VERSION: u32 = 1;
const CAPABILITIES_SCHEMA_VERSION: u32 = 28;
const CAPABILITIES_API_VERSION: &str = "forgejo-reconcile-capabilities-v28";
const MERGE_PREFLIGHT_BASE_BRANCH: &str = "main";
const MERGE_LOCK_ROOT_PREFIX: &str = "agent-forge-merge-locks-";
const MERGE_LOCK_WAIT: Duration = Duration::from_secs(5);
const MAX_FORGEJO_API_TOKEN_BYTES: usize = 4096;
const HOST_CREDENTIAL_PIPE_READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_REVIEW_BASE_BINDING_BYTES: usize = 64 * 1024;
const MAX_MISSION_ADMISSION_BYTES: usize = 256 * 1024;
const MAX_POLICY_BYTES: usize = 64 * 1024;
const MAX_REVIEW_PAGES: usize = 201;
const MERGE_REQUEST_PATH_SUFFIX: &str = "/merge";
const MERGE_REVIEW_PAGE_SIZE: u64 = 50;
const COMMIT_STATUS_PAGE_SIZE: u64 = 50;
const MAX_COMMIT_STATUS_PAGES: u64 = 10;
const MAX_COMMIT_STATUS_COUNT: u64 = COMMIT_STATUS_PAGE_SIZE * MAX_COMMIT_STATUS_PAGES;
const MISSION_ADMISSION_FILE_FLAG: &str = "--mission-admission-file";
const MISSION_EXECUTION_JOURNAL_FLAG: &str = "--mission-execution-journal";
const RECONCILE_HOST_CREDENTIAL_FD_FLAG: &str = "--host-credential-fd";
const TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG: &str =
    "--trusted-webhook-attestation-public-key-hex";
const MAX_TRUSTED_WEBHOOK_ATTESTATION_KEYS: usize = 8;
const GIT_TREE_REQUEST_PAGE_SIZE: usize = 1000;
const MAX_GIT_TREE_ENTRY_COUNT: usize = 100_000;
const MAX_GIT_TREE_PAGES: usize = 100;

fn merge_lock_root_for_uid(uid: libc::uid_t) -> PathBuf {
    Path::new("/tmp").join(format!("{MERGE_LOCK_ROOT_PREFIX}{uid}"))
}

fn is_canonical_positive_decimal(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|number| number > 0 && number.to_string() == value)
}

fn is_safe_forgejo_login_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_canonical_review_page_query(query: &str) -> bool {
    query
        .strip_prefix(&format!("limit={MERGE_REVIEW_PAGE_SIZE}&page="))
        .is_some_and(|page| {
            is_canonical_positive_decimal(page)
                && page
                    .parse::<u64>()
                    .is_ok_and(|page| page <= MAX_REVIEW_PAGES as u64)
        })
}

fn is_canonical_tree_page_query(query: &str) -> bool {
    query
        .strip_prefix("page=")
        .and_then(|query| query.split_once('&'))
        .is_some_and(|(page, rest)| {
            rest == format!("per_page={GIT_TREE_REQUEST_PAGE_SIZE}&recursive=false")
                && is_canonical_positive_decimal(page)
                && page
                    .parse::<u64>()
                    .is_ok_and(|page| page <= MAX_GIT_TREE_PAGES as u64)
        })
}

fn valid_canonical_commit_status_sha(value: &str) -> bool {
    valid_full_git_object_id(value) && value.bytes().all(|byte| !byte.is_ascii_uppercase())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalForgejoConfigDocument {
    schema_version: u32,
    origin: String,
    owner: String,
    repository: String,
}

struct LocalForgejoConfigV1 {
    origin: String,
    owner: String,
    repository: String,
}

struct ReconcileInvocationV1 {
    config: LocalForgejoConfigV1,
    journal_path: std::ffi::OsString,
    delivery_ids: Vec<String>,
    credential_fd: Option<RawFd>,
    trusted_attestation_keys: Vec<VerifyingKey>,
}

/// Read-only reconciliation fixture kept separate from merge target profiles.
/// The command line selects only this name; its origin and repository remain
/// indivisible compile-time literals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReconcileOnlyTargetProfile {
    OrgTeamReviewE2E,
}

impl ReconcileOnlyTargetProfile {
    fn selector(self) -> &'static str {
        match self {
            Self::OrgTeamReviewE2E => "org-team-review-e2e",
        }
    }

    fn config(self) -> LocalForgejoConfigV1 {
        match self {
            Self::OrgTeamReviewE2E => LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: "forge-team-org".to_owned(),
                repository: "team-review-readback".to_owned(),
            },
        }
    }

    fn parse(selector: &str) -> Result<Self, String> {
        match selector {
            "org-team-review-e2e" => Ok(Self::OrgTeamReviewE2E),
            _ => Err(
                "unknown Forgejo reconciliation profile; expected org-team-review-e2e".to_owned(),
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct LocalApiRetryPolicy {
    max_attempts: usize,
    request_timeout: Duration,
    total_budget: Duration,
    backoff: [Duration; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LocalApiRequestPolicy {
    RepositoryApi,
    ReconciliationReadOnly,
    MergePolicyReadOnly,
    MergeAuthority,
}

impl Default for LocalApiRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            request_timeout: Duration::from_secs(5),
            total_budget: Duration::from_secs(30),
            backoff: [Duration::from_millis(100), Duration::from_millis(200)],
        }
    }
}

/// Compile-time loopback fixtures the local merge gate may target. Each
/// variant is one indivisible (origin, owner, repository) literal tuple; no
/// origin, host, port or repository text from the command line or a file can
/// reach a merge request. The base branch is always `main`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalForgejoTargetProfile {
    /// MERGE-014 fixture and the unchanged default when nothing is selected.
    Merge014AgentForgeCore,
    /// MISSION-012 PR sandbox. Reconciliation and merge authority may select
    /// this fixed target; read-only preflight keeps the default target.
    Mission012Clamp,
}

impl LocalForgejoTargetProfile {
    const ALL: [Self; 2] = [Self::Merge014AgentForgeCore, Self::Mission012Clamp];

    fn selector(self) -> &'static str {
        match self {
            Self::Merge014AgentForgeCore => "merge-014-agent-forge-core",
            Self::Mission012Clamp => "mission-012-clamp",
        }
    }

    fn origin(self) -> &'static str {
        match self {
            Self::Merge014AgentForgeCore => LOCAL_FORGEJO_ORIGIN,
            Self::Mission012Clamp => MISSION_012_FORGEJO_ORIGIN,
        }
    }

    fn owner(self) -> &'static str {
        match self {
            Self::Merge014AgentForgeCore => LOCAL_REPOSITORY_OWNER,
            Self::Mission012Clamp => MISSION_012_REPOSITORY_OWNER,
        }
    }

    fn repository(self) -> &'static str {
        match self {
            Self::Merge014AgentForgeCore => LOCAL_REPOSITORY_NAME,
            Self::Mission012Clamp => MISSION_012_REPOSITORY_NAME,
        }
    }

    /// Exact, case-sensitive selector match; no trimming or normalization.
    fn parse(selector: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|profile| profile.selector() == selector)
            .ok_or_else(|| {
                "unknown Forgejo target profile; expected merge-014-agent-forge-core or mission-012-clamp"
                    .to_owned()
            })
    }

    fn config(self) -> LocalForgejoConfigV1 {
        LocalForgejoConfigV1 {
            origin: self.origin().to_owned(),
            owner: self.owner().to_owned(),
            repository: self.repository().to_owned(),
        }
    }
}

impl Default for LocalForgejoConfigV1 {
    fn default() -> Self {
        LocalForgejoTargetProfile::Merge014AgentForgeCore.config()
    }
}

impl LocalForgejoConfigV1 {
    /// Loopback fixtures listen on an ephemeral port; owner and repository
    /// still come from the profile so the request scope stays the profile's.
    #[cfg(test)]
    fn for_profile_at_test_origin(profile: LocalForgejoTargetProfile, origin: String) -> Self {
        Self {
            origin,
            ..profile.config()
        }
    }

    fn load(path: &Path) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| "cannot inspect adapter config file".to_owned())?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_CONFIG_BYTES as u64 {
            return Err(
                "adapter config must be a regular file no larger than 4096 bytes".to_owned(),
            );
        }

        let mut file =
            File::open(path).map_err(|_| "cannot open adapter config file".to_owned())?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "cannot read adapter config file".to_owned())?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(
                "adapter config must be a regular file no larger than 4096 bytes".to_owned(),
            );
        }

        let document: LocalForgejoConfigDocument = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid adapter config: {error}"))?;
        if document.schema_version != 1 {
            return Err("adapter config schema_version must be 1".to_owned());
        }
        if document.origin != LOCAL_FORGEJO_ORIGIN {
            return Err(format!(
                "adapter origin must be exactly {LOCAL_FORGEJO_ORIGIN}"
            ));
        }
        if document.owner != LOCAL_REPOSITORY_OWNER || document.repository != LOCAL_REPOSITORY_NAME
        {
            return Err(format!(
                "adapter config repository must be {LOCAL_REPOSITORY_OWNER}/{LOCAL_REPOSITORY_NAME}"
            ));
        }

        Ok(Self {
            origin: document.origin,
            owner: document.owner,
            repository: document.repository,
        })
    }
}

fn parse_reconcile_invocation(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<ReconcileInvocationV1, String> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let mut index = 0;
    let mut config = None;
    let mut config_seen = false;
    let mut target_profile = None;
    let mut reconcile_only_target_profile = None;
    let mut credential_fd = None;
    let mut credential_seen = false;
    let mut trusted_attestation_keys = Vec::new();

    while let Some(argument) = arguments.get(index) {
        match argument.to_str() {
            Some("--config") => {
                if config_seen {
                    return Err("--config may be supplied only once".to_owned());
                }
                if target_profile.is_some() {
                    return Err("--config cannot be combined with --target-profile".to_owned());
                }
                if reconcile_only_target_profile.is_some() {
                    return Err(
                        "--config cannot be combined with --reconcile-target-profile".to_owned(),
                    );
                }
                let path = arguments.get(index + 1).ok_or_else(usage)?;
                config = Some(LocalForgejoConfigV1::load(Path::new(path))?);
                config_seen = true;
                index += 2;
            }
            Some(MERGE_TARGET_PROFILE_FLAG) => {
                if target_profile.is_some() {
                    return Err("--target-profile may be supplied only once".to_owned());
                }
                if reconcile_only_target_profile.is_some() {
                    return Err(
                        "--target-profile cannot be combined with --reconcile-target-profile"
                            .to_owned(),
                    );
                }
                if config_seen {
                    return Err("--target-profile cannot be combined with --config".to_owned());
                }
                let selector = arguments
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(usage)?;
                target_profile = Some(LocalForgejoTargetProfile::parse(selector)?);
                index += 2;
            }
            Some(RECONCILE_ONLY_TARGET_PROFILE_FLAG) => {
                if reconcile_only_target_profile.is_some() {
                    return Err("--reconcile-target-profile may be supplied only once".to_owned());
                }
                if config_seen {
                    return Err(
                        "--reconcile-target-profile cannot be combined with --config".to_owned(),
                    );
                }
                if target_profile.is_some() {
                    return Err(
                        "--reconcile-target-profile cannot be combined with --target-profile"
                            .to_owned(),
                    );
                }
                let selector = arguments
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(usage)?;
                reconcile_only_target_profile = Some(ReconcileOnlyTargetProfile::parse(selector)?);
                index += 2;
            }
            Some(TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG) => {
                if trusted_attestation_keys.len() >= MAX_TRUSTED_WEBHOOK_ATTESTATION_KEYS {
                    return Err(format!(
                        "at most {MAX_TRUSTED_WEBHOOK_ATTESTATION_KEYS} trusted webhook attestation keys may be configured"
                    ));
                }
                let value = arguments
                    .get(index + 1)
                    .and_then(|argument| argument.to_str())
                    .ok_or_else(|| {
                        format!(
                            "{TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG} requires a canonical lowercase Ed25519 public key"
                        )
                    })?;
                let bytes = hex::decode(value).map_err(|_| {
                    format!(
                        "{TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG} requires a canonical lowercase Ed25519 public key"
                    )
                })?;
                let key_bytes: [u8; 32] = bytes.try_into().map_err(|_| {
                    format!(
                        "{TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG} requires a 32-byte Ed25519 public key"
                    )
                })?;
                if hex::encode(key_bytes) != value {
                    return Err(format!(
                        "{TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG} must use canonical lowercase hexadecimal"
                    ));
                }
                let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| {
                    format!(
                        "{TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG} is not a valid Ed25519 public key"
                    )
                })?;
                if trusted_attestation_keys
                    .iter()
                    .any(|known: &VerifyingKey| known.as_bytes() == key.as_bytes())
                {
                    return Err(format!(
                        "{TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG} values must be unique"
                    ));
                }
                trusted_attestation_keys.push(key);
                index += 2;
            }
            Some(RECONCILE_HOST_CREDENTIAL_FD_FLAG) => {
                if credential_seen {
                    return Err("--host-credential-fd may be supplied only once".to_owned());
                }
                let value = arguments
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| {
                        "host credential descriptor must be a file descriptor number".to_owned()
                    })?;
                let fd = value.parse::<RawFd>().map_err(|_| {
                    "host credential descriptor must be a file descriptor number".to_owned()
                })?;
                if fd <= libc::STDERR_FILENO {
                    return Err(
                        "host credential descriptor must be an anonymous pipe above stderr"
                            .to_owned(),
                    );
                }
                credential_fd = Some(fd);
                credential_seen = true;
                index += 2;
            }
            _ => break,
        }
    }

    let journal_path = arguments.get(index).cloned().ok_or_else(usage)?;
    index += 1;
    let delivery_ids = arguments[index..]
        .iter()
        .map(|argument| {
            argument
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "delivery IDs must be valid UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if delivery_ids.is_empty() {
        return Err(usage());
    }

    Ok(ReconcileInvocationV1 {
        config: config
            .or_else(|| reconcile_only_target_profile.map(ReconcileOnlyTargetProfile::config))
            .or_else(|| target_profile.map(LocalForgejoTargetProfile::config))
            .unwrap_or_default(),
        journal_path,
        delivery_ids,
        credential_fd,
        trusted_attestation_keys,
    })
}

fn read_limited<R: Read>(reader: &mut R, max_bytes: usize) -> Result<Vec<u8>, ApiReadError> {
    let mut output = Vec::new();
    reader
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut output)
        .map_err(|_| ApiReadError::Unavailable)?;
    if output.len() > max_bytes {
        return Err(ApiReadError::InvalidResponse);
    }
    Ok(output)
}

#[cfg(test)]
fn run_command_bounded(
    command: Command,
    max_bytes: usize,
    deadline: Instant,
) -> Result<Vec<u8>, ApiReadError> {
    run_command_bounded_with_input(command, None, max_bytes, deadline)
}

fn run_command_bounded_with_input(
    mut command: Command,
    stdin_input: Option<&[u8]>,
    max_bytes: usize,
    deadline: Instant,
) -> Result<Vec<u8>, ApiReadError> {
    if Instant::now() >= deadline {
        return Err(ApiReadError::Unavailable);
    }
    command.stdin(if stdin_input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    configure_curl_child_lifecycle(&mut command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ApiReadError::Unavailable)?;
    if let Some(input) = stdin_input {
        let Some(mut stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ApiReadError::Unavailable);
        };
        if std::io::Write::write_all(&mut stdin, input).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ApiReadError::Unavailable);
        }
    }
    run_bounded_child(child, max_bytes, deadline, |reader_task, _child_id| {
        std::thread::Builder::new().spawn(reader_task)
    })
}

/// Keep curl from outliving this adapter when its parent exits unexpectedly.
///
/// PR_SET_PDEATHSIG is Linux-specific; capability discovery reports the
/// weaker behavior on other Unix targets instead of claiming this guarantee.
#[cfg(target_os = "linux")]
fn configure_curl_child_lifecycle(command: &mut Command) {
    let expected_parent = unsafe { libc::getpid() };
    // SAFETY: the pre-exec closure only calls async-signal-safe syscalls.
    // The parent PID check closes the race where the CLI exits before the
    // kernel installs the parent-death signal for curl.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != expected_parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_curl_child_lifecycle(_command: &mut Command) {}

#[cfg(test)]
fn run_command_bounded_with_spawner<F>(
    mut command: Command,
    max_bytes: usize,
    deadline: Instant,
    spawn_reader: F,
) -> Result<Vec<u8>, ApiReadError>
where
    F: FnOnce(
        Box<dyn FnOnce() + Send + 'static>,
        u32,
    ) -> std::io::Result<std::thread::JoinHandle<()>>,
{
    if Instant::now() >= deadline {
        return Err(ApiReadError::Unavailable);
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ApiReadError::Unavailable)?;
    run_bounded_child(child, max_bytes, deadline, spawn_reader)
}

fn run_bounded_child<F>(
    mut child: std::process::Child,
    max_bytes: usize,
    deadline: Instant,
    spawn_reader: F,
) -> Result<Vec<u8>, ApiReadError>
where
    F: FnOnce(
        Box<dyn FnOnce() + Send + 'static>,
        u32,
    ) -> std::io::Result<std::thread::JoinHandle<()>>,
{
    if Instant::now() >= deadline {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ApiReadError::Unavailable);
    }
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ApiReadError::InvalidResponse);
    };

    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader_task = Box::new(move || {
        let result = read_limited(&mut stdout, max_bytes).map_err(|error| match error {
            ApiReadError::InvalidResponse => ApiReadError::ResponseTooLarge {
                component: "captured_output",
                maximum_bytes: max_bytes,
            },
            other => other,
        });
        let _ = sender.send(result);
    });
    let reader = match spawn_reader(reader_task, child.id()) {
        Ok(reader) => reader,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ApiReadError::Unavailable);
        }
    };
    let mut output = None;
    loop {
        match receiver.try_recv() {
            Ok(Ok(bytes)) => output = Some(bytes),
            Ok(Err(error)) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(error);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) if output.is_none() => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(ApiReadError::InvalidResponse);
            }
            Err(
                std::sync::mpsc::TryRecvError::Disconnected | std::sync::mpsc::TryRecvError::Empty,
            ) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if Instant::now() >= deadline {
                    let _ = reader.join();
                    return Err(ApiReadError::Unavailable);
                }
                let output = match output {
                    Some(bytes) => Ok(bytes),
                    None => match receiver.recv() {
                        Ok(result) => result,
                        Err(_) => Err(ApiReadError::InvalidResponse),
                    },
                }?;
                let _ = reader.join();
                if status.success() {
                    return Ok(output);
                }
                return if status.code() == Some(CURL_EXIT_FILE_TOO_LARGE) {
                    Err(ApiReadError::ResponseTooLarge {
                        component: "body",
                        maximum_bytes: MAX_RESPONSE_BYTES,
                    })
                } else {
                    Err(ApiReadError::Unavailable)
                };
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(ApiReadError::Unavailable);
            }
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(ApiReadError::Unavailable);
        }
        // Cancellation begins on the first poll after expiry; OS scheduling can add jitter.
        std::thread::sleep(CHILD_DEADLINE_POLL_INTERVAL);
    }
}

fn curl_timeout_argument(timeout: Duration) -> Option<String> {
    let milliseconds = timeout.as_millis();
    if milliseconds == 0 {
        return None;
    }
    Some(format!(
        "{}.{:03}",
        milliseconds / 1000,
        milliseconds % 1000
    ))
}

fn request_url_matches_origin(url: &str, origin: &str) -> bool {
    let Some(path) = url.strip_prefix(origin) else {
        return false;
    };
    path.starts_with('/')
        && !path.starts_with("//")
        && !path
            .bytes()
            .any(|byte| matches!(byte, b'\\' | b'\r' | b'\n' | b'\0' | b'#'))
}

struct ForgejoApiToken(Vec<u8>);

impl ForgejoApiToken {
    fn parse(value: Vec<u8>) -> Result<Self, ApiReadError> {
        if value.is_empty() || value.len() > MAX_FORGEJO_API_TOKEN_BYTES {
            return Err(ApiReadError::InvalidCredential);
        }
        let padding_start = value
            .iter()
            .position(|byte| *byte == b'=')
            .unwrap_or(value.len());
        let token_char = |byte: &u8| {
            byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
        };
        if padding_start == 0
            || !value[..padding_start].iter().all(token_char)
            || !value[padding_start..].iter().all(|byte| *byte == b'=')
        {
            return Err(ApiReadError::InvalidCredential);
        }
        Ok(Self(value))
    }

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

trait HostForgejoCredentialProviderV1: Send + Sync {
    fn api_token(&self) -> Result<Option<ForgejoApiToken>, ApiReadError>;
}

struct FixedHostForgejoCredentialProvider(Vec<u8>);

impl HostForgejoCredentialProviderV1 for FixedHostForgejoCredentialProvider {
    fn api_token(&self) -> Result<Option<ForgejoApiToken>, ApiReadError> {
        ForgejoApiToken::parse(self.0.clone()).map(Some)
    }
}

fn read_inherited_credential_pipe(fd: RawFd) -> Result<ForgejoApiToken, String> {
    if fd <= libc::STDERR_FILENO {
        return Err("host credential descriptor must be an anonymous pipe above stderr".to_owned());
    }
    // Mark the inherited descriptor close-on-exec before spawning curl. The
    // token is sent to curl only through its stdin config, never by argv,
    // environment, journal, or response. It still resides in this process and
    // curl's memory; this CLI does not isolate its process from an agent. The
    // caller must keep credential-bearing invocations in a trusted host
    // process outside the agent execution scope.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err("cannot secure the host credential descriptor".to_owned());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect the host credential descriptor".to_owned())?;
    if !metadata.file_type().is_fifo() {
        return Err("host credential input must be a pipe".to_owned());
    }
    let descriptor_target = fs::read_link(format!("/proc/self/fd/{fd}"))
        .map_err(|_| "cannot verify the anonymous credential pipe".to_owned())?;
    let descriptor_target = descriptor_target.to_string_lossy();
    // This verifies only the descriptor's pipe shape. It does not authenticate
    // the producer or prove that the caller is the only process holding a
    // reader or writer; the trusted caller owns that boundary.
    if !descriptor_target.starts_with("pipe:[") || !descriptor_target.ends_with(']') {
        return Err("host credential input must be an anonymous pipe".to_owned());
    }
    let bytes = read_bounded_pipe_until_eof(&mut file, HOST_CREDENTIAL_PIPE_READ_TIMEOUT)?;
    ForgejoApiToken::parse(bytes).map_err(|_| {
        "host credential input is invalid; inspect it without exposing its value".to_owned()
    })
}

fn read_bounded_pipe_until_eof(file: &mut File, timeout: Duration) -> Result<Vec<u8>, String> {
    read_bounded_pipe_until_eof_after_poll(file, timeout, || {})
}

fn read_bounded_pipe_until_eof_after_poll(
    file: &mut File,
    timeout: Duration,
    mut after_poll: impl FnMut(),
) -> Result<Vec<u8>, String> {
    let status_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if status_flags < 0
        || unsafe {
            libc::fcntl(
                file.as_raw_fd(),
                libc::F_SETFL,
                status_flags | libc::O_NONBLOCK,
            )
        } < 0
    {
        return Err("cannot make host credential pipe nonblocking".to_owned());
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| "host credential pipe deadline is invalid".to_owned())?;
    let mut bytes = Vec::with_capacity(MAX_FORGEJO_API_TOKEN_BYTES);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for host credential pipe EOF".to_owned());
        }
        let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let poll_result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if poll_result < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err("cannot wait for host credential pipe input".to_owned());
        }
        if poll_result == 0 {
            return Err("timed out waiting for host credential pipe EOF".to_owned());
        }
        if descriptor.revents & libc::POLLNVAL != 0 || descriptor.revents & libc::POLLERR != 0 {
            return Err("host credential pipe became unavailable".to_owned());
        }
        if descriptor.revents & (libc::POLLIN | libc::POLLHUP) == 0 {
            continue;
        }

        after_poll();
        let remaining_capacity = MAX_FORGEJO_API_TOKEN_BYTES + 1 - bytes.len();
        let mut buffer = [0; 1024];
        let read_len = buffer.len().min(remaining_capacity);
        match file.read(&mut buffer[..read_len]) {
            Ok(0) => return Ok(bytes),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() > MAX_FORGEJO_API_TOKEN_BYTES {
                    return Err(
                        "host credential input exceeds the configured byte limit".to_owned()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => return Err("cannot read bounded host credential input".to_owned()),
        }
    }
}

struct OriginBoundHostCredentialProvider {
    origin: String,
    provider: Box<dyn HostForgejoCredentialProviderV1>,
}

impl OriginBoundHostCredentialProvider {
    fn token_for(&self, origin: &str) -> Result<Option<ForgejoApiToken>, ApiReadError> {
        if origin == self.origin {
            self.provider.api_token()
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
fn build_curl_request(
    expected_origin: &str,
    url: &str,
    timeout: Duration,
    token: Option<&ForgejoApiToken>,
) -> Result<(Command, Option<Vec<u8>>), ApiReadError> {
    build_curl_api_request(expected_origin, url, "GET", None, timeout, token, false)
}

fn build_curl_api_request(
    expected_origin: &str,
    url: &str,
    method: &str,
    body: Option<&[u8]>,
    timeout: Duration,
    token: Option<&ForgejoApiToken>,
    include_total_count: bool,
) -> Result<(Command, Option<Vec<u8>>), ApiReadError> {
    if !request_url_matches_origin(url, expected_origin) {
        return Err(ApiReadError::Forbidden);
    }
    if !matches!(method, "GET" | "POST")
        || (method == "GET" && body.is_some())
        || body.is_some_and(|body| body.len() > MAX_RESPONSE_BYTES)
    {
        return Err(ApiReadError::InvalidResponse);
    }
    let Some(timeout_seconds) = curl_timeout_argument(timeout) else {
        return Err(ApiReadError::Unavailable);
    };
    let stdin_config = token.map(|token| {
        let token = std::str::from_utf8(token.as_bytes())
            .expect("ForgejoApiToken::parse only accepts ASCII token bytes");
        format!(r#"header = "Authorization: token {token}"{}"#, '\n').into_bytes()
    });
    let mut command = Command::new("/usr/bin/curl");
    command
        .env_clear()
        .args([
            "-q",
            "-sS",
            "--noproxy",
            "*",
            "--proto",
            "=http",
            "--connect-timeout",
            "1",
            "--max-time",
        ])
        .arg(timeout_seconds)
        .arg("--max-filesize")
        .arg(MAX_RESPONSE_BYTES.to_string());
    command.arg("--write-out").arg(if include_total_count {
        "\\n__AGENT_FORGE_STATUS__%{http_code}\\n__AGENT_FORGE_HEADERS__%{header_json}\\n__AGENT_FORGE_END__"
    } else {
        "\\n%{http_code}"
    });
    if stdin_config.is_some() {
        command.args(["--config", "-"]);
    }
    if method != "GET" {
        command.args(["--request", method]);
    }
    if let Some(body) = body {
        let body = std::str::from_utf8(body).map_err(|_| ApiReadError::InvalidResponse)?;
        command.args([
            "--header",
            "Content-Type: application/json",
            "--data-binary",
        ]);
        command.arg(body);
    }
    command.arg(url);
    Ok((command, stdin_config))
}

#[derive(Debug)]
struct ForgejoHttpResponse {
    status: u16,
    total_count: Option<u64>,
    retry_after: Option<RetryAfterValue>,
    body: Option<Value>,
}

fn parse_commit_status_snapshot_response(
    requested_commit_sha: &str,
    owner: &str,
    repository: &str,
    response: ForgejoHttpResponse,
) -> Result<(Value, bool), ApiReadError> {
    if response.status != 200 {
        return Err(ApiReadError::InvalidResponse);
    }
    let body = response.body.ok_or(ApiReadError::InvalidResponse)?;
    let state = body
        .get("state")
        .and_then(Value::as_str)
        .ok_or(ApiReadError::InvalidResponse)?;
    let response_sha = body
        .get("sha")
        .and_then(Value::as_str)
        .ok_or(ApiReadError::InvalidResponse)?;
    let page_count = body
        .get("total_count")
        .and_then(Value::as_u64)
        .ok_or(ApiReadError::InvalidResponse)?;
    let statuses = match body.get("statuses") {
        Some(Value::Null) | None => None,
        Some(Value::Array(statuses)) => Some(statuses),
        _ => return Err(ApiReadError::InvalidResponse),
    };
    let status_count = statuses.map_or(0, |statuses| statuses.len());
    let header_count = response.total_count;
    let empty_status_response = state.is_empty()
        && response_sha.is_empty()
        && page_count == 0
        && status_count == 0
        && header_count.is_none_or(|count| count == 0);
    if empty_status_response {
        return Ok((
            json!({
                "schema_version": "forgejo-commit-status-snapshot-v1",
                "repository": format!("{owner}/{repository}"),
                "commit_sha": requested_commit_sha,
                "current_pr_head_checked": false,
                "observation": "no_statuses",
                "forgejo_reported_combined_state": null,
                "forgejo_reported_page_states": [],
                "status_count": 0,
                "total_count": header_count,
                "pages_read": 1,
                "pagination_complete": true,
                "snapshot_consistency": "single_response",
                "statuses": [],
                "evidence_scope": "forgejo_reported_contexts_only",
                "independent_verification": false,
                "merge_authorizing": false,
            }),
            true,
        ));
    }

    if response_sha != requested_commit_sha
        || !valid_commit_status_state(state)
        || !valid_full_git_object_id(response_sha)
    {
        return Err(ApiReadError::InvalidResponse);
    }
    let repository_full_name = body
        .get("repository")
        .and_then(|value| value.get("full_name"))
        .and_then(Value::as_str)
        .ok_or(ApiReadError::InvalidResponse)?;
    if repository_full_name != format!("{owner}/{repository}") {
        return Err(ApiReadError::InvalidResponse);
    }
    let statuses = statuses.ok_or(ApiReadError::InvalidResponse)?;
    if statuses.is_empty()
        || page_count != statuses.len() as u64
        || statuses.iter().any(|status| {
            status
                .get("status")
                .and_then(Value::as_str)
                .is_none_or(|status| !valid_commit_status_state(status))
                || status.get("context").and_then(Value::as_str).is_none()
        })
    {
        return Err(ApiReadError::InvalidResponse);
    }
    let mut page_contexts = BTreeSet::new();
    for status in statuses {
        let context = status
            .get("context")
            .and_then(Value::as_str)
            .ok_or(ApiReadError::InvalidResponse)?;
        if !page_contexts.insert(context) {
            return Err(ApiReadError::InconsistentPagination);
        }
    }

    let complete = header_count == Some(statuses.len() as u64)
        && statuses.len() <= COMMIT_STATUS_PAGE_SIZE as usize;
    let observation = if complete { "complete" } else { "inconclusive" };
    Ok((
        json!({
            "schema_version": "forgejo-commit-status-snapshot-v1",
            "repository": format!("{owner}/{repository}"),
            "commit_sha": requested_commit_sha,
            "current_pr_head_checked": false,
            "observation": observation,
            "forgejo_reported_combined_state": state,
            "forgejo_reported_page_states": [state],
            "status_count": statuses.len(),
            "total_count": header_count,
            "pages_read": 1,
            "pagination_complete": complete,
            "snapshot_consistency": "single_response",
            "statuses": statuses,
            "evidence_scope": "forgejo_reported_contexts_only",
            "independent_verification": false,
            "merge_authorizing": false,
        }),
        complete,
    ))
}

fn valid_commit_status_state(state: &str) -> bool {
    matches!(
        state,
        "pending" | "success" | "failure" | "error" | "warning" | "skipped"
    )
}

fn pull_request_status_binding(pr: &PullRequestSnapshotV1) -> Value {
    json!({
        "number": pr.number,
        "state": pr.state,
        "merged": pr.merged,
        "base_branch": pr.base_branch,
        "base_sha": pr.base_sha,
        "head_sha": pr.head_sha,
    })
}

fn pull_request_status_binding_changed_fields(
    before: &PullRequestSnapshotV1,
    after: &PullRequestSnapshotV1,
) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if before.number != after.number {
        changed.push("number");
    }
    if before.state != after.state {
        changed.push("state");
    }
    if before.merged != after.merged {
        changed.push("merged");
    }
    if before.base_branch != after.base_branch {
        changed.push("base_branch");
    }
    if before.base_sha != after.base_sha {
        changed.push("base_sha");
    }
    if before.head_sha != after.head_sha {
        changed.push("head_sha");
    }
    changed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetryAfterValue {
    Delay(Duration),
    Invalid,
}

struct ForgejoReadError {
    error: ApiReadError,
    retry_after: Option<RetryAfterValue>,
}

impl From<ApiReadError> for ForgejoReadError {
    fn from(error: ApiReadError) -> Self {
        Self {
            error,
            retry_after: None,
        }
    }
}

impl ForgejoReadError {
    fn as_api_read_error(&self) -> ApiReadError {
        match (&self.error, self.retry_after) {
            (ApiReadError::HttpStatus(status), Some(RetryAfterValue::Delay(delay))) => {
                ApiReadError::HttpStatusWithRetryAfter {
                    status: *status,
                    retry_after_ms: delay.as_millis().min(u64::MAX as u128) as u64,
                }
            }
            (ApiReadError::HttpStatus(status), Some(RetryAfterValue::Invalid)) => {
                ApiReadError::HttpStatusWithInvalidRetryAfter { status: *status }
            }
            _ => self.error.clone(),
        }
    }
}

/// Deliberately narrow test adapter: credentials are absent by default, any
/// injected host provider is bound to the configured origin, and only the
/// synthetic repository in the local Forgejo instance is allowlisted.
struct LocalLoopbackForgejoApi {
    config: LocalForgejoConfigV1,
    request_policy: LocalApiRequestPolicy,
    retry_policy: LocalApiRetryPolicy,
    deadline: Instant,
    upstream_read_attempts: AtomicUsize,
    authorization_rejected: AtomicBool,
    credential_provider: Option<OriginBoundHostCredentialProvider>,
}

impl Default for LocalLoopbackForgejoApi {
    fn default() -> Self {
        Self::new(LocalForgejoConfigV1::default())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeAuthorityPostBodyV1 {
    #[serde(rename = "Do")]
    action: String,
    head_commit_id: String,
}

fn merge_authority_post_body_is_allowed(body: &[u8]) -> bool {
    serde_json::from_slice::<MergeAuthorityPostBodyV1>(body).is_ok_and(|request| {
        request.action == "merge" && valid_full_git_object_id(&request.head_commit_id)
    })
}

impl LocalLoopbackForgejoApi {
    fn new(config: LocalForgejoConfigV1) -> Self {
        Self::with_policy(config, LocalApiRetryPolicy::default())
    }

    fn with_policy(config: LocalForgejoConfigV1, retry_policy: LocalApiRetryPolicy) -> Self {
        Self {
            config,
            request_policy: LocalApiRequestPolicy::RepositoryApi,
            retry_policy,
            deadline: Instant::now() + retry_policy.total_budget,
            upstream_read_attempts: AtomicUsize::new(0),
            authorization_rejected: AtomicBool::new(false),
            credential_provider: None,
        }
    }

    #[cfg(test)]
    fn with_host_credentials(
        config: LocalForgejoConfigV1,
        retry_policy: LocalApiRetryPolicy,
        provider: Box<dyn HostForgejoCredentialProviderV1>,
    ) -> Self {
        let credential_origin = config.origin.clone();
        let mut api = Self::with_policy(config, retry_policy);
        api.credential_provider = Some(OriginBoundHostCredentialProvider {
            origin: credential_origin,
            provider,
        });
        api
    }

    fn request_api_once(
        &self,
        path: &str,
        method: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<ForgejoHttpResponse, ApiReadError> {
        if self.request_policy == LocalApiRequestPolicy::ReconciliationReadOnly
            && !self.reconciliation_request_is_allowed(path, method, body)
        {
            return Err(ApiReadError::Forbidden);
        }
        if self.request_policy == LocalApiRequestPolicy::MergePolicyReadOnly
            && !self.merge_policy_request_is_allowed(path, method, body)
        {
            return Err(ApiReadError::Forbidden);
        }
        if self.request_policy == LocalApiRequestPolicy::MergeAuthority
            && !self.merge_authority_request_is_allowed(path, method, body)
        {
            return Err(ApiReadError::Forbidden);
        }
        if !path.starts_with('/') || path.starts_with("//") {
            return Err(ApiReadError::Forbidden);
        }
        if !self.request_path_stays_in_configured_repository(path) {
            return Err(ApiReadError::Forbidden);
        }
        let url = format!("{}/api/v1{path}", self.config.origin);
        if !request_url_matches_origin(&url, &self.config.origin) {
            return Err(ApiReadError::Forbidden);
        }
        let token = self
            .credential_provider
            .as_ref()
            .map(|provider| provider.token_for(&self.config.origin))
            .transpose()?
            .flatten();
        let (command, stdin_input) = build_curl_api_request(
            &self.config.origin,
            &url,
            method,
            body,
            timeout,
            token.as_ref(),
            true,
        )?;
        let output = run_command_bounded_with_input(
            command,
            stdin_input.as_deref(),
            MAX_RESPONSE_BYTES + MAX_CURL_HEADER_JSON_BYTES + HTTP_STATUS_TRAILER_BYTES,
            self.deadline,
        )?;
        parse_curl_api_output(&output)
    }

    fn reconciliation_request_is_allowed(
        &self,
        path: &str,
        method: &str,
        body: Option<&[u8]>,
    ) -> bool {
        if method != "GET" || body.is_some() {
            return false;
        }
        if self.commit_status_path_is_valid(path) {
            return true;
        }
        if self.workflow_run_path_is_valid(path) {
            return true;
        }
        if self.workflow_job_path_is_valid(path) {
            return true;
        }
        let segments = path.split('/').skip(1).collect::<Vec<_>>();
        match segments.as_slice() {
            ["repos", owner, repository, "issues" | "pulls", number]
                if *owner == self.config.owner && *repository == self.config.repository =>
            {
                number
                    .parse::<u64>()
                    .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == *number)
            }
            ["repos", owner, repository, "issues", "comments", comment_id]
                if *owner == self.config.owner && *repository == self.config.repository =>
            {
                comment_id
                    .parse::<u64>()
                    .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == *comment_id)
            }
            ["repos", owner, repository, "branches", encoded_branch]
                if *owner == self.config.owner && *repository == self.config.repository =>
            {
                self.encoded_branch_path_is_valid(path) && !encoded_branch.is_empty()
            }
            ["repos", owner, repository, "releases", release_id]
                if *owner == self.config.owner && *repository == self.config.repository =>
            {
                release_id
                    .parse::<u64>()
                    .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == *release_id)
            }
            _ => false,
        }
    }

    fn commit_status_path_is_valid(&self, path: &str) -> bool {
        let Some((route, query)) = path.split_once('?') else {
            return false;
        };
        let Some(page) = query
            .strip_prefix(&format!("limit={COMMIT_STATUS_PAGE_SIZE}&page="))
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return false;
        };
        if path.contains('#') || !(1..=MAX_COMMIT_STATUS_PAGES).contains(&page) {
            return false;
        }
        if query != format!("limit={COMMIT_STATUS_PAGE_SIZE}&page={page}") {
            return false;
        }
        let segments = route.split('/').skip(1).collect::<Vec<_>>();
        matches!(
            segments.as_slice(),
            ["repos", owner, repository, "commits", sha, "status"]
                if *owner == self.config.owner
                    && *repository == self.config.repository
                    && valid_canonical_commit_status_sha(sha)
        )
    }

    fn workflow_run_path_is_valid(&self, path: &str) -> bool {
        if path.contains('?') || path.contains('#') {
            return false;
        }
        let segments = path.split('/').skip(1).collect::<Vec<_>>();
        matches!(
            segments.as_slice(),
            ["repos", owner, repository, "actions", "runs", run_id]
                if *owner == self.config.owner
                    && *repository == self.config.repository
                    && run_id
                        .parse::<u64>()
                        .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == *run_id)
        )
    }

    fn workflow_job_path_is_valid(&self, path: &str) -> bool {
        if path.contains('?') || path.contains('#') {
            return false;
        }
        let segments = path.split('/').skip(1).collect::<Vec<_>>();
        matches!(
            segments.as_slice(),
            ["repos", owner, repository, "actions", "jobs", job_id]
                if *owner == self.config.owner
                    && *repository == self.config.repository
                    && job_id
                        .parse::<u64>()
                        .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == *job_id)
        )
    }

    fn encoded_branch_path_is_valid(&self, path: &str) -> bool {
        if path.contains('?') {
            return false;
        }
        let segments = path.split('/').skip(1).collect::<Vec<_>>();
        let ["repos", owner, repository, "branches", encoded_branch] = segments.as_slice() else {
            return false;
        };
        if *owner != self.config.owner || *repository != self.config.repository {
            return false;
        }
        let Some(branch) = decode_url_path_segment(encoded_branch) else {
            return false;
        };
        valid_branch(&branch) && encode_url_path_segment(&branch) == *encoded_branch
    }

    fn merge_policy_request_is_allowed(
        &self,
        path: &str,
        method: &str,
        body: Option<&[u8]>,
    ) -> bool {
        method == "GET" && body.is_none() && path == self.repository_api_path("/branch_protections")
    }

    fn merge_authority_request_is_allowed(
        &self,
        path: &str,
        method: &str,
        body: Option<&[u8]>,
    ) -> bool {
        if path.contains('#') {
            return false;
        }
        let (route, query) = path
            .split_once('?')
            .map_or((path, None), |(route, query)| (route, Some(query)));

        if method == "POST" {
            return query.is_none()
                && body.is_some_and(merge_authority_post_body_is_allowed)
                && route
                    .strip_prefix(&self.repository_api_path("/pulls/"))
                    .and_then(|suffix| suffix.strip_suffix(MERGE_REQUEST_PATH_SUFFIX))
                    .is_some_and(is_canonical_positive_decimal);
        }
        if method != "GET" || body.is_some() {
            return false;
        }
        if query.is_none() && route == "/user" {
            return true;
        }
        if query.is_none()
            && let Some(login) = route.strip_prefix("/users/")
        {
            return is_safe_forgejo_login_segment(login);
        }

        let Some(repository_route) = route.strip_prefix(&self.repository_api_path("/")) else {
            return false;
        };
        let segments = repository_route.split('/').collect::<Vec<_>>();
        match segments.as_slice() {
            ["pulls", number] if query.is_none() => is_canonical_positive_decimal(number),
            ["pulls", number, "reviews"] => {
                is_canonical_positive_decimal(number)
                    && query.is_some_and(is_canonical_review_page_query)
            }
            ["branches", branch] if query.is_none() => *branch == MERGE_PREFLIGHT_BASE_BRANCH,
            ["issues", number] if query.is_none() => is_canonical_positive_decimal(number),
            ["git", "commits", commit] if query.is_none() => valid_full_git_object_id(commit),
            ["git", "trees", commit] => {
                valid_full_git_object_id(commit) && query.is_some_and(is_canonical_tree_page_query)
            }
            _ => false,
        }
    }

    /// Only three route shapes are allowed: `/user`, `/users/<login>` and
    /// `/repos/<owner>/<repository>/...` naming this adapter's own repository,
    /// so a request cannot pair one target's origin and credential with
    /// another repository (including repository-by-ID routes). Dot segments,
    /// empty segments and percent escapes are refused because curl or Forgejo
    /// could normalize them elsewhere.
    fn request_path_stays_in_configured_repository(&self, path: &str) -> bool {
        let route = path.split_once('?').map_or(path, |(route, _)| route);
        let segments = route.split('/').skip(1).collect::<Vec<_>>();
        if let ["repos", owner, repository, "branches", _] = segments.as_slice() {
            return *owner == self.config.owner
                && *repository == self.config.repository
                && self.encoded_branch_path_is_valid(path);
        }
        if let ["repos", owner, repository, "actions", "jobs", _] = segments.as_slice() {
            return *owner == self.config.owner
                && *repository == self.config.repository
                && self.workflow_job_path_is_valid(path);
        }
        if route.contains('%') && !self.encoded_branch_path_is_valid(route) {
            return false;
        }
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
        {
            return false;
        }
        match segments.as_slice() {
            ["user"] | ["users", _] => true,
            ["repos", owner, repository, _, ..] => {
                *owner == self.config.owner && *repository == self.config.repository
            }
            _ => false,
        }
    }

    fn repository_api_path(&self, suffix: &str) -> String {
        format!(
            "/repos/{}/{}{suffix}",
            self.config.owner, self.config.repository
        )
    }

    fn read_pull_request_status_binding(
        &self,
        pull_request_number: u64,
    ) -> Result<PullRequestSnapshotV1, ApiReadError> {
        if pull_request_number == 0 {
            return Err(ApiReadError::Forbidden);
        }
        let resource = ResourceRef {
            kind: ResourceKind::PullRequest,
            owner: self.config.owner.clone(),
            repository: self.config.repository.clone(),
            id: ResourceId::Number(pull_request_number),
        };
        let response = self.read_current(&resource)?;
        let snapshot = PullRequestSnapshotV1::from_forgejo_response(
            &self.config.origin,
            &self.config.owner,
            &self.config.repository,
            pull_request_number,
            &response,
        )
        .map_err(|_| ApiReadError::InvalidResponse)?;
        if !valid_canonical_commit_status_sha(&snapshot.base_sha)
            || !valid_canonical_commit_status_sha(&snapshot.head_sha)
            || !matches!(snapshot.state.as_str(), "open" | "closed")
        {
            return Err(ApiReadError::InvalidResponse);
        }
        Ok(snapshot)
    }

    fn read_pull_request_status_snapshot(
        &self,
        pull_request_number: u64,
    ) -> Result<(Value, bool), ApiReadError> {
        if self.authorization_rejected.load(Ordering::Acquire) {
            return Err(ApiReadError::Unauthorized);
        }
        let before = self.read_pull_request_status_binding(pull_request_number)?;
        let (status_snapshot, statuses_complete) =
            self.read_commit_status_snapshot(&before.head_sha)?;
        let after = self.read_pull_request_status_binding(pull_request_number)?;
        let binding_changed_fields = pull_request_status_binding_changed_fields(&before, &after);
        let binding_stable = binding_changed_fields.is_empty();
        let head_stable = before.head_sha == after.head_sha;
        let no_statuses = status_snapshot["observation"] == "no_statuses";
        let observation = if binding_stable {
            status_snapshot["observation"].clone()
        } else {
            json!("inconclusive")
        };
        let reason = if !binding_stable {
            json!("pull_request_changed_during_snapshot")
        } else if no_statuses {
            json!("no_statuses")
        } else if !statuses_complete {
            json!("status_snapshot_incomplete")
        } else {
            Value::Null
        };
        let action = if !binding_stable {
            json!(
                "the pull request binding changed during the read; reread the pull request and status snapshot before treating these statuses as current"
            )
        } else if no_statuses {
            json!(
                "no Forgejo status evidence exists for this PR head; run an independent verifier and do not report verification success"
            )
        } else if !statuses_complete {
            json!(
                "Forgejo did not provide a complete status snapshot; reread the status pages and do not treat this as verification success"
            )
        } else {
            Value::Null
        };
        let snapshot = json!({
            "schema_version": "forgejo-pull-request-status-snapshot-v1",
            "repository": format!("{}/{}", self.config.owner, self.config.repository),
            "pull_request_number": pull_request_number,
            "pull_request_head_before": pull_request_status_binding(&before),
            "pull_request_head_after": pull_request_status_binding(&after),
            "pull_request_head_rechecked_after_statuses": true,
            "pull_request_head_stable_between_reads": head_stable,
            "pull_request_binding_stable_between_reads": binding_stable,
            "pull_request_binding_changed_fields": binding_changed_fields,
            "current_pr_head_checked": head_stable,
            "observation": observation,
            "snapshot_consistency": if head_stable {
                "pull_request_rechecked_before_and_after_sequential_status_reads_non_atomic"
            } else {
                "pull_request_changed_during_status_read"
            },
            "reason": reason,
            "action": action,
            "status_snapshot": status_snapshot,
            "independent_verification": false,
            "merge_authorizing": false,
            "exit_code_zero_means": "read_complete_and_pull_request_binding_stable_at_two_observations_not_verification_passed",
        });
        Ok((snapshot, binding_stable && statuses_complete))
    }

    fn read_commit_status_snapshot(&self, commit_sha: &str) -> Result<(Value, bool), ApiReadError> {
        if !valid_canonical_commit_status_sha(commit_sha) {
            return Err(ApiReadError::Forbidden);
        }
        if self.authorization_rejected.load(Ordering::Acquire) {
            return Err(ApiReadError::Unauthorized);
        }
        let first_response = self.read_commit_status_page(commit_sha, 1)?;
        let (mut snapshot, first_page_complete) = parse_commit_status_snapshot_response(
            commit_sha,
            &self.config.owner,
            &self.config.repository,
            first_response,
        )?;
        if first_page_complete || snapshot["observation"] == "no_statuses" {
            return Ok((snapshot, first_page_complete));
        }

        let Some(expected_total) = snapshot["total_count"].as_u64() else {
            return Ok((snapshot, false));
        };
        let mut statuses = snapshot["statuses"]
            .as_array()
            .cloned()
            .ok_or(ApiReadError::InvalidResponse)?;
        let mut status_pages = vec![statuses.clone()];
        if statuses.len() > COMMIT_STATUS_PAGE_SIZE as usize {
            return Err(ApiReadError::InvalidResponse);
        }
        if expected_total < statuses.len() as u64 {
            return Err(ApiReadError::InconsistentPagination);
        }
        let mut contexts = BTreeSet::new();
        for status in &statuses {
            let context = status
                .get("context")
                .and_then(Value::as_str)
                .ok_or(ApiReadError::InvalidResponse)?;
            if !contexts.insert(context.to_owned()) {
                return Err(ApiReadError::InvalidResponse);
            }
        }
        let mut page_states = snapshot["forgejo_reported_page_states"]
            .as_array()
            .cloned()
            .ok_or(ApiReadError::InvalidResponse)?;
        let mut pages_read = 1;
        let max_pages = MAX_COMMIT_STATUS_PAGES.min(
            expected_total
                .saturating_add(COMMIT_STATUS_PAGE_SIZE - 1)
                .checked_div(COMMIT_STATUS_PAGE_SIZE)
                .unwrap_or(MAX_COMMIT_STATUS_PAGES),
        );

        while (statuses.len() as u64) < expected_total && pages_read < max_pages {
            let page_number = pages_read + 1;
            let response = self.read_commit_status_page(commit_sha, page_number)?;
            let (page_snapshot, _) = parse_commit_status_snapshot_response(
                commit_sha,
                &self.config.owner,
                &self.config.repository,
                response,
            )?;
            if page_snapshot["observation"] == "no_statuses"
                || page_snapshot["total_count"].as_u64() != Some(expected_total)
            {
                return Err(ApiReadError::InconsistentPagination);
            }
            let page_statuses = page_snapshot["statuses"]
                .as_array()
                .ok_or(ApiReadError::InvalidResponse)?;
            if page_statuses.is_empty() || page_statuses.len() > COMMIT_STATUS_PAGE_SIZE as usize {
                return Err(ApiReadError::InconsistentPagination);
            }
            for status in page_statuses {
                let context = status
                    .get("context")
                    .and_then(Value::as_str)
                    .ok_or(ApiReadError::InvalidResponse)?;
                if !contexts.insert(context.to_owned()) {
                    return Err(ApiReadError::InconsistentPagination);
                }
            }
            status_pages.push(page_statuses.clone());
            statuses.extend(page_statuses.iter().cloned());
            if statuses.len() as u64 > expected_total {
                return Err(ApiReadError::InconsistentPagination);
            }
            page_states.push(
                page_snapshot["forgejo_reported_combined_state"]
                    .as_str()
                    .map(|state| json!(state))
                    .ok_or(ApiReadError::InvalidResponse)?,
            );
            pages_read = page_number;
        }

        let complete = statuses.len() as u64 == expected_total;
        if complete && pages_read > 1 {
            for page_number in 1..=pages_read {
                let response = self.read_commit_status_page(commit_sha, page_number as u64)?;
                let (page_snapshot, _) = parse_commit_status_snapshot_response(
                    commit_sha,
                    &self.config.owner,
                    &self.config.repository,
                    response,
                )?;
                let page_index = (page_number - 1) as usize;
                let expected_statuses = status_pages
                    .get(page_index)
                    .ok_or(ApiReadError::InconsistentPagination)?;
                let expected_state = page_states
                    .get(page_index)
                    .ok_or(ApiReadError::InconsistentPagination)?;
                let observed_statuses = page_snapshot["statuses"]
                    .as_array()
                    .ok_or(ApiReadError::InvalidResponse)?;
                if page_snapshot["observation"] == "no_statuses"
                    || page_snapshot["total_count"].as_u64() != Some(expected_total)
                    || observed_statuses != expected_statuses
                    || page_snapshot["forgejo_reported_combined_state"] != *expected_state
                {
                    return Err(ApiReadError::InconsistentPagination);
                }
            }
        }
        snapshot["observation"] = json!(if complete { "complete" } else { "inconclusive" });
        snapshot["forgejo_reported_combined_state"] = if pages_read == 1 {
            snapshot["forgejo_reported_combined_state"].clone()
        } else {
            Value::Null
        };
        snapshot["forgejo_reported_page_states"] = json!(page_states);
        snapshot["status_count"] = json!(statuses.len());
        snapshot["total_count"] = json!(expected_total);
        snapshot["pages_read"] = json!(pages_read);
        snapshot["pagination_complete"] = json!(complete);
        snapshot["snapshot_consistency"] = json!(if pages_read == 1 {
            "single_response"
        } else {
            "sequential_non_atomic"
        });
        snapshot["statuses"] = json!(statuses);
        Ok((snapshot, complete))
    }

    fn read_commit_status_page(
        &self,
        commit_sha: &str,
        page: u64,
    ) -> Result<ForgejoHttpResponse, ApiReadError> {
        if !(1..=MAX_COMMIT_STATUS_PAGES).contains(&page) {
            return Err(ApiReadError::Forbidden);
        }
        if self.authorization_rejected.load(Ordering::Acquire) {
            return Err(ApiReadError::Unauthorized);
        }
        let path = format!(
            "{}/commits/{commit_sha}/status?limit={COMMIT_STATUS_PAGE_SIZE}&page={page}",
            self.repository_api_path("")
        );
        for attempt in 0..self.retry_policy.max_attempts {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining < Duration::from_millis(1) {
                return Err(ApiReadError::Unavailable);
            }
            let timeout = self.retry_policy.request_timeout.min(remaining);
            self.upstream_read_attempts.fetch_add(1, Ordering::Relaxed);
            let response = self.request_api_once(&path, "GET", None, timeout);
            if Instant::now() >= self.deadline {
                return Err(ApiReadError::Unavailable);
            }
            let response = match response {
                Ok(response) if response.status == 200 => return Ok(response),
                Ok(response) if response.status == 401 => {
                    self.authorization_rejected.store(true, Ordering::Release);
                    Err(ForgejoReadError::from(ApiReadError::Unauthorized))
                }
                Ok(response) if response.status == 403 => {
                    Err(ForgejoReadError::from(ApiReadError::Forbidden))
                }
                Ok(response) if response.status == 404 => {
                    Err(ForgejoReadError::from(ApiReadError::NotFound))
                }
                Ok(response) => Err(ForgejoReadError {
                    error: ApiReadError::HttpStatus(response.status),
                    retry_after: response.retry_after,
                }),
                Err(error) => Err(ForgejoReadError::from(error)),
            };
            match response {
                Ok(response) => return Ok(response),
                Err(failure) if transient_api_read_error(&failure.error) => {
                    if attempt + 1 >= self.retry_policy.max_attempts {
                        return Err(failure.as_api_read_error());
                    }
                    let delay = match failure.retry_after {
                        Some(RetryAfterValue::Delay(delay)) => delay,
                        Some(RetryAfterValue::Invalid) => {
                            return Err(failure.as_api_read_error());
                        }
                        None => self
                            .retry_policy
                            .backoff
                            .get(attempt)
                            .copied()
                            .unwrap_or_default(),
                    };
                    let remaining = self.deadline.saturating_duration_since(Instant::now());
                    if delay >= remaining {
                        return Err(failure.as_api_read_error());
                    }
                    std::thread::sleep(delay);
                }
                Err(failure) => return Err(failure.error),
            }
        }
        Err(ApiReadError::Unavailable)
    }

    fn read_current_once(&self, url: &str, timeout: Duration) -> Result<Value, ForgejoReadError> {
        if !request_url_matches_origin(url, &self.config.origin) {
            return Err(ApiReadError::Forbidden.into());
        }
        let path = url
            .strip_prefix(&format!("{}/api/v1", self.config.origin))
            .ok_or(ApiReadError::Forbidden)?;
        let response = self.request_api_once(path, "GET", None, timeout)?;
        match response.status {
            200 => response.body.ok_or(ApiReadError::InvalidResponse.into()),
            401 => Err(ApiReadError::Unauthorized.into()),
            403 => Err(ApiReadError::Forbidden.into()),
            404 => Err(ApiReadError::NotFound.into()),
            status => Err(ForgejoReadError {
                error: ApiReadError::HttpStatus(status),
                retry_after: response.retry_after,
            }),
        }
    }
}

impl ForgejoCurrentState for LocalLoopbackForgejoApi {
    fn read_current(&self, resource: &ResourceRef) -> Result<Value, ApiReadError> {
        if resource.owner != self.config.owner || resource.repository != self.config.repository {
            return Err(ApiReadError::Forbidden);
        }
        // This API client and its optional host credential have one configured
        // origin/repository scope. Once that endpoint has rejected the client
        // with 401, do not send the same credential again for sibling resources
        // in this invocation; keep each result inconclusive and let a new run
        // retry after the host credential or repository policy changes.
        if self.authorization_rejected.load(Ordering::Acquire) {
            return Err(ApiReadError::Unauthorized);
        }
        let url = match (&resource.kind, &resource.id) {
            (ResourceKind::Issue, ResourceId::Number(number)) => format!(
                "{}/api/v1/repos/{}/{}/issues/{number}",
                self.config.origin, self.config.owner, self.config.repository
            ),
            (
                ResourceKind::IssueComment | ResourceKind::PullRequestComment,
                ResourceId::IssueComment { comment_id, .. },
            ) => format!(
                "{}/api/v1/repos/{}/{}/issues/comments/{comment_id}",
                self.config.origin, self.config.owner, self.config.repository
            ),
            (ResourceKind::PullRequest, ResourceId::Number(number)) => format!(
                "{}/api/v1/repos/{}/{}/pulls/{number}",
                self.config.origin, self.config.owner, self.config.repository
            ),
            (ResourceKind::Branch, ResourceId::Branch(branch)) if valid_branch(branch) => format!(
                "{}/api/v1/repos/{}/{}/branches/{}",
                self.config.origin,
                self.config.owner,
                self.config.repository,
                encode_url_path_segment(branch)
            ),
            (ResourceKind::WorkflowRun, ResourceId::WorkflowRun { run_id, .. }) if *run_id > 0 => {
                format!(
                    "{}/api/v1/repos/{}/{}/actions/runs/{run_id}",
                    self.config.origin, self.config.owner, self.config.repository
                )
            }
            (ResourceKind::WorkflowJob, ResourceId::WorkflowJob { job_id, .. }) if *job_id > 0 => {
                format!(
                    "{}/api/v1/repos/{}/{}/actions/jobs/{job_id}",
                    self.config.origin, self.config.owner, self.config.repository
                )
            }
            (ResourceKind::Release, ResourceId::Release(release_id)) if *release_id > 0 => {
                format!(
                    "{}/api/v1/repos/{}/{}/releases/{release_id}",
                    self.config.origin, self.config.owner, self.config.repository
                )
            }
            _ => return Err(ApiReadError::Forbidden),
        };
        for attempt in 0..self.retry_policy.max_attempts {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining < Duration::from_millis(1) {
                return Err(ApiReadError::Unavailable);
            }
            let timeout = self.retry_policy.request_timeout.min(remaining);
            self.upstream_read_attempts.fetch_add(1, Ordering::Relaxed);
            let response = self.read_current_once(&url, timeout);
            if Instant::now() >= self.deadline {
                return Err(ApiReadError::Unavailable);
            }

            match response {
                Ok(current) => return Ok(current),
                Err(failure) if transient_api_read_error(&failure.error) => {
                    if attempt + 1 >= self.retry_policy.max_attempts {
                        return Err(failure.as_api_read_error());
                    }
                    let delay = match failure.retry_after {
                        Some(RetryAfterValue::Delay(delay)) => delay,
                        Some(RetryAfterValue::Invalid) => {
                            return Err(failure.as_api_read_error());
                        }
                        None => self
                            .retry_policy
                            .backoff
                            .get(attempt)
                            .copied()
                            .unwrap_or_default(),
                    };
                    let remaining = self.deadline.saturating_duration_since(Instant::now());
                    if delay >= remaining {
                        return Err(failure.as_api_read_error());
                    }
                    std::thread::sleep(delay);
                }
                Err(failure) => {
                    if failure.error == ApiReadError::Unauthorized {
                        self.authorization_rejected.store(true, Ordering::Release);
                    }
                    return Err(failure.error);
                }
            }
        }
        Err(ApiReadError::Unavailable)
    }

    fn expected_html_url(&self, resource: &ResourceRef) -> String {
        match (&resource.kind, &resource.id) {
            (ResourceKind::Issue, ResourceId::Number(number)) => format!(
                "{}/{}/{}/issues/{number}",
                self.config.origin, self.config.owner, self.config.repository
            ),
            (ResourceKind::IssueComment, ResourceId::IssueComment { issue_number, .. }) => format!(
                "{}/{}/{}/issues/{issue_number}",
                self.config.origin, self.config.owner, self.config.repository
            ),
            (ResourceKind::PullRequestComment, ResourceId::IssueComment { issue_number, .. }) => {
                format!(
                    "{}/{}/{}/pulls/{issue_number}",
                    self.config.origin, self.config.owner, self.config.repository
                )
            }
            (ResourceKind::PullRequest, ResourceId::Number(number)) => format!(
                "{}/{}/{}/pulls/{number}",
                self.config.origin, self.config.owner, self.config.repository
            ),
            (ResourceKind::Branch, ResourceId::Branch(branch)) if valid_branch(branch) => format!(
                "{}/{}/{}/src/branch/{}",
                self.config.origin,
                self.config.owner,
                self.config.repository,
                encode_url_path_segment(branch)
            ),
            (ResourceKind::WorkflowRun, ResourceId::WorkflowRun { index_in_repo, .. })
                if *index_in_repo > 0 =>
            {
                format!(
                    "{}/{}/{}/actions/runs/{index_in_repo}",
                    self.config.origin, self.config.owner, self.config.repository
                )
            }
            (ResourceKind::WorkflowJob, ResourceId::WorkflowJob { index_in_repo, .. })
                if *index_in_repo > 0 =>
            {
                format!(
                    "{}/{}/{}/actions/runs/{index_in_repo}",
                    self.config.origin, self.config.owner, self.config.repository
                )
            }
            (ResourceKind::Release, ResourceId::Release(release_id)) if *release_id > 0 => {
                format!(
                    "{}/{}/{}/releases",
                    self.config.origin, self.config.owner, self.config.repository
                )
            }
            _ => String::new(),
        }
    }
}

fn decode_url_path_segment(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            decoded.push((hex_value(high)? << 4) | hex_value(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn transient_api_read_error(error: &ApiReadError) -> bool {
    match error {
        ApiReadError::Unavailable => true,
        ApiReadError::HttpStatus(status)
        | ApiReadError::HttpStatusWithRetryAfter { status, .. }
        | ApiReadError::HttpStatusWithInvalidRetryAfter { status } => {
            matches!(*status, 408 | 429 | 500..=599)
        }
        ApiReadError::Unauthorized
        | ApiReadError::Forbidden
        | ApiReadError::NotFound
        | ApiReadError::InvalidCredential
        | ApiReadError::BatchCacheBudgetExceeded { .. }
        | ApiReadError::InconsistentPagination
        | ApiReadError::ResponseTooLarge { .. }
        | ApiReadError::InvalidResponse => false,
    }
}

fn parse_curl_api_output(output: &[u8]) -> Result<ForgejoHttpResponse, ApiReadError> {
    let end_marker = output
        .windows(CURL_END_MARKER.len())
        .rposition(|window| window == CURL_END_MARKER)
        .ok_or(ApiReadError::InvalidResponse)?;
    if end_marker + CURL_END_MARKER.len() != output.len() {
        return Err(ApiReadError::InvalidResponse);
    }
    let headers_marker = output[..end_marker]
        .windows(CURL_HEADERS_MARKER.len())
        .rposition(|window| window == CURL_HEADERS_MARKER)
        .ok_or(ApiReadError::InvalidResponse)?;
    let status_marker = output[..headers_marker]
        .windows(CURL_STATUS_MARKER.len())
        .rposition(|window| window == CURL_STATUS_MARKER)
        .ok_or(ApiReadError::InvalidResponse)?;
    let status_bytes = &output[status_marker + CURL_STATUS_MARKER.len()..headers_marker];
    let status = std::str::from_utf8(status_bytes)
        .map_err(|_| ApiReadError::InvalidResponse)?
        .parse::<u16>()
        .map_err(|_| ApiReadError::InvalidResponse)?;
    let body_bytes = &output[..status_marker];
    if body_bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ApiReadError::ResponseTooLarge {
            component: "body",
            maximum_bytes: MAX_RESPONSE_BYTES,
        });
    }
    let headers_bytes = &output[headers_marker + CURL_HEADERS_MARKER.len()..end_marker];
    if headers_bytes.len() > MAX_CURL_HEADER_JSON_BYTES {
        return Err(ApiReadError::ResponseTooLarge {
            component: "header_json",
            maximum_bytes: MAX_CURL_HEADER_JSON_BYTES,
        });
    }
    let headers =
        agent_forge_core::strict_json::parse_value_rejecting_duplicate_members(headers_bytes)
            .map_err(|_| ApiReadError::InvalidResponse)?;

    let total_count = if status == 200 {
        single_curl_header_value(&headers, "x-total-count")?
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| ApiReadError::InvalidResponse)
            })
            .transpose()?
    } else {
        None
    };
    let retry_after = if status != 200 {
        match single_curl_header_value(&headers, "retry-after") {
            Ok(Some(value)) => parse_retry_after(value.as_bytes()),
            Ok(None) => None,
            Err(_) => Some(RetryAfterValue::Invalid),
        }
    } else {
        None
    };
    let body = if status == 200 && !body_bytes.is_empty() {
        Some(
            agent_forge_core::strict_json::parse_value_rejecting_duplicate_members(body_bytes)
                .map_err(|_| ApiReadError::InvalidResponse)?,
        )
    } else {
        None
    };

    Ok(ForgejoHttpResponse {
        status,
        total_count,
        retry_after,
        body,
    })
}

fn single_curl_header_value<'a>(
    headers: &'a Value,
    expected_name: &str,
) -> Result<Option<&'a str>, ApiReadError> {
    let object = headers.as_object().ok_or(ApiReadError::InvalidResponse)?;
    let mut matching = object
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(expected_name));
    let Some((_, value)) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() {
        return Err(ApiReadError::InvalidResponse);
    }
    let values = value.as_array().ok_or(ApiReadError::InvalidResponse)?;
    if values.len() != 1 {
        return Err(ApiReadError::InvalidResponse);
    }
    values[0]
        .as_str()
        .map(Some)
        .ok_or(ApiReadError::InvalidResponse)
}

fn parse_retry_after(value: &[u8]) -> Option<RetryAfterValue> {
    if value.is_empty() {
        return None;
    }
    let Ok(value) = std::str::from_utf8(value) else {
        return Some(RetryAfterValue::Invalid);
    };
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(
            value
                .parse::<u64>()
                .map(Duration::from_secs)
                .map(RetryAfterValue::Delay)
                .unwrap_or(RetryAfterValue::Invalid),
        );
    }

    let Some(target_epoch_seconds) = parse_http_date_epoch_seconds(value) else {
        return Some(RetryAfterValue::Invalid);
    };
    let now_epoch_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64;
    Some(RetryAfterValue::Delay(Duration::from_secs(
        target_epoch_seconds
            .saturating_sub(now_epoch_seconds)
            .max(0) as u64,
    )))
}

fn parse_http_date_epoch_seconds(value: &str) -> Option<i64> {
    let fields = value.split_ascii_whitespace().collect::<Vec<_>>();
    match fields.as_slice() {
        [weekday, day, month, year, time, "GMT"] => {
            let weekday = parse_weekday(weekday.strip_suffix(',')?, false)?;
            epoch_seconds_for_date(
                weekday,
                parse_fixed_decimal(day, 2)?,
                parse_month(month)?,
                parse_fixed_decimal(year, 4)?,
                time,
            )
        }
        [weekday, date, time, "GMT"] => {
            let weekday = parse_weekday(weekday.strip_suffix(',')?, true)?;
            let mut date = date.split('-');
            let day = parse_fixed_decimal(date.next()?, 2)?;
            let month = parse_month(date.next()?)?;
            let short_year = parse_fixed_decimal(date.next()?, 2)?;
            if date.next().is_some() {
                return None;
            }
            let now = current_epoch_seconds();
            let current_year = civil_from_days(now.div_euclid(86_400)).0;
            let mut year = current_year - current_year.rem_euclid(100) + short_year;
            let (hour, minute, second) = parse_time_of_day(time)?;
            if rfc850_date_is_more_than_fifty_years_ahead(
                now, year, month, day, hour, minute, second,
            ) {
                year -= 100;
            }
            epoch_seconds_for_parts(weekday, day, month, year, hour, minute, second)
        }
        [weekday, month, day, time, year] => {
            let weekday = parse_weekday(weekday, false)?;
            let day = parse_asctime_day(day)?;
            epoch_seconds_for_date(
                weekday,
                day,
                parse_month(month)?,
                parse_fixed_decimal(year, 4)?,
                time,
            )
        }
        _ => None,
    }
}

fn parse_weekday(value: &str, long: bool) -> Option<i64> {
    match (long, value) {
        (false, "Sun") | (true, "Sunday") => Some(0),
        (false, "Mon") | (true, "Monday") => Some(1),
        (false, "Tue") | (true, "Tuesday") => Some(2),
        (false, "Wed") | (true, "Wednesday") => Some(3),
        (false, "Thu") | (true, "Thursday") => Some(4),
        (false, "Fri") | (true, "Friday") => Some(5),
        (false, "Sat") | (true, "Saturday") => Some(6),
        _ => None,
    }
}

fn parse_month(value: &str) -> Option<i64> {
    match value {
        "Jan" => Some(1),
        "Feb" => Some(2),
        "Mar" => Some(3),
        "Apr" => Some(4),
        "May" => Some(5),
        "Jun" => Some(6),
        "Jul" => Some(7),
        "Aug" => Some(8),
        "Sep" => Some(9),
        "Oct" => Some(10),
        "Nov" => Some(11),
        "Dec" => Some(12),
        _ => None,
    }
}

fn parse_asctime_day(value: &str) -> Option<i64> {
    if !(1..=2).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_time_of_day(value: &str) -> Option<(i64, i64, i64)> {
    let time = value.split(':').collect::<Vec<_>>();
    if time.len() != 3 {
        return None;
    }
    let hour = parse_fixed_decimal(time[0], 2)?;
    let minute = parse_fixed_decimal(time[1], 2)?;
    let second = parse_fixed_decimal(time[2], 2)?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((hour, minute, second))
}

fn epoch_seconds_for_date(
    weekday: i64,
    day: i64,
    month: i64,
    year: i64,
    time: &str,
) -> Option<i64> {
    let (hour, minute, second) = parse_time_of_day(time)?;
    epoch_seconds_for_parts(weekday, day, month, year, hour, minute, second)
}

fn epoch_seconds_for_parts(
    weekday: i64,
    day: i64,
    month: i64,
    year: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> Option<i64> {
    if !(1601..=9999).contains(&year) {
        return None;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > last_day {
        return None;
    }
    let days_since_epoch = days_from_civil(year, month, day);
    if (days_since_epoch + 4).rem_euclid(7) != weekday {
        return None;
    }
    days_since_epoch
        .checked_mul(86_400)?
        .checked_add(hour * 3_600)?
        .checked_add(minute * 60)?
        .checked_add(second)
}

fn current_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn rfc850_date_is_more_than_fifty_years_ahead(
    now: i64,
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> bool {
    let (current_year, current_month, current_day) = civil_from_days(now.div_euclid(86_400));
    let current_seconds_of_day = now.rem_euclid(86_400);
    let current_hour = current_seconds_of_day / 3_600;
    let current_minute = (current_seconds_of_day % 3_600) / 60;
    let current_second = current_seconds_of_day % 60;
    let candidate_parts = (year, month, day, hour, minute, second);
    let cutoff_parts = (
        current_year + 50,
        current_month,
        current_day.min(last_day_of_month(current_year + 50, current_month)),
        current_hour,
        current_minute,
        current_second,
    );
    candidate_parts > cutoff_parts
}

fn last_day_of_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted_days = days + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn parse_fixed_decimal(value: &str, width: usize) -> Option<i64> {
    if value.len() != width || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn reconciliation_error_value(error: &ReconcileError) -> Value {
    let (code, retryable, action, details) = match error {
        ReconcileError::BatchTooLarge => (
            "batch_too_large",
            false,
            "split the request into batches of at most 100 delivery IDs",
            json!({"maximum_events": MAX_RECONCILE_BATCH_EVENTS}),
        ),
        ReconcileError::BatchOutputBudgetExceeded {
            maximum_output_bytes,
        } => (
            "batch_result_budget_exceeded",
            false,
            "split the delivery IDs into smaller batches; do not retry the unchanged batch",
            json!({
                "maximum_projected_output_bytes": maximum_output_bytes,
                "measurement": "serialized_canonical_state_json_plus_2048_bytes_per_delivery_for_cli_envelope"
            }),
        ),
        ReconcileError::InvalidEnvelope => (
            "invalid_envelope",
            false,
            "use a verified inbox delivery and do not edit the journal directly",
            json!({}),
        ),
        ReconcileError::UnexpectedEventPayloadField(field) => (
            "invalid_envelope",
            false,
            "inspect the event payload field against the event contract; preserve the append-only journal and do not edit it directly",
            json!({"field_path":format!("payload.{field}")}),
        ),
        ReconcileError::InvalidEventPayloadField(field) => (
            "invalid_envelope",
            false,
            "preserve the persisted row; compare its payload field with trusted sender evidence if available; the inbox does not retain the HMAC, so do not treat replay as signature re-verification",
            json!({
                "field_path":format!("payload.{field}"),
                "persistence_state":"already_persisted",
                "source_hmac_persisted":false,
                "source_hmac_reverified":false,
            }),
        ),
        ReconcileError::UnsupportedEventPayloadField(field) => (
            "unsupported_event",
            false,
            "this adapter accepts branch refs only; preserve the persisted event, consult --capabilities, and add explicit tag support before enabling tag hooks",
            json!({
                "field_path":format!("payload.{field}"),
                "event_contracts_capability": "operations[reconcile_webhook_deliveries].event_contracts",
            }),
        ),
        ReconcileError::UnsupportedEvent => (
            "unsupported_event",
            false,
            "consult --capabilities for the matching event contract; preserve the journal and correct the event source",
            json!({
                "supported_events": SUPPORTED_EVENTS,
                "event_contracts_capability": "operations[reconcile_webhook_deliveries].event_contracts",
            }),
        ),
        ReconcileError::InvalidRepository => (
            "invalid_repository",
            false,
            "confirm the repository reference against the configured allowlist",
            json!({}),
        ),
        ReconcileError::InvalidNumber => (
            "invalid_resource_number",
            false,
            "provide a positive issue or pull request number",
            json!({"minimum":1}),
        ),
        ReconcileError::InvalidBranch => (
            "invalid_branch",
            false,
            "provide a valid refs/heads branch name and retry with the corrected push delivery",
            json!({"required_ref_prefix":"refs/heads/"}),
        ),
        ReconcileError::Source(ApiReadError::Unavailable) => (
            "upstream_unavailable",
            true,
            "restore local Forgejo availability, then retry within the caller's retry budget",
            json!({"upstream":"forgejo"}),
        ),
        ReconcileError::Source(ApiReadError::Unauthorized) => (
            "upstream_unauthorized",
            false,
            "verify the host-scoped Forgejo API credential is present, current, and accepted for the configured repository; keep its value outside agent input",
            json!({"upstream":"forgejo"}),
        ),
        ReconcileError::Source(ApiReadError::Forbidden) => (
            "upstream_forbidden",
            false,
            "inspect repository visibility and local API permissions",
            json!({"upstream":"forgejo"}),
        ),
        ReconcileError::Source(ApiReadError::NotFound) => (
            "resource_not_found",
            false,
            "confirm that the resource still exists and that the host-scoped Forgejo credential can read it; a private resource may be masked as HTTP 404, so keep the credential outside agent input",
            json!({"upstream":"forgejo"}),
        ),
        ReconcileError::ParentWorkflowRunNotFound {
            run_id,
            index_in_repo,
        } => (
            "resource_not_found",
            false,
            "the workflow job was readable, but its parent workflow run returned HTTP 404; verify the parent run ID and repository access because a private run may be masked as missing",
            json!({
                "upstream":"forgejo",
                "readback_resource": {
                    "kind":"workflow_run",
                    "run_id":run_id,
                    "index_in_repo":index_in_repo,
                },
            }),
        ),
        ReconcileError::Source(ApiReadError::InvalidCredential) => (
            "credential_source_invalid",
            false,
            "inspect the host credential provider; do not move credentials into agent input",
            json!({"upstream":"forgejo"}),
        ),
        ReconcileError::Source(ApiReadError::HttpStatus(status)) => (
            "upstream_http_status",
            matches!(*status, 408 | 429) || (500..=599).contains(status),
            "inspect the Forgejo HTTP status; retry only when retryable is true",
            json!({"http_status":status}),
        ),
        ReconcileError::Source(ApiReadError::HttpStatusWithRetryAfter {
            status,
            retry_after_ms,
        }) => (
            "upstream_http_status",
            matches!(*status, 408 | 429) || (500..=599).contains(status),
            "wait at least details.retry_after_ms before retrying; respect the caller's retry budget",
            json!({"http_status":status,"retry_after_ms":retry_after_ms}),
        ),
        ReconcileError::Source(ApiReadError::HttpStatusWithInvalidRetryAfter { status }) => (
            "upstream_http_status",
            matches!(*status, 408 | 429) || (500..=599).contains(status),
            "the upstream Retry-After is invalid or ambiguous; do not retry automatically from it, and retry only under the caller's bounded policy after a fresh read",
            json!({"http_status":status,"retry_after_disposition":"invalid_or_ambiguous"}),
        ),
        ReconcileError::Source(ApiReadError::ResponseTooLarge {
            component,
            maximum_bytes,
        }) => (
            "upstream_response_too_large",
            false,
            "the Forgejo response exceeded a local byte limit; reduce the requested resource or correct the upstream response, then reread instead of retrying unchanged",
            json!({"upstream":"forgejo","component":component,"maximum_bytes":maximum_bytes}),
        ),
        ReconcileError::Source(ApiReadError::InvalidResponse) => (
            "upstream_invalid_response",
            false,
            "inspect the Forgejo response size and JSON before retrying",
            json!({"upstream":"forgejo"}),
        ),
        ReconcileError::Source(ApiReadError::InconsistentPagination) => (
            "upstream_pagination_inconsistent",
            true,
            "discard the partial snapshot and reread the complete Forgejo response before retrying",
            json!({"upstream":"forgejo"}),
        ),
        ReconcileError::Source(ApiReadError::BatchCacheBudgetExceeded {
            maximum_cached_json_bytes,
        }) => (
            "batch_cache_budget_exceeded",
            false,
            "split the delivery IDs into smaller batches; do not retry the unchanged batch",
            json!({
                "maximum_cached_json_bytes": maximum_cached_json_bytes,
                "measurement": "combined_uncompressed_serialized_json"
            }),
        ),
        ReconcileError::ResourceMismatch => (
            "resource_mismatch",
            false,
            "verify that the API resource identity, repository and canonical URL match the inbox reference",
            json!({}),
        ),
        ReconcileError::InvalidCurrentState => (
            "invalid_current_state",
            false,
            "inspect the API snapshot and require valid state, timestamp, URL and PR commit SHAs",
            json!({}),
        ),
        ReconcileError::InvalidCurrentStateField(field) => (
            "invalid_current_state",
            false,
            "inspect the named field in the Forgejo API snapshot and retry only after the source is corrected",
            json!({"field":field}),
        ),
        ReconcileError::DeliveryQuarantined {
            source_sequence,
            source_event_hash,
            disposition_sequence,
        } => (
            "delivery_quarantined",
            false,
            "keep this delivery inconclusive; compare the persisted payload with trusted sender evidence if available before deciding how to proceed",
            json!({
                "source_sequence": source_sequence,
                "source_event_hash": source_event_hash,
                "disposition_sequence": disposition_sequence,
                "canonical_state_created": false,
                "source_hmac_persisted": false,
                "source_hmac_reverified": false,
                "actor_identity_recorded": false,
            }),
        ),
    };

    json!({
        "code": code,
        "retryable": retryable,
        "action": action,
        "details": details,
    })
}

fn attach_journal_source_identity(error: &mut Value, event: &JournalEvent) {
    if let Some(details) = error.get_mut("details").and_then(Value::as_object_mut) {
        details.insert(
            "journal_source_sequence".to_owned(),
            Value::from(event.sequence),
        );
        details.insert(
            "journal_source_hash".to_owned(),
            Value::String(event.hash.clone()),
        );
    }
}

fn attach_journal_source_observation(
    result: &mut Value,
    event: &JournalEvent,
    attestation_verified: bool,
    quarantined: bool,
) {
    let observation = journal_source_observation(event, attestation_verified, quarantined);
    if let Some(object) = result.as_object_mut() {
        object.insert("journal_source".to_owned(), observation);
    }
}

fn journal_source_observation(
    event: &JournalEvent,
    attestation_verified: bool,
    quarantined: bool,
) -> Value {
    let attestation_status = if quarantined {
        "quarantined_not_checked"
    } else if attestation_verified {
        "verified"
    } else if event.receiver_attestation.is_some() {
        "present_unverified"
    } else {
        "missing"
    };
    let attestation_algorithm = attestation_verified
        .then(|| {
            event
                .receiver_attestation
                .as_ref()
                .map(|value| value.algorithm.as_str())
        })
        .flatten();
    let key_id = attestation_verified
        .then(|| {
            event
                .receiver_attestation
                .as_ref()
                .map(|value| value.key_id.as_str())
        })
        .flatten();
    json!({
        "sequence": event.sequence,
        "event_sha256": event.hash,
        "receiver_attestation": {
            "status": attestation_status,
            "verification_scope": if attestation_verified { "receiver_signature_only" } else { "not_verified" },
            "algorithm": attestation_algorithm,
            "key_id_sha256": key_id,
            "signature_included": false,
        },
    })
}

fn selected_event_source_observations(
    delivery_ids: &[String],
    events: &[JournalEvent],
    quarantined_delivery_ids: &BTreeSet<String>,
    attestation_verified_delivery_ids: &BTreeSet<String>,
) -> Value {
    assert_eq!(
        delivery_ids.len(),
        events.len(),
        "selected delivery IDs and journal rows must remain positionally bound"
    );
    Value::Array(
        delivery_ids
            .iter()
            .zip(events)
            .map(|(delivery_id, event)| {
                json!({
                    "delivery_id": delivery_id,
                    "journal_source": journal_source_observation(
                        event,
                        attestation_verified_delivery_ids.contains(delivery_id),
                        quarantined_delivery_ids.contains(delivery_id),
                    ),
                })
            })
            .collect(),
    )
}

fn attach_selected_event_sources(
    result: &mut Value,
    delivery_ids: &[String],
    events: &[JournalEvent],
    quarantined_delivery_ids: &BTreeSet<String>,
    attestation_verified_delivery_ids: &BTreeSet<String>,
) {
    if delivery_ids.len() < 2 {
        return;
    }
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "selected_event_sources".to_owned(),
            selected_event_source_observations(
                delivery_ids,
                events,
                quarantined_delivery_ids,
                attestation_verified_delivery_ids,
            ),
        );
    }
}

fn attach_scoped_resource_detail(
    error: &mut Value,
    event: &JournalEvent,
    configured_owner: &str,
    configured_repository: &str,
) {
    if !error.get("details").is_some_and(Value::is_object) {
        return;
    }
    let Ok(resource) = event_resource_ref(event) else {
        return;
    };
    if resource.owner != configured_owner || resource.repository != configured_repository {
        return;
    }

    if error.get("code").and_then(Value::as_str) == Some("resource_not_found") {
        let missing_parent_workflow_run = error
            .get("details")
            .and_then(|details| details.get("readback_resource"))
            .and_then(|resource| resource.get("kind"))
            .and_then(Value::as_str)
            == Some("workflow_run");
        let action = match resource.kind {
            ResourceKind::Issue => {
                "verify the issue number and repository access; HTTP 404 does not distinguish a missing issue from an inaccessible repository"
            }
            ResourceKind::IssueComment => {
                "verify the comment ID, parent issue, and repository access; a deleted comment is absent only after the parent issue is readable"
            }
            ResourceKind::PullRequestComment => {
                "verify the comment ID, parent pull request, and repository access; a deleted comment is absent only after the parent pull request is readable"
            }
            ResourceKind::PullRequest => {
                "verify the pull request number and repository access; HTTP 404 does not distinguish a missing pull request from an inaccessible repository"
            }
            ResourceKind::Branch => {
                "verify the branch name and repository access; HTTP 404 does not distinguish an absent branch from an inaccessible repository"
            }
            ResourceKind::WorkflowRun => {
                "verify the workflow run ID and repository access; HTTP 404 does not distinguish an absent run from an inaccessible repository"
            }
            ResourceKind::WorkflowJob if missing_parent_workflow_run => {
                "the workflow job was readable, but its parent workflow run returned HTTP 404; verify the parent run ID and repository access because a private run may be masked as missing"
            }
            ResourceKind::WorkflowJob => {
                "verify the workflow job ID and repository access; HTTP 404 does not distinguish an absent job from an inaccessible repository"
            }
            ResourceKind::Release => {
                "verify the release ID and repository access; HTTP 404 does not distinguish a deleted release from an inaccessible repository"
            }
        };
        if let Some(action_value) = error.get_mut("action") {
            *action_value = Value::String(action.to_owned());
        }
    }

    let Some(details) = error.get_mut("details").and_then(Value::as_object_mut) else {
        return;
    };
    let resource_value = match (&resource.kind, &resource.id) {
        (ResourceKind::Issue, ResourceId::Number(number)) => json!({
            "kind": "issue",
            "owner": resource.owner,
            "repository": resource.repository,
            "number": number,
        }),
        (
            ResourceKind::IssueComment,
            ResourceId::IssueComment {
                issue_number,
                comment_id,
                ..
            },
        ) => json!({
            "kind": "issue_comment",
            "owner": resource.owner,
            "repository": resource.repository,
            "issue_number": issue_number,
            "comment_id": comment_id,
        }),
        (
            ResourceKind::PullRequestComment,
            ResourceId::IssueComment {
                issue_number,
                comment_id,
                ..
            },
        ) => json!({
            "kind": "pull_request_comment",
            "owner": resource.owner,
            "repository": resource.repository,
            "pull_request_number": issue_number,
            "comment_id": comment_id,
        }),
        (ResourceKind::PullRequest, ResourceId::Number(number)) => json!({
            "kind": "pull_request",
            "owner": resource.owner,
            "repository": resource.repository,
            "number": number,
        }),
        (ResourceKind::Branch, ResourceId::Branch(branch)) => json!({
            "kind": "branch",
            "owner": resource.owner,
            "repository": resource.repository,
            "branch": branch,
        }),
        (
            ResourceKind::WorkflowRun,
            ResourceId::WorkflowRun {
                run_id,
                index_in_repo,
            },
        ) => json!({
            "kind": "workflow_run",
            "owner": resource.owner,
            "repository": resource.repository,
            "run_id": run_id,
            "index_in_repo": index_in_repo,
        }),
        (
            ResourceKind::WorkflowJob,
            ResourceId::WorkflowJob {
                job_id,
                run_id,
                index_in_repo,
                ..
            },
        ) => json!({
            "kind": "workflow_job",
            "owner": resource.owner,
            "repository": resource.repository,
            "job_id": job_id,
            "run_id": run_id,
            "index_in_repo": index_in_repo,
        }),
        (ResourceKind::Release, ResourceId::Release(release_id)) => json!({
            "kind": "release",
            "owner": resource.owner,
            "repository": resource.repository,
            "release_id": release_id,
        }),
        _ => return,
    };
    details.insert("resource".to_owned(), resource_value);
}

fn classify_out_of_scope_delivery_error(
    error: &mut Value,
    event: &JournalEvent,
    configured_owner: &str,
    configured_repository: &str,
) {
    if error.get("code").and_then(Value::as_str) != Some("upstream_forbidden") {
        return;
    }
    let Ok(resource) = event_resource_ref(event) else {
        return;
    };
    if resource.owner == configured_owner && resource.repository == configured_repository {
        return;
    }

    *error = json!({
        "code": "resource_out_of_scope",
        "retryable": false,
        "action": "route this delivery to the matching configured repository adapter; do not retry it against this repository",
        "details": {
            "configured_repository": format!("{configured_owner}/{configured_repository}"),
            "event_repository": format!("{}/{}", resource.owner, resource.repository),
        },
    });
}

/// Describe the boundary between receiver authentication and later journal
/// replay. This is a contract statement, not per-delivery cryptographic proof.
fn source_event_provenance_contract() -> Value {
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
}

fn attestation_refusal_code(error: &JournalAttestationError) -> &'static str {
    match error {
        JournalAttestationError::Missing => "host_journal_attestation_missing",
        JournalAttestationError::Unsupported => "host_journal_attestation_unsupported",
        JournalAttestationError::UntrustedKey => "host_journal_attestation_untrusted_key",
        JournalAttestationError::InvalidSignature => "host_journal_attestation_invalid",
    }
}

fn host_attestation_refusal_failure(
    delivery_ids: &[String],
    failed_delivery_id: Option<&str>,
    code: &str,
    action: &str,
) -> (Value, bool) {
    let mut details = json!({
        "verification_scope": "selected_inbox_delivery_rows",
        "host_credential_read": false,
        "upstream_calls_attempted": false,
        "canonical_state_created": false,
    });
    if let Some(delivery_id) = failed_delivery_id {
        details["delivery_id"] = json!(delivery_id);
    }
    let error = json!({
        "code": code,
        "retryable": false,
        "action": action,
        "details": details,
    });
    if delivery_ids.len() == 1 {
        (
            json!({
                "schema_version": RECONCILE_SINGLE_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "delivery_id": delivery_ids[0],
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    } else {
        (
            json!({
                "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "requested_delivery_count": delivery_ids.len(),
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    }
}

fn host_credential_pipe_read_failure(delivery_ids: &[String], reader_error: &str) -> (Value, bool) {
    let error = json!({
        "code": "host_credential_pipe_read_failed",
        "retryable": false,
        "action": "preserve the selected inbox rows, provide a fresh valid anonymous host credential pipe, and retry; never place the credential in argv, environment, or output",
        "details": {
            "credential_pipe_read_attempted": true,
            "credential_pipe_read_succeeded": false,
            "upstream_calls_attempted": false,
            "canonical_state_created": false,
            "reader_error": reader_error,
        },
    });
    if delivery_ids.len() == 1 {
        (
            json!({
                "schema_version": RECONCILE_SINGLE_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "delivery_id": delivery_ids[0],
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    } else {
        (
            json!({
                "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "requested_delivery_count": delivery_ids.len(),
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    }
}

fn single_reconciliation_result(
    delivery_id: &str,
    result: Result<ReconciledState, ReconcileError>,
    upstream_read_attempts: usize,
) -> (Value, bool) {
    match result {
        Ok(reconciled) => {
            let state = reconciled_state_value(&reconciled);
            (
                json!({
                    "schema_version": RECONCILE_SINGLE_SCHEMA_VERSION,
                    "api_version": FORGEJO_REST_API_VERSION,
                    "source_event_provenance_contract": source_event_provenance_contract(),
                    "verdict": "current_state_read",
                    "delivery_id": reconciled.delivery_id,
                    "upstream_read_attempts": upstream_read_attempts,
                    "resource": state["resource"],
                    "canonical_state": state["canonical_state"],
                }),
                true,
            )
        }
        Err(error) => (
            json!({
                "schema_version": RECONCILE_SINGLE_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "delivery_id": delivery_id,
                "upstream_read_attempts": upstream_read_attempts,
                "error": reconciliation_error_value(&error),
            }),
            false,
        ),
    }
}

fn batch_reconciliation_failure(
    error: &ReconcileError,
    requested_delivery_count: usize,
    upstream_read_attempts: usize,
) -> (Value, bool) {
    (
        json!({
            "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
            "api_version": FORGEJO_REST_API_VERSION,
            "source_event_provenance_contract": source_event_provenance_contract(),
            "verdict": "inconclusive",
            "requested_delivery_count": requested_delivery_count,
            "upstream_read_attempts": upstream_read_attempts,
            "error": reconciliation_error_value(error),
        }),
        false,
    )
}

fn duplicate_delivery_selection_failure(requested_delivery_count: usize) -> (Value, bool) {
    (
        json!({
            "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
            "api_version": FORGEJO_REST_API_VERSION,
            "source_event_provenance_contract": source_event_provenance_contract(),
            "verdict": "inconclusive",
            "requested_delivery_count": requested_delivery_count,
            "upstream_read_attempts": 0,
            "error": {
                "code": "duplicate_delivery_selection",
                "retryable": false,
                "action": "remove duplicate delivery IDs and retry with each accepted inbox delivery listed once",
                "details": {
                    "selection": "delivery_ids_must_be_unique",
                },
            },
        }),
        false,
    )
}

fn invalid_quarantine_disposition_failure(delivery_ids: &[String]) -> (Value, bool) {
    let error = json!({
        "code": "invalid_quarantine_disposition",
        "retryable": false,
        "action": "preserve the journal; inspect every quarantine disposition with a trusted offline tool; restore a verified copy before retrying, and do not edit the hash-chained journal in place",
        "details": {
            "validation_scope": "all_quarantine_dispositions_in_journal",
            "canonical_state_created": false,
            "host_credential_read": false,
            "upstream_calls_attempted": false,
        },
    });
    if delivery_ids.len() == 1 {
        (
            json!({
                "schema_version": RECONCILE_SINGLE_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "delivery_id": delivery_ids[0],
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    } else {
        (
            json!({
                "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "requested_delivery_count": delivery_ids.len(),
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    }
}

fn invalid_webhook_inbox_failure(delivery_ids: &[String]) -> (Value, bool) {
    let error = json!({
        "code": "inbox_journal_invalid",
        "retryable": false,
        "action": "preserve a copy of the inbox; validate or recover it through a trusted offline procedure, then retry only after a complete hash-chain-valid snapshot exists; do not edit the journal in place",
        "details": {
            "validation_scope": "complete_webhook_inbox_snapshot",
            "canonical_state_created": false,
            "host_credential_read": false,
            "upstream_calls_attempted": false,
            "journal_repair_attempted": false,
        },
    });
    if delivery_ids.len() == 1 {
        (
            json!({
                "schema_version": RECONCILE_SINGLE_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "delivery_id": delivery_ids[0],
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    } else {
        (
            json!({
                "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "requested_delivery_count": delivery_ids.len(),
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    }
}

fn delivery_selection_failure(delivery_ids: &[String], missing_id: &str) -> (Value, bool) {
    let error = json!({
        "code": "delivery_not_found",
        "retryable": false,
        "action": "confirm that the delivery ID was accepted into the verified inbox before retrying",
        "details": {
            "delivery_id": missing_id,
            "source": "verified_inbox",
        },
    });
    if delivery_ids.len() == 1 {
        (
            json!({
                "schema_version": RECONCILE_SINGLE_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "delivery_id": delivery_ids[0],
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    } else {
        (
            json!({
                "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "source_event_provenance_contract": source_event_provenance_contract(),
                "verdict": "inconclusive",
                "requested_delivery_count": delivery_ids.len(),
                "upstream_read_attempts": 0,
                "error": error,
            }),
            false,
        )
    }
}

fn read_adapter_input(path: &Path, max_bytes: usize, private: bool) -> Result<Vec<u8>, String> {
    // O_NONBLOCK makes opening a FIFO return instead of waiting for a writer, so
    // the regular-file check below can reject it.
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "input must be a readable regular non-symlink file".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect adapter input".to_owned())?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes as u64 {
        return Err(format!(
            "adapter input must be a regular file no larger than {max_bytes} bytes"
        ));
    }
    if private {
        let effective_uid = unsafe { libc::geteuid() };
        if metadata.uid() != effective_uid
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
            || metadata.mode() & 0o600 != 0o600
        {
            return Err(
                "private adapter input must be owner-owned, singly linked, and mode 0600"
                    .to_owned(),
            );
        }
    }
    read_limited(&mut file, max_bytes)
        .map_err(|_| "adapter input could not be read within its size limit".to_owned())
}

fn merge_preflight_error(error: &MergePreflightError) -> Value {
    use agent_forge_core::GateError;
    let (code, retryable, action, details) = match error {
        MergePreflightError::WrongScope => (
            "wrong_scope",
            false,
            "verify the configured Forgejo origin, repository and base branch",
            json!({}),
        ),
        MergePreflightError::WrongPullRequest => (
            "wrong_pull_request",
            false,
            "verify the requested pull request number against the Forgejo response",
            json!({}),
        ),
        MergePreflightError::InvalidSnapshot => (
            "invalid_snapshot",
            false,
            "inspect the current Forgejo pull request response and require all bound fields",
            json!({}),
        ),
        MergePreflightError::PullRequestNotOpen => (
            "pull_request_not_open",
            false,
            "reopen the pull request or stop this mission",
            json!({}),
        ),
        MergePreflightError::AlreadyMerged => (
            "already_merged",
            false,
            "read back the merge result and create a new mission if more work is needed",
            json!({}),
        ),
        MergePreflightError::DraftPullRequest => (
            "draft_pull_request",
            false,
            "resolve the draft state through the normal review decision",
            json!({}),
        ),
        MergePreflightError::MergeabilityUnknown => (
            "mergeability_unknown",
            true,
            "refresh the pull request after Forgejo recalculates mergeability",
            json!({}),
        ),
        MergePreflightError::NotMergeable => (
            "not_mergeable",
            false,
            "repair the merge conflict, then create a fresh verification for the updated base",
            json!({}),
        ),
        MergePreflightError::InvalidCommitId => (
            "invalid_commit_id",
            false,
            "inspect the API snapshot and require a full hexadecimal commit identifier",
            json!({}),
        ),
        MergePreflightError::InvalidExpectedBinding => (
            "invalid_expected_binding",
            false,
            "load current requirement, policy and verifier identities from the trusted authority",
            json!({}),
        ),
        MergePreflightError::ExpectedPostmergeProof => (
            "expected_postmerge_proof",
            false,
            "supply a pre-merge proof with no merge commit binding",
            json!({}),
        ),
        MergePreflightError::VerifierNotIndependent => (
            "verifier_not_independent",
            false,
            "assign verification to an identity distinct from the code executor",
            json!({}),
        ),
        MergePreflightError::StaleBinding(binding) => (
            match *binding {
                "base" => "stale_base",
                "candidate" => "stale_candidate",
                "requirements" => "stale_requirements",
                "policy" => "stale_policy",
                _ => "stale_binding",
            },
            false,
            "refresh the changed input and rerun independent verification",
            json!({"binding":binding}),
        ),
        MergePreflightError::Proof(GateError::MissingProof) => (
            "missing_proof",
            false,
            "run the independent verifier and provide its signed proof",
            json!({}),
        ),
        MergePreflightError::Proof(GateError::InvalidSignature) => (
            "invalid_proof_signature",
            false,
            "obtain the signed proof from the trusted verifier; do not edit it",
            json!({}),
        ),
        MergePreflightError::Proof(GateError::InvalidVerifierKey) => (
            "invalid_verifier_key",
            false,
            "configure the trusted verifier key with at least 32 bytes",
            json!({"minimum_bytes": agent_forge_core::MIN_TARGET_BOUND_PROOF_KEY_BYTES}),
        ),
        MergePreflightError::Proof(GateError::InvalidTarget) => (
            "invalid_proof_target",
            false,
            "use a canonical origin and repository target from the trusted host",
            json!({}),
        ),
        MergePreflightError::Proof(GateError::WrongTarget) => (
            "proof_target_mismatch",
            false,
            "request a fresh proof for the selected Forgejo profile",
            json!({}),
        ),
        MergePreflightError::Proof(GateError::UnsupportedProofVersion) => (
            "unsupported_proof_version",
            false,
            "issue a target-bound proof using the supported schema version",
            json!({}),
        ),
        MergePreflightError::Proof(GateError::StaleBinding(binding)) => (
            match *binding {
                "base" => "stale_base",
                "candidate" => "stale_candidate",
                "requirements" => "stale_requirements",
                "policy" => "stale_policy",
                _ => "stale_binding",
            },
            false,
            "refresh the changed input and rerun independent verification",
            json!({"binding":binding}),
        ),
        MergePreflightError::Proof(GateError::OutcomeFailed) => (
            "proof_failed",
            false,
            "repair the reported failure and request a new independent verification",
            json!({}),
        ),
        MergePreflightError::Proof(GateError::OutcomeInconclusive) => (
            "proof_inconclusive",
            false,
            "resolve the missing observation or unavailable verifier and rerun verification",
            json!({}),
        ),
    };
    json!({
        "code": code,
        "retryable": retryable,
        "action": action,
        "details": details,
    })
}

fn merge_preflight_result(
    pull_request_number: u64,
    upstream_read_attempts: usize,
    result: Result<(), MergePreflightError>,
) -> (Value, bool) {
    match result {
        Ok(()) => (
            json!({
                "schema_version": MERGE_PREFLIGHT_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "operation": "read_only_merge_preflight",
                "verdict": "proof_current",
                "pull_request_number": pull_request_number,
                "upstream_read_attempts": upstream_read_attempts,
                "merge_authorized": false,
                "merge_attempted": false,
            }),
            true,
        ),
        Err(error) => (
            {
                let verdict = if matches!(
                    &error,
                    MergePreflightError::Proof(agent_forge_core::GateError::OutcomeFailed)
                ) {
                    "rejected"
                } else {
                    "inconclusive"
                };
                json!({
                "schema_version": MERGE_PREFLIGHT_SCHEMA_VERSION,
                "api_version": FORGEJO_REST_API_VERSION,
                "operation": "read_only_merge_preflight",
                "verdict": verdict,
                "pull_request_number": pull_request_number,
                "upstream_read_attempts": upstream_read_attempts,
                "merge_authorized": false,
                "merge_attempted": false,
                "error": merge_preflight_error(&error),
                })
            },
            false,
        ),
    }
}

fn merge_preflight_external_failure(
    pull_request_number: u64,
    upstream_read_attempts: usize,
    code: &str,
    retryable: bool,
    action: &str,
    details: Value,
) -> (Value, bool) {
    (
        json!({
            "schema_version": MERGE_PREFLIGHT_SCHEMA_VERSION,
            "api_version": FORGEJO_REST_API_VERSION,
            "operation": "read_only_merge_preflight",
            "verdict": "inconclusive",
            "pull_request_number": pull_request_number,
            "upstream_read_attempts": upstream_read_attempts,
            "merge_authorized": false,
            "merge_attempted": false,
            "error": {
                "code": code,
                "retryable": retryable,
                "action": action,
                "details": details,
            }
        }),
        false,
    )
}

fn merge_preflight_api_failure(
    pull_request_number: u64,
    upstream_read_attempts: usize,
    error: ApiReadError,
) -> (Value, bool) {
    let (code, retryable, action, details) = match error {
        ApiReadError::Unavailable => (
            "upstream_unavailable",
            true,
            "retry the loopback read after confirming the local Forgejo service is available",
            json!({}),
        ),
        ApiReadError::Unauthorized => (
            "upstream_unauthorized",
            false,
            "confirm this repository and pull request are anonymously readable; the read-only preflight sends no credential, so keep private-repository checks within the host-authenticated merge authority",
            json!({"http_status":401}),
        ),
        ApiReadError::Forbidden => (
            "upstream_forbidden",
            false,
            "confirm the local repository allowlist and read permission",
            json!({"http_status":403}),
        ),
        ApiReadError::NotFound => (
            "resource_not_found",
            false,
            "verify the pull request number in the configured local repository",
            json!({"http_status":404}),
        ),
        ApiReadError::InvalidCredential => (
            "credential_source_invalid",
            false,
            "inspect the host credential provider without exposing its value",
            json!({}),
        ),
        ApiReadError::HttpStatus(status) => (
            "upstream_http_status",
            matches!(status, 408 | 429) || (500..=599).contains(&status),
            "inspect the Forgejo HTTP status and retry only when retryable is true",
            json!({"http_status":status}),
        ),
        ApiReadError::HttpStatusWithRetryAfter {
            status,
            retry_after_ms,
        } => (
            "upstream_http_status",
            matches!(status, 408 | 429) || (500..=599).contains(&status),
            "wait at least details.retry_after_ms before retrying; refresh preflight after the delay",
            json!({"http_status":status,"retry_after_ms":retry_after_ms}),
        ),
        ApiReadError::HttpStatusWithInvalidRetryAfter { status } => (
            "upstream_http_status",
            matches!(status, 408 | 429) || (500..=599).contains(&status),
            "the upstream Retry-After is invalid or ambiguous; do not retry automatically from it, and retry only under the caller's bounded policy after a fresh read",
            json!({"http_status":status,"retry_after_disposition":"invalid_or_ambiguous"}),
        ),
        ApiReadError::ResponseTooLarge {
            component,
            maximum_bytes,
        } => (
            "upstream_response_too_large",
            false,
            "the Forgejo response exceeded a local byte limit; reduce the requested resource or correct the upstream response, then reread instead of retrying unchanged",
            json!({"upstream":"forgejo","component":component,"maximum_bytes":maximum_bytes}),
        ),
        ApiReadError::InvalidResponse => (
            "upstream_invalid_response",
            false,
            "inspect the bounded Forgejo response before retrying",
            json!({}),
        ),
        ApiReadError::InconsistentPagination => (
            "review_pagination_inconsistent",
            true,
            "discard all pages and reread the complete review list before retrying the merge preflight",
            json!({}),
        ),
        ApiReadError::BatchCacheBudgetExceeded {
            maximum_cached_json_bytes,
        } => (
            "batch_cache_budget_exceeded",
            false,
            "split the requested reads into smaller batches",
            json!({"maximum_cached_json_bytes": maximum_cached_json_bytes}),
        ),
    };
    merge_preflight_external_failure(
        pull_request_number,
        upstream_read_attempts,
        code,
        retryable,
        action,
        details,
    )
}

fn run_merge_preflight(
    arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(Value, bool), String> {
    let arguments = arguments.collect::<Vec<_>>();
    if arguments.len() != 5 {
        return Err(usage());
    }
    let pull_request_number = arguments[0]
        .to_str()
        .ok_or_else(usage)?
        .parse::<u64>()
        .map_err(|_| "pull request number must be a positive integer".to_owned())?;
    if pull_request_number == 0 {
        return Err("pull request number must be a positive integer".to_owned());
    }
    let proof_path = PathBuf::from(&arguments[1]);
    let expected_path = PathBuf::from(&arguments[2]);
    let key_path = PathBuf::from(&arguments[3]);
    let execution_identity = arguments[4].to_str().ok_or_else(usage)?.to_owned();

    let expected: ExpectedBinding = serde_json::from_slice(&read_adapter_input(
        &expected_path,
        MAX_EXPECTED_BINDING_BYTES,
        false,
    )?)
    .map_err(|_| "expected binding must be valid JSON for schema v1".to_owned())?;
    let proof = if proof_path == Path::new("-") {
        None
    } else {
        Some(
            serde_json::from_slice::<SignedProof>(&read_adapter_input(
                &proof_path,
                MAX_PROOF_BYTES,
                false,
            )?)
            .map_err(|_| "proof must be valid signed-proof JSON".to_owned())?,
        )
    };
    let verifier_key = read_adapter_input(&key_path, MAX_VERIFIER_KEY_BYTES, true)?;
    let config = LocalForgejoConfigV1::default();
    let scope = BaseBranchScope::new(
        &config.origin,
        &config.owner,
        &config.repository,
        MERGE_PREFLIGHT_BASE_BRANCH,
    )
    .map_err(|error| format!("invalid local merge scope: {error}"))?;
    let uid = unsafe { libc::geteuid() };
    let lock_root = merge_lock_root_for_uid(uid);
    let _lock = match BaseBranchMergeLock::acquire(&lock_root, &scope, MERGE_LOCK_WAIT) {
        Ok(lock) => lock,
        Err(error) => {
            let (code, retryable, action) = match error {
                MergeLockError::TimedOut => (
                    "merge_lock_timeout",
                    true,
                    "wait for the current cooperating preflight and retry with a fresh snapshot",
                ),
                MergeLockError::InvalidScope => (
                    "invalid_merge_scope",
                    false,
                    "inspect the configured local Forgejo scope",
                ),
                MergeLockError::UntrustedLockDirectory => (
                    "untrusted_merge_lock_directory",
                    false,
                    "use a private local lock directory owned by this user",
                ),
                MergeLockError::UntrustedLockFile => (
                    "untrusted_merge_lock_file",
                    false,
                    "inspect the existing lock file and do not delete it while a process may hold it",
                ),
                MergeLockError::Io(_) => (
                    "merge_lock_io_failure",
                    false,
                    "inspect local filesystem permissions and lock support",
                ),
            };
            return Ok(merge_preflight_external_failure(
                pull_request_number,
                0,
                code,
                retryable,
                action,
                json!({}),
            ));
        }
    };

    let resource = ResourceRef {
        kind: ResourceKind::PullRequest,
        owner: config.owner.clone(),
        repository: config.repository.clone(),
        id: ResourceId::Number(pull_request_number),
    };
    let api = LocalLoopbackForgejoApi::new(config);
    let response = match api.read_current(&resource) {
        Ok(response) => response,
        Err(error) => {
            return Ok(merge_preflight_api_failure(
                pull_request_number,
                api.upstream_read_attempts.load(Ordering::Relaxed),
                error,
            ));
        }
    };
    let upstream_read_attempts = api.upstream_read_attempts.load(Ordering::Relaxed);
    let current = match PullRequestSnapshotV1::from_forgejo_response(
        LOCAL_FORGEJO_ORIGIN,
        LOCAL_REPOSITORY_OWNER,
        LOCAL_REPOSITORY_NAME,
        pull_request_number,
        &response,
    ) {
        Ok(current) => current,
        Err(error) => {
            return Ok(merge_preflight_result(
                pull_request_number,
                upstream_read_attempts,
                Err(error),
            ));
        }
    };
    let context = MergePreflightContextV1 {
        pull_request_number,
        execution_identity,
        expected,
    };
    Ok(merge_preflight_result(
        pull_request_number,
        upstream_read_attempts,
        validate_merge_preflight(&scope, &current, proof.as_ref(), &verifier_key, &context),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostReviewBaseBindingFileV1 {
    schema_version: u32,
    review_id: u64,
    reviewer_id: u64,
    reviewer_login: String,
    pull_request_number: u64,
    base_branch: String,
    base_commit: String,
    candidate_commit: String,
    source: String,
    #[serde(default)]
    mission_admission_sha256: Option<String>,
    #[serde(default)]
    mission_source_issue_number: Option<u64>,
}

fn verify_mission_admission_binding(
    profile: LocalForgejoTargetProfile,
    origin: &str,
    owner: &str,
    repository: &str,
    expected: &ExpectedBinding,
    host_binding: &HostReviewBaseBindingFileV1,
    inputs: &MergeAuthorityInputsV1,
) -> Result<Value, Value> {
    if profile == LocalForgejoTargetProfile::Merge014AgentForgeCore {
        if inputs.mission_admission_path.is_some()
            || inputs.mission_journal_path.is_some()
            || host_binding.mission_admission_sha256.is_some()
            || host_binding.mission_source_issue_number.is_some()
        {
            return Err(json!({
                "code":"unexpected_mission_admission_binding",
                "retryable":false,
                "action":"remove MISSION-012 admission inputs from the MERGE-014 target",
                "details":{},
            }));
        }
        return Ok(Value::Null);
    }

    let (Some(admission_path), Some(journal_path)) = (
        inputs.mission_admission_path.as_deref(),
        inputs.mission_journal_path.as_deref(),
    ) else {
        return Err(json!({
            "code":"mission_admission_missing",
            "retryable":false,
            "action":"supply the private admission handoff and durable mission execution journal",
            "details":{},
        }));
    };
    let admission_bytes = read_adapter_input(admission_path, MAX_MISSION_ADMISSION_BYTES, true)
        .map_err(|_| {
            json!({
                "code":"mission_admission_unavailable",
                "retryable":false,
                "action":"restore the owner-only regular admission file and retry the merge gate",
                "details":{},
            })
        })?;
    let admission_sha256 = agent_forge_core::sha256(&admission_bytes);
    if host_binding.mission_admission_sha256.as_deref() != Some(admission_sha256.as_str()) {
        return Err(json!({
            "code":"mission_admission_binding_mismatch",
            "retryable":false,
            "action":"recreate the host review binding from the exact admission used for verification",
            "details":{"admission_sha256":admission_sha256},
        }));
    }
    let admission: agent_forge_core::mission_admission::MissionHostAdmissionV1 =
        serde_json::from_slice(&admission_bytes).map_err(|_| {
            json!({
                "code":"mission_admission_invalid",
                "retryable":false,
                "action":"reissue a valid versioned mission admission before merge",
                "details":{},
            })
        })?;
    admission.validate().map_err(|code| {
        json!({
            "code":code,
            "retryable":false,
            "action":"reissue a valid versioned mission admission before merge",
            "details":{},
        })
    })?;

    let source_issue_number = admission.issue_number;
    let scope_matches = admission.scope.origin == origin
        && admission.scope.owner == owner
        && admission.scope.repository == repository
        && admission.scope.branch == MERGE_PREFLIGHT_BASE_BRANCH;
    if !scope_matches
        || host_binding.mission_source_issue_number != Some(source_issue_number)
        || admission.base_commit != expected.base_commit
        || admission.requirement_sha256 != expected.requirement_sha256
    {
        return Err(json!({
            "code":"mission_admission_binding_mismatch",
            "retryable":false,
            "action":"verify the PR against the admitted issue, protected base and requirement revision",
            "details":{
                "source_issue_number":source_issue_number,
                "base_commit":admission.base_commit,
                "requirement_sha256":admission.requirement_sha256,
            },
        }));
    }

    let scope = BaseBranchScope::new(origin, owner, repository, MERGE_PREFLIGHT_BASE_BRANCH)
        .map_err(|_| {
            json!({
                "code":"mission_admission_scope_invalid",
                "retryable":false,
                "action":"use the compiled MISSION-012 Forgejo scope",
                "details":{},
            })
        })?;
    let claim = agent_forge_core::agent_runtime::MissionExecutionJournalV1::open(journal_path)
        .verify_claimed_request(
            &scope,
            source_issue_number,
            &admission.allocation.allocation_id,
            admission.allocation.execution_generation,
            &admission.request,
        )
        .map_err(|error| {
            json!({
                "code":error.code,
                "retryable":error.retryable,
                "action":error.action,
                "details":error.details,
            })
        })?;

    Ok(json!({
        "schema_version": 1,
        "admission_sha256": admission_sha256,
        "issue_number": source_issue_number,
        "mission_id": claim.mission_id,
        "allocation_id": claim.allocation_id,
        "execution_generation": claim.execution_generation,
        "request_sha256": claim.request_sha256,
        "base_commit": admission.base_commit,
        "requirement_sha256": admission.requirement_sha256,
    }))
}

fn revalidate_mission_merge_inputs(
    profile: LocalForgejoTargetProfile,
    inputs: &MergeAuthorityInputsV1,
    context: &MissionMergeAttemptContextV1,
) -> Result<(), agent_forge_core::agent_runtime::AgentRuntimeErrorV1> {
    use agent_forge_core::agent_runtime::AgentRuntimeErrorV1;

    let unavailable = || {
        AgentRuntimeErrorV1::new(
            "merge_attempt_inputs_unavailable",
            false,
            "restore the same owner-only mission, proof, policy and review files before recovering or starting this merge",
            json!({}),
        )
    };
    let mismatch = || {
        AgentRuntimeErrorV1::new(
            "merge_attempt_binding_mismatch",
            false,
            "restore the exact host inputs bound to the durable merge attempt or admit a new generation",
            json!({"pull_request_number":context.binding.pull_request_number}),
        )
    };

    let expected_bytes =
        read_adapter_input(&inputs.expected_path, MAX_EXPECTED_BINDING_BYTES, true)
            .map_err(|_| unavailable())?;
    let expected: ExpectedBinding =
        serde_json::from_slice(&expected_bytes).map_err(|_| mismatch())?;
    let proof_bytes =
        read_adapter_input(&inputs.proof_path, MAX_PROOF_BYTES, true).map_err(|_| unavailable())?;
    let policy_bytes = read_adapter_input(&inputs.policy_path, MAX_POLICY_BYTES, true)
        .map_err(|_| unavailable())?;
    let review_binding_bytes = read_adapter_input(
        &inputs.review_binding_path,
        MAX_REVIEW_BASE_BINDING_BYTES,
        true,
    )
    .map_err(|_| unavailable())?;
    let admission_bytes =
        read_adapter_input(&context.admission_path, MAX_MISSION_ADMISSION_BYTES, true)
            .map_err(|_| unavailable())?;
    let admission: agent_forge_core::mission_admission::MissionHostAdmissionV1 =
        serde_json::from_slice(&admission_bytes).map_err(|_| mismatch())?;
    let review_binding: HostReviewBaseBindingFileV1 =
        serde_json::from_slice(&review_binding_bytes).map_err(|_| mismatch())?;
    let policy: Policy = serde_json::from_slice(&policy_bytes).map_err(|_| mismatch())?;

    if admission.validate().is_err()
        || admission != context.admission
        || agent_forge_core::sha256(&admission_bytes) != context.admission_sha256
        || agent_forge_core::sha256(&proof_bytes) != context.binding.proof_sha256
        || agent_forge_core::sha256(&policy_bytes) != context.binding.policy_sha256
        || agent_forge_core::sha256(&review_binding_bytes) != context.binding.review_sha256
        || expected.base_commit != context.binding.base_commit
        || expected.candidate_commit != context.binding.candidate_commit
        || expected.requirement_sha256 != context.binding.requirement_sha256
        || expected.policy_sha256 != context.binding.policy_sha256
        || review_binding.schema_version != 1
        || review_binding.pull_request_number != context.binding.pull_request_number
        || review_binding.base_commit != context.binding.base_commit
        || review_binding.candidate_commit != context.binding.candidate_commit
        || review_binding.mission_admission_sha256.as_deref()
            != Some(context.binding.admission_sha256.as_str())
        || policy.merge_approval_policy.is_none()
    {
        return Err(mismatch());
    }

    let proof_value: Value = serde_json::from_slice(&proof_bytes).map_err(|_| mismatch())?;
    let proof: TargetBoundProofV2 = serde_json::from_value(proof_value).map_err(|_| mismatch())?;
    let verifier_key = read_adapter_input(&inputs.key_path, MAX_VERIFIER_KEY_BYTES, true)
        .map_err(|_| unavailable())?;
    let target = ProofTargetV2::new(
        profile.selector(),
        context.scope.origin(),
        context.scope.owner(),
        context.scope.repository(),
        context.scope.branch(),
    )
    .map_err(|_| mismatch())?;
    agent_forge_core::verify_target_bound_proof_v2(Some(&proof), &verifier_key, &target, &expected)
        .map_err(|_| mismatch())?;
    Ok(())
}

fn merge_authority_result(
    pull_request_number: u64,
    verdict: &str,
    merge_attempted: bool,
    code: Option<&str>,
    retryable: bool,
    http_status: Option<u16>,
) -> (Value, bool) {
    let mut value = json!({
        "schema_version": 1,
        "api_version": FORGEJO_REST_API_VERSION,
        "operation": "local_test_merge_authority",
        "verdict": verdict,
        "pull_request_number": pull_request_number,
        "merge_authorized": verdict == "merged",
        "merge_attempted": merge_attempted,
        "external_writer_exclusion": "not_provided_by_cooperative_lock",
    });
    if let Some(status) = http_status {
        value["http_status"] = json!(status);
    }
    if let Some(code) = code {
        let mut error = json!({
            "code": code,
            "retryable": retryable,
            "http_status": http_status,
        });
        if code == "invalid_verifier_key" {
            error["action"] = json!(
                "replace the host-managed verifier key with at least 32 bytes and issue a new proof"
            );
            error["details"] = json!({
                "minimum_bytes": agent_forge_core::MIN_TARGET_BOUND_PROOF_KEY_BYTES,
            });
        } else if code == "draft_pull_request" {
            error["action"] = json!("resolve the draft state through the normal review decision");
            error["details"] = json!({"stage":"pull_request_read","draft":true});
        }
        value["error"] = error;
    }
    (value, verdict == "merged")
}

fn merge_writer_admission_refusal_result(
    profile: LocalForgejoTargetProfile,
    pull_request_number: u64,
    writer_profile: MergeWriterAdmissionProfileV1,
    error: MergeWriterAdmissionErrorV1,
) -> (Value, bool) {
    let (mut result, success) = merge_authority_result(
        pull_request_number,
        "inconclusive",
        false,
        Some(error.code()),
        false,
        None,
    );
    result["error"]["action"] = json!(error.action());
    result["error"]["details"] = json!({
        "writer_admission_profile": merge_writer_admission_profile_name(writer_profile),
        "host_single_writer_exclusion": if writer_profile == MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox {
            "supervisor_attestation_missing"
        } else {
            "not_attested"
        },
        "server_expected_base_precondition": "not_mandatory_or_unavailable",
        "merge_post_attempted": false,
    });
    result["target"] = json!({
        "selector": profile.selector(),
        "origin": profile.origin(),
        "owner": profile.owner(),
        "repository": profile.repository(),
        "base_branch": MERGE_PREFLIGHT_BASE_BRANCH,
    });
    (result, success)
}

fn merge_attempt_resolution_from_result(
    result: &Value,
    success: bool,
    observation_sha256: Option<&str>,
) -> agent_forge_core::agent_runtime::MergeAttemptResolutionV1 {
    use agent_forge_core::agent_runtime::MergeAttemptResolutionV1;

    let reason_code = result
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("merge_outcome_not_confirmed");
    if success {
        if let (Some(commit), Some(tree), Some(observation)) = (
            result.get("merge_commit_sha").and_then(Value::as_str),
            result
                .get("merge_commit_tree_entries_sha256")
                .and_then(Value::as_str),
            observation_sha256,
        ) {
            return MergeAttemptResolutionV1::merged(commit, tree, observation);
        }
        return MergeAttemptResolutionV1::inconclusive("merge_result_binding_missing");
    }
    let retryable = result
        .get("error")
        .and_then(|error| error.get("retryable"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if result.get("verdict").and_then(Value::as_str) == Some("rejected") {
        MergeAttemptResolutionV1::rejected(reason_code).with_retryable(retryable)
    } else {
        MergeAttemptResolutionV1::inconclusive(reason_code).with_retryable(retryable)
    }
}

fn finish_merge_attempt_decision(
    result: (Value, bool),
    observation_sha256: Option<String>,
) -> (
    Value,
    bool,
    agent_forge_core::agent_runtime::MergeAttemptResolutionV1,
) {
    let resolution =
        merge_attempt_resolution_from_result(&result.0, result.1, observation_sha256.as_deref());
    (result.0, result.1, resolution)
}

fn merge_readback_observation_sha256(
    readback: &Result<MergeReadbackV1, ApiReadError>,
) -> Option<String> {
    readback
        .as_ref()
        .ok()
        .and_then(|readback| serde_json::to_vec(readback).ok())
        .map(|bytes| agent_forge_core::sha256(&bytes))
}

fn finalize_readback_only(
    pull_request_number: u64,
    expected_owner: &str,
    expected_repository: &str,
    validated_base_sha: &str,
    submitted_head_sha: &str,
    readback: Result<MergeReadbackV1, ApiReadError>,
) -> (
    Value,
    bool,
    agent_forge_core::agent_runtime::MergeAttemptResolutionV1,
) {
    let observation_sha256 = merge_readback_observation_sha256(&readback);
    let outcome = MergePostReadbackV1 {
        // A durable prior intent exists. This flag describes that attempt;
        // this recovery call itself makes no POST.
        post_attempted: true,
        post_status: None,
        post_error: None,
        readback,
    };
    let (mut result, success) = finalize_merge_post_readback(
        pull_request_number,
        expected_owner,
        expected_repository,
        validated_base_sha,
        submitted_head_sha,
        outcome,
    );
    // A durable intent does not prove whether the stopped process sent its
    // POST. The readback may resolve the state, but the submission status is
    // unknown and must not be reported as a POST by this recovery invocation.
    result["merge_attempted"] = json!(false);
    result["merge_attempt_recovery"] = json!(true);
    result["merge_post_attempt_status"] = json!("unknown");
    let resolution =
        merge_attempt_resolution_from_result(&result, success, observation_sha256.as_deref());
    (result, success, resolution)
}

fn merge_attempt_resolution_result(
    pull_request_number: u64,
    resolution: &agent_forge_core::agent_runtime::MergeAttemptResolutionV1,
) -> (Value, bool) {
    use agent_forge_core::agent_runtime::MergeAttemptOutcomeV1;

    let (verdict, code, retryable) = match resolution.outcome {
        MergeAttemptOutcomeV1::Merged => ("merged", None, false),
        MergeAttemptOutcomeV1::Rejected => (
            "rejected",
            resolution.reason_code.as_deref(),
            resolution.is_retryable(),
        ),
        MergeAttemptOutcomeV1::Inconclusive => (
            "inconclusive",
            resolution.reason_code.as_deref(),
            resolution.is_retryable(),
        ),
    };
    let (mut result, success) =
        merge_authority_result(pull_request_number, verdict, false, code, retryable, None);
    result["merge_attempt_recovery"] = json!(true);
    result["merge_post_attempt_status"] = json!("unknown");
    if let Some(commit) = resolution.merge_commit.as_deref() {
        result["merge_commit_sha"] = json!(commit);
    }
    if let Some(tree) = resolution.merge_tree_manifest_sha256.as_deref() {
        result["merge_commit_tree_entries_sha256"] = json!(tree);
    }
    if let Some(observation) = resolution.observation_sha256.as_deref() {
        result["readback_observation_sha256"] = json!(observation);
    }
    (result, success)
}

fn merge_attempt_runtime_error_result(
    pull_request_number: u64,
    error: &agent_forge_core::agent_runtime::AgentRuntimeErrorV1,
    merge_attempted: bool,
) -> (Value, bool) {
    let verdict = if error.retryable {
        "inconclusive"
    } else {
        "rejected"
    };
    let mut result = merge_authority_result(
        pull_request_number,
        verdict,
        merge_attempted,
        Some(&error.code),
        error.retryable,
        None,
    )
    .0;
    result["error"]["action"] = json!(error.action);
    result["error"]["details"] = error.details.clone();
    (result, false)
}

/// Perform the final read/POST/readback window for a merge. MISSION-012 calls
/// this only while holding its durable claim's per-mission generation lock;
/// the default MERGE-014 profile calls it directly and preserves its existing
/// policy. The network effects remain bounded by the API client's deadline.
fn run_final_merge_window(
    api: &LocalLoopbackForgejoApi,
    policy_api: &LocalLoopbackForgejoApi,
    current: &PullRequestSnapshotV1,
    validated_branch_protection_rule: &Value,
    pull_request_number: u64,
    mission_admission_evidence: &Value,
    before_post: impl FnOnce() -> Result<(), agent_forge_core::agent_runtime::AgentRuntimeErrorV1>,
) -> (Value, bool) {
    let (result, success, _) = run_final_merge_window_with_resolution(
        api,
        policy_api,
        current,
        validated_branch_protection_rule,
        pull_request_number,
        mission_admission_evidence,
        before_post,
    );
    (result, success)
}

fn validate_supervisor_capability_before_post(
    capability: Option<&VerifiedSupervisorMergeCapabilityV1>,
) -> Result<(), agent_forge_core::agent_runtime::AgentRuntimeErrorV1> {
    if let Some(capability) = capability {
        return capability.ensure_live_before_merge_post().map_err(|error| {
            agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                error.code(),
                true,
                error.action(),
                json!({ "stage": "immediately_before_merge_post" }),
            )
        });
    }
    if cfg!(test) {
        // Existing in-memory contract fixtures exercise the downstream merge
        // state machine without claiming a host-process attestation.
        return Ok(());
    }
    Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
        SupervisorCapabilityErrorV1::MissingPacket.code(),
        false,
        SupervisorCapabilityErrorV1::MissingPacket.action(),
        json!({ "stage": "immediately_before_merge_post" }),
    ))
}

fn run_final_merge_window_with_resolution(
    api: &LocalLoopbackForgejoApi,
    policy_api: &LocalLoopbackForgejoApi,
    current: &PullRequestSnapshotV1,
    validated_branch_protection_rule: &Value,
    pull_request_number: u64,
    mission_admission_evidence: &Value,
    before_post: impl FnOnce() -> Result<(), agent_forge_core::agent_runtime::AgentRuntimeErrorV1>,
) -> (
    Value,
    bool,
    agent_forge_core::agent_runtime::MergeAttemptResolutionV1,
) {
    let merge_body = match serde_json::to_vec(&json!({
        "Do": "merge",
        "head_commit_id": current.head_sha,
    })) {
        Ok(body) => body,
        Err(_) => {
            return finish_merge_attempt_decision(
                merge_authority_result(
                    pull_request_number,
                    "rejected",
                    false,
                    Some("merge_request_serialization_failed"),
                    false,
                    None,
                ),
                None,
            );
        }
    };
    let post_outcome = match validate_then_post_merge_with_host_check(
        api,
        policy_api,
        current,
        validated_branch_protection_rule,
        &merge_body,
        before_post,
        post_merge_once_then_readback,
    ) {
        Ok(post_outcome) => post_outcome,
        Err(FinalMergeStateError::Api(error)) => {
            return finish_merge_attempt_decision(
                merge_authority_api_error_result(
                    pull_request_number,
                    "final-merge-state-read",
                    &error,
                ),
                None,
            );
        }
        Err(FinalMergeStateError::InvalidPullRequest) => {
            return finish_merge_attempt_decision(
                merge_authority_result(
                    pull_request_number,
                    "rejected",
                    false,
                    Some("invalid_final_pull_request"),
                    false,
                    None,
                ),
                None,
            );
        }
        Err(FinalMergeStateError::PullRequestChanged) => {
            return finish_merge_attempt_decision(
                merge_authority_result(
                    pull_request_number,
                    "rejected",
                    false,
                    Some("pull_request_changed_during_gate"),
                    true,
                    None,
                ),
                None,
            );
        }
        Err(FinalMergeStateError::StaleBase) => {
            return finish_merge_attempt_decision(
                merge_authority_result(
                    pull_request_number,
                    "rejected",
                    false,
                    Some("stale_base"),
                    false,
                    None,
                ),
                None,
            );
        }
        Err(FinalMergeStateError::BranchProtectionChanged) => {
            return finish_merge_attempt_decision(
                merge_authority_result(
                    pull_request_number,
                    "rejected",
                    false,
                    Some("branch_protection_changed_during_gate"),
                    true,
                    None,
                ),
                None,
            );
        }
        Err(FinalMergeStateError::MissionInputs(error)) => {
            return finish_merge_attempt_decision(
                merge_attempt_runtime_error_result(pull_request_number, &error, false),
                None,
            );
        }
    };
    let observation_sha256 = merge_readback_observation_sha256(&post_outcome.readback);
    let (mut result, success) = finalize_merge_post_readback(
        pull_request_number,
        &api.config.owner,
        &api.config.repository,
        &current.base_sha,
        &current.head_sha,
        post_outcome,
    );
    if success && !mission_admission_evidence.is_null() {
        result["mission_admission"] = mission_admission_evidence.clone();
    }
    finish_merge_attempt_decision((result, success), observation_sha256)
}

fn gate_api_json(api: &LocalLoopbackForgejoApi, path: &str) -> Result<Value, ApiReadError> {
    let remaining = api.deadline.saturating_duration_since(Instant::now());
    if remaining < Duration::from_millis(1) {
        return Err(ApiReadError::Unavailable);
    }
    let response =
        api.request_api_once(path, "GET", None, Duration::from_secs(5).min(remaining))?;
    match response.status {
        200 => response.body.ok_or(ApiReadError::InvalidResponse),
        401 => Err(ApiReadError::Unauthorized),
        403 => Err(ApiReadError::Forbidden),
        404 => Err(ApiReadError::NotFound),
        status => Err(ApiReadError::HttpStatus(status)),
    }
}

fn main_commit_matches_validated_base(
    api: &LocalLoopbackForgejoApi,
    validated_base_sha: &str,
) -> Result<bool, ApiReadError> {
    let main = gate_api_json(
        api,
        &api.repository_api_path(&format!("/branches/{MERGE_PREFLIGHT_BASE_BRANCH}")),
    )?;
    let main_sha = main
        .get("commit")
        .and_then(|commit| commit.get("id"))
        .and_then(Value::as_str)
        .ok_or(ApiReadError::InvalidResponse)?;
    Ok(main_sha == validated_base_sha)
}

#[derive(Debug)]
enum FinalMergeStateError {
    Api(ApiReadError),
    InvalidPullRequest,
    PullRequestChanged,
    StaleBase,
    BranchProtectionChanged,
    MissionInputs(agent_forge_core::agent_runtime::AgentRuntimeErrorV1),
}

fn validate_final_merge_state(
    api: &LocalLoopbackForgejoApi,
    policy_api: &LocalLoopbackForgejoApi,
    validated_pull_request: &PullRequestSnapshotV1,
    validated_branch_protection_rule: &Value,
) -> Result<(), FinalMergeStateError> {
    if policy_api.config.origin != api.config.origin
        || policy_api.config.owner != api.config.owner
        || policy_api.config.repository != api.config.repository
    {
        return Err(FinalMergeStateError::Api(ApiReadError::Forbidden));
    }
    // Forgejo has no expected-base compare-and-swap on its merge endpoint.
    let final_pr_json = gate_api_json(
        api,
        &format!(
            "/repos/{}/{}/pulls/{}",
            validated_pull_request.owner,
            validated_pull_request.repository,
            validated_pull_request.number
        ),
    )
    .map_err(FinalMergeStateError::Api)?;
    let final_pr = PullRequestSnapshotV1::from_forgejo_response(
        &validated_pull_request.origin,
        &validated_pull_request.owner,
        &validated_pull_request.repository,
        validated_pull_request.number,
        &final_pr_json,
    )
    .map_err(|_| FinalMergeStateError::InvalidPullRequest)?;
    if final_pr != *validated_pull_request {
        return Err(FinalMergeStateError::PullRequestChanged);
    }

    if !main_commit_matches_validated_base(api, &validated_pull_request.base_sha)
        .map_err(FinalMergeStateError::Api)?
    {
        return Err(FinalMergeStateError::StaleBase);
    }

    // Forgejo exposes no atomic policy-version precondition on the merge API.
    // Re-read the exact rule that authorized this decision in the final gate
    // window and refuse if it changed since the earlier review.
    let final_protection = gate_api_json(
        policy_api,
        &policy_api.repository_api_path("/branch_protections"),
    )
    .map_err(FinalMergeStateError::Api)?;
    let final_rule = match branch_protection_rule(&final_protection) {
        Ok(rule) => rule,
        Err("invalid_branch_protection_response") => {
            return Err(FinalMergeStateError::Api(ApiReadError::InvalidResponse));
        }
        Err(_) => return Err(FinalMergeStateError::BranchProtectionChanged),
    };
    if final_rule != validated_branch_protection_rule {
        return Err(FinalMergeStateError::BranchProtectionChanged);
    }

    Ok(())
}

fn validate_then_post_merge_with_host_check(
    api: &LocalLoopbackForgejoApi,
    policy_api: &LocalLoopbackForgejoApi,
    validated_pull_request: &PullRequestSnapshotV1,
    validated_branch_protection_rule: &Value,
    merge_body: &[u8],
    before_post: impl FnOnce() -> Result<(), agent_forge_core::agent_runtime::AgentRuntimeErrorV1>,
    post_merge: impl FnOnce(&LocalLoopbackForgejoApi, u64, &[u8]) -> MergePostReadbackV1,
) -> Result<MergePostReadbackV1, FinalMergeStateError> {
    validate_final_merge_state(
        api,
        policy_api,
        validated_pull_request,
        validated_branch_protection_rule,
    )?;
    before_post().map_err(FinalMergeStateError::MissionInputs)?;
    Ok(post_merge(api, validated_pull_request.number, merge_body))
}

fn final_merge_state_runtime_error(
    error: FinalMergeStateError,
) -> agent_forge_core::agent_runtime::AgentRuntimeErrorV1 {
    use agent_forge_core::agent_runtime::AgentRuntimeErrorV1;

    if let FinalMergeStateError::MissionInputs(error) = error {
        return error;
    }
    let (code, retryable, action, details) = match error {
        FinalMergeStateError::Api(ApiReadError::ResponseTooLarge {
            component,
            maximum_bytes,
        }) => (
            "upstream_response_too_large",
            false,
            "the Forgejo response exceeded a local byte limit; reduce the requested resource or correct the upstream response, then reread instead of retrying unchanged",
            json!({
                "upstream": "forgejo",
                "component": component,
                "maximum_bytes": maximum_bytes,
            }),
        ),
        FinalMergeStateError::Api(error) => {
            let (code, retryable) = gate_api_failure_code(&error);
            (
                code,
                retryable,
                "refresh the candidate and rerun the complete merge preflight before authorizing a new attempt",
                json!({}),
            )
        }
        FinalMergeStateError::InvalidPullRequest => (
            "invalid_final_pull_request",
            false,
            "refresh the candidate and rerun the complete merge preflight before authorizing a new attempt",
            json!({}),
        ),
        FinalMergeStateError::PullRequestChanged => (
            "pull_request_changed_during_gate",
            true,
            "refresh the candidate and rerun the complete merge preflight before authorizing a new attempt",
            json!({}),
        ),
        FinalMergeStateError::StaleBase => (
            "stale_base",
            false,
            "refresh the candidate and rerun the complete merge preflight before authorizing a new attempt",
            json!({}),
        ),
        FinalMergeStateError::BranchProtectionChanged => (
            "branch_protection_changed_during_gate",
            true,
            "refresh the candidate and rerun the complete merge preflight before authorizing a new attempt",
            json!({}),
        ),
        FinalMergeStateError::MissionInputs(_) => unreachable!("returned above"),
    };
    AgentRuntimeErrorV1::new(code, retryable, action, details)
}

fn gate_api_failure_code(error: &ApiReadError) -> (&'static str, bool) {
    match error {
        ApiReadError::Unavailable => ("upstream_unavailable", true),
        ApiReadError::Unauthorized => ("upstream_unauthorized", false),
        ApiReadError::Forbidden => ("upstream_forbidden", false),
        ApiReadError::NotFound => ("resource_not_found", false),
        ApiReadError::HttpStatus(status)
        | ApiReadError::HttpStatusWithRetryAfter { status, .. }
        | ApiReadError::HttpStatusWithInvalidRetryAfter { status } => (
            "upstream_http_status",
            matches!(status, 408 | 429) || (500..=599).contains(status),
        ),
        ApiReadError::InvalidResponse => ("upstream_invalid_response", false),
        ApiReadError::ResponseTooLarge { .. } => ("upstream_response_too_large", false),
        ApiReadError::InvalidCredential => ("credential_source_invalid", false),
        ApiReadError::InconsistentPagination => ("review_pagination_inconsistent", true),
        ApiReadError::BatchCacheBudgetExceeded { .. } => ("batch_cache_budget_exceeded", false),
    }
}

fn merge_authority_api_error_result(
    pull_request_number: u64,
    stage: &'static str,
    error: &ApiReadError,
) -> (Value, bool) {
    let (code, retryable) = gate_api_failure_code(error);
    let http_status = match error {
        ApiReadError::HttpStatus(status)
        | ApiReadError::HttpStatusWithRetryAfter { status, .. }
        | ApiReadError::HttpStatusWithInvalidRetryAfter { status } => Some(*status),
        _ => None,
    };
    let mut result = merge_authority_result(
        pull_request_number,
        "inconclusive",
        false,
        Some(code),
        retryable,
        http_status,
    );
    result.0["error"]["stage"] = json!(stage);
    if matches!(error, ApiReadError::InconsistentPagination) {
        result.0["error"]["action"] = json!(
            "discard all pages and reread the complete review list before retrying the merge preflight"
        );
    }
    if let ApiReadError::HttpStatusWithInvalidRetryAfter { status } = error {
        result.0["error"]["action"] = json!(
            "the upstream Retry-After is invalid or ambiguous; do not use it or retry automatically, and perform a fresh read before another merge preflight"
        );
        result.0["error"]["details"] = json!({
            "http_status": status,
            "retry_after_disposition": "invalid_or_ambiguous",
        });
    }
    if let ApiReadError::ResponseTooLarge {
        component,
        maximum_bytes,
    } = error
    {
        result.0["error"]["action"] = json!(
            "the Forgejo response exceeded a local byte limit; reduce the requested resource or correct the upstream response, then reread instead of retrying unchanged"
        );
        result.0["error"]["details"] = json!({
            "component": component,
            "maximum_bytes": maximum_bytes,
        });
    }
    result
}

fn merge_post_readback_failure_result(
    pull_request_number: u64,
    post_status: Option<u16>,
    error: ApiReadError,
) -> (Value, bool) {
    let (code, _) = gate_api_failure_code(&error);
    let mut result = merge_authority_result(
        pull_request_number,
        "inconclusive",
        true,
        Some(if post_status.is_some() {
            "post_readback_failed"
        } else {
            code
        }),
        true,
        post_status,
    );
    if let ApiReadError::ResponseTooLarge {
        component,
        maximum_bytes,
    } = error
    {
        result.0["error"]["action"] = json!(
            "the Forgejo response exceeded a local byte limit; correct the upstream response and perform a fresh read before another merge attempt"
        );
        result.0["error"]["details"] = json!({
            "component": component,
            "maximum_bytes": maximum_bytes,
        });
    }
    result
}

fn forgejo_user_by_login(
    api: &LocalLoopbackForgejoApi,
    login: &str,
) -> Result<(u64, String), ApiReadError> {
    if login.is_empty()
        || login.len() > 255
        || !login
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ApiReadError::InvalidResponse);
    }
    let user = gate_api_json(api, &format!("/users/{login}"))?;
    let user_id = user.get("id").and_then(Value::as_u64).unwrap_or(0);
    user.get("login")
        .and_then(Value::as_str)
        .filter(|actual_login| user_id != 0 && *actual_login == login)
        .map(|actual_login| (user_id, actual_login.to_owned()))
        .ok_or(ApiReadError::InvalidResponse)
}

fn forgejo_review_pages(
    api: &LocalLoopbackForgejoApi,
    scope: &PullRequestReviewScopeV1,
) -> Result<PullRequestReviewsSnapshotV1, ApiReadError> {
    const PAGE_SIZE: u64 = 50;
    let mut pages = Vec::new();
    let mut page = 1_u64;
    let total_count = loop {
        if page > MAX_REVIEW_PAGES as u64 {
            return Err(ApiReadError::InvalidResponse);
        }
        let path = format!(
            "/repos/{}/{}/pulls/{}/reviews?limit={PAGE_SIZE}&page={page}",
            scope.owner(),
            scope.repository(),
            scope.pull_request_number(),
        );
        let remaining = api.deadline.saturating_duration_since(Instant::now());
        if remaining < Duration::from_millis(1) {
            return Err(ApiReadError::Unavailable);
        }
        let response =
            api.request_api_once(&path, "GET", None, Duration::from_secs(5).min(remaining))?;
        if response.status != 200 {
            return Err(match response.status {
                401 => ApiReadError::Unauthorized,
                403 => ApiReadError::Forbidden,
                404 => ApiReadError::NotFound,
                status => ApiReadError::HttpStatus(status),
            });
        }
        let total = response.total_count.ok_or(ApiReadError::InvalidResponse)?;
        let body = response.body.ok_or(ApiReadError::InvalidResponse)?;
        let body = serde_json::to_vec(&body).map_err(|_| ApiReadError::InvalidResponse)?;
        pages.push(PullRequestReviewPageBodyV1 {
            scope: scope.clone(),
            page: u32::try_from(page).map_err(|_| ApiReadError::InvalidResponse)?,
            total_count: total,
            body: Cursor::new(body),
        });
        if total > 10_000 {
            return Err(ApiReadError::InvalidResponse);
        }
        let pages_required = total.div_ceil(PAGE_SIZE).max(1);
        if page >= pages_required {
            break total;
        }
        page += 1;
    };
    if pages.is_empty()
        || pages
            .last()
            .is_none_or(|last| last.total_count != total_count)
    {
        return Err(ApiReadError::InvalidResponse);
    }
    collect_pull_request_review_json_pages(scope, pages).map_err(|error| match error {
        MergeApprovalError::IncompleteReviewList => ApiReadError::InconsistentPagination,
        _ => ApiReadError::InvalidResponse,
    })
}

fn branch_protection_rule(protection_response: &Value) -> Result<&Value, &'static str> {
    let Some(protections) = protection_response.as_array() else {
        return Err("invalid_branch_protection_response");
    };
    let matching = protections
        .iter()
        .filter(|item| item.get("rule_name").and_then(Value::as_str) == Some("main"))
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err("branch_protection_rule_missing_or_ambiguous");
    }
    Ok(matching[0])
}

fn validate_local_test_branch_protection<'a>(
    protection_response: &'a Value,
    policy: &Policy,
    reviewer_logins: &[String],
    merge_login: &str,
) -> Result<&'a Value, &'static str> {
    let rule = branch_protection_rule(protection_response)?;
    let Some(approval_policy) = policy.merge_approval_policy.as_ref() else {
        return Err("merge_approval_policy_missing");
    };
    let usernames = |field: &str| -> Option<Vec<&str>> {
        rule.get(field)?
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect()
    };
    let approval_whitelist =
        usernames("approvals_whitelist_username").ok_or("invalid_approval_whitelist")?;
    let merge_whitelist =
        usernames("merge_whitelist_usernames").ok_or("invalid_merge_whitelist")?;
    let merge_whitelist_teams =
        usernames("merge_whitelist_teams").ok_or("invalid_merge_whitelist_teams")?;
    if rule.get("branch_name").and_then(Value::as_str) != Some("main")
        || rule.get("required_approvals").and_then(Value::as_u64)
            != Some(u64::from(approval_policy.required_approvals))
        || rule
            .get("enable_approvals_whitelist")
            .and_then(Value::as_bool)
            != Some(true)
        || rule.get("apply_to_admins").and_then(Value::as_bool) != Some(true)
        || rule.get("dismiss_stale_approvals").and_then(Value::as_bool) != Some(true)
        || rule
            .get("block_on_outdated_branch")
            .and_then(Value::as_bool)
            != Some(true)
        || rule.get("enable_merge_whitelist").and_then(Value::as_bool) != Some(true)
        || rule.get("enable_push").and_then(Value::as_bool) != Some(false)
        || rule
            .get("unprotected_file_patterns")
            .and_then(Value::as_str)
            != Some("")
        || reviewer_logins
            .iter()
            .any(|login| !approval_whitelist.contains(&login.as_str()))
        || merge_whitelist.len() != 1
        || merge_whitelist.first().copied() != Some(merge_login)
        || !merge_whitelist_teams.is_empty()
    {
        return Err("branch_protection_does_not_match_local_gate_policy");
    }
    Ok(rule)
}

fn valid_full_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

type GitTreeEntryFingerprintV1 = (String, String, String, String, i64);
type MergeReadbackV1 = (
    Value,
    Value,
    Value,
    Value,
    Value,
    Option<Vec<GitTreeEntryFingerprintV1>>,
    Option<Vec<GitTreeEntryFingerprintV1>>,
);

/// Forgejo v17's GetSingleCommit response sets RepoCommit.Tree.SHA to the
/// commit ID. Fetching the tree route by commit ID resolves the commit to its
/// root tree entries; compare those entry identities instead of trusting that
/// misleading `commit.tree.sha` field.
fn forgejo_commit_tree_entries(
    api: &LocalLoopbackForgejoApi,
    commit_sha: &str,
) -> Result<Vec<GitTreeEntryFingerprintV1>, ApiReadError> {
    if !valid_full_git_object_id(commit_sha) {
        return Err(ApiReadError::InvalidResponse);
    }

    let mut entries = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut expected_total = None;
    let mut effective_page_size = None;
    let mut total_pages = None;
    for page in 1..=MAX_GIT_TREE_PAGES {
        let response = gate_api_json(
            api,
            &api.repository_api_path(&format!(
                "/git/trees/{commit_sha}?page={page}&per_page={GIT_TREE_REQUEST_PAGE_SIZE}&recursive=false"
            )),
        )?;
        if response.get("sha").and_then(Value::as_str) != Some(commit_sha)
            || response.get("page").and_then(Value::as_u64) != Some(page as u64)
        {
            return Err(ApiReadError::InvalidResponse);
        }
        let total = response
            .get("total_count")
            .and_then(Value::as_u64)
            .and_then(|total| usize::try_from(total).ok())
            .filter(|total| *total <= MAX_GIT_TREE_ENTRY_COUNT)
            .ok_or(ApiReadError::InvalidResponse)?;
        if expected_total.is_some_and(|previous| previous != total) {
            return Err(ApiReadError::InvalidResponse);
        }
        expected_total = Some(total);
        let page_entries = match response.get("tree") {
            Some(Value::Array(entries)) => entries.as_slice(),
            Some(Value::Null) if total == 0 && page == 1 => &[],
            _ => return Err(ApiReadError::InvalidResponse),
        };
        let page_size = match effective_page_size {
            Some(page_size) => page_size,
            None if total == 0 => 1,
            None if !page_entries.is_empty() => page_entries.len(),
            None => return Err(ApiReadError::InvalidResponse),
        };
        if page_size == 0 || page_size > GIT_TREE_REQUEST_PAGE_SIZE {
            return Err(ApiReadError::InvalidResponse);
        }
        effective_page_size = Some(page_size);
        if total_pages.is_none() {
            let page_count = total.div_ceil(page_size).max(1);
            if page_count > MAX_GIT_TREE_PAGES {
                return Err(ApiReadError::InvalidResponse);
            }
            total_pages = Some(page_count);
        }
        let page_count = total_pages.ok_or(ApiReadError::InvalidResponse)?;
        if page > page_count {
            return Err(ApiReadError::InvalidResponse);
        }
        let offset = (page - 1)
            .checked_mul(page_size)
            .ok_or(ApiReadError::InvalidResponse)?;
        let expected_page_entries = total.saturating_sub(offset).min(page_size);
        if page_entries.len() != expected_page_entries
            || response.get("truncated").and_then(Value::as_bool) != Some(total > page_size)
        {
            return Err(ApiReadError::InvalidResponse);
        }
        for entry in page_entries {
            let path = entry
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty() && !path.contains('/') && !path.contains('\0'))
                .ok_or(ApiReadError::InvalidResponse)?;
            let mode = entry
                .get("mode")
                .and_then(Value::as_str)
                .filter(|mode| {
                    mode.len() == 6 && mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
                })
                .ok_or(ApiReadError::InvalidResponse)?;
            let entry_type = entry
                .get("type")
                .and_then(Value::as_str)
                .ok_or(ApiReadError::InvalidResponse)?;
            let mode_type_is_valid = matches!(
                (mode, entry_type),
                ("040000", "tree")
                    | ("100644", "blob")
                    | ("100755", "blob")
                    | ("120000", "blob")
                    | ("160000", "commit")
            );
            let sha = entry
                .get("sha")
                .and_then(Value::as_str)
                .filter(|sha| valid_full_git_object_id(sha))
                .ok_or(ApiReadError::InvalidResponse)?;
            let size = entry
                .get("size")
                .and_then(Value::as_i64)
                .filter(|size| *size >= 0)
                .ok_or(ApiReadError::InvalidResponse)?;
            if !mode_type_is_valid || !seen_paths.insert(path.to_owned()) {
                return Err(ApiReadError::InvalidResponse);
            }
            entries.push((
                path.to_owned(),
                mode.to_owned(),
                entry_type.to_owned(),
                sha.to_owned(),
                size,
            ));
        }
        if page == page_count {
            break;
        }
    }
    if entries.len() != expected_total.ok_or(ApiReadError::InvalidResponse)? {
        return Err(ApiReadError::InvalidResponse);
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

fn merged_readback(
    api: &LocalLoopbackForgejoApi,
    pull_request_number: u64,
) -> Result<MergeReadbackV1, ApiReadError> {
    let pull_request = gate_api_json(
        api,
        &api.repository_api_path(&format!("/pulls/{pull_request_number}")),
    )?;
    let issue_number = pull_request
        .get("number")
        .and_then(Value::as_u64)
        .ok_or(ApiReadError::InvalidResponse)?;
    if issue_number != pull_request_number {
        return Err(ApiReadError::InvalidResponse);
    }
    let (merge_commit, candidate_commit, merge_commit_sha, candidate_sha) =
        if pull_request.get("merged").and_then(Value::as_bool) == Some(true) {
            let candidate_sha = pull_request
                .get("head")
                .and_then(|head| head.get("sha"))
                .and_then(Value::as_str)
                .filter(|sha| valid_full_git_object_id(sha))
                .ok_or(ApiReadError::InvalidResponse)?;
            let merge_commit_sha = pull_request
                .get("merge_commit_sha")
                .and_then(Value::as_str)
                .filter(|sha| valid_full_git_object_id(sha))
                .ok_or(ApiReadError::InvalidResponse)?;
            let merge_commit = gate_api_json(
                api,
                &api.repository_api_path(&format!("/git/commits/{merge_commit_sha}")),
            )?;
            let candidate_commit = gate_api_json(
                api,
                &api.repository_api_path(&format!("/git/commits/{candidate_sha}")),
            )?;
            (
                merge_commit,
                candidate_commit,
                Some(merge_commit_sha.to_owned()),
                Some(candidate_sha.to_owned()),
            )
        } else if pull_request.get("merged").and_then(Value::as_bool) == Some(false) {
            (Value::Null, Value::Null, None, None)
        } else {
            return Err(ApiReadError::InvalidResponse);
        };
    let main = gate_api_json(
        api,
        &api.repository_api_path(&format!("/branches/{MERGE_PREFLIGHT_BASE_BRANCH}")),
    )?;
    let issue = gate_api_json(
        api,
        &api.repository_api_path(&format!("/issues/{issue_number}")),
    )?;
    if issue.get("number").and_then(Value::as_u64) != Some(pull_request_number) {
        return Err(ApiReadError::InvalidResponse);
    }
    let (merge_tree_entries, candidate_tree_entries) =
        match (merge_commit_sha.as_deref(), candidate_sha.as_deref()) {
            (Some(merge_sha), Some(candidate_sha)) => (
                Some(forgejo_commit_tree_entries(api, merge_sha)?),
                Some(forgejo_commit_tree_entries(api, candidate_sha)?),
            ),
            (None, None) => (None, None),
            _ => return Err(ApiReadError::InvalidResponse),
        };
    Ok((
        pull_request,
        merge_commit,
        candidate_commit,
        main,
        issue,
        merge_tree_entries,
        candidate_tree_entries,
    ))
}

struct MergePostReadbackV1 {
    post_attempted: bool,
    post_status: Option<u16>,
    post_error: Option<ApiReadError>,
    readback: Result<MergeReadbackV1, ApiReadError>,
}

fn post_merge_once_then_readback(
    api: &LocalLoopbackForgejoApi,
    pull_request_number: u64,
    body: &[u8],
) -> MergePostReadbackV1 {
    let remaining = api.deadline.saturating_duration_since(Instant::now());
    if remaining < Duration::from_millis(1) {
        return MergePostReadbackV1 {
            post_attempted: false,
            post_status: None,
            post_error: Some(ApiReadError::Unavailable),
            readback: Err(ApiReadError::Unavailable),
        };
    }
    // Deliberately one request. A transport error cannot distinguish "not
    // delivered" from "Forgejo committed before the reply was lost".
    let post = api.request_api_once(
        &api.repository_api_path(&format!(
            "/pulls/{pull_request_number}{MERGE_REQUEST_PATH_SUFFIX}"
        )),
        "POST",
        Some(body),
        Duration::from_secs(5).min(remaining),
    );
    let post_status = post.as_ref().ok().map(|response| response.status);
    let post_error = post.err();
    let readback = merged_readback(api, pull_request_number);
    MergePostReadbackV1 {
        post_attempted: true,
        post_status,
        post_error,
        readback,
    }
}

fn merge_readback_is_confirmed(
    pull_request_number: u64,
    expected_owner: &str,
    expected_repository: &str,
    submitted_head_sha: &str,
    merged_pr: &Value,
    merged_main: &Value,
    merged_issue: &Value,
) -> bool {
    let expected_base_repository = format!("{expected_owner}/{expected_repository}");
    let main_sha = merged_main
        .get("commit")
        .and_then(|commit| commit.get("id"))
        .and_then(Value::as_str);
    let merge_sha = merged_pr.get("merge_commit_sha").and_then(Value::as_str);
    merged_pr.get("number").and_then(Value::as_u64) == Some(pull_request_number)
        && merged_pr.get("merged").and_then(Value::as_bool) == Some(true)
        && merged_pr.get("state").and_then(Value::as_str) == Some("closed")
        && merged_pr
            .get("head")
            .and_then(|head| head.get("sha"))
            .and_then(Value::as_str)
            == Some(submitted_head_sha)
        && merged_pr
            .get("base")
            .and_then(|base| base.get("ref"))
            .and_then(Value::as_str)
            == Some(MERGE_PREFLIGHT_BASE_BRANCH)
        && merged_pr
            .get("base")
            .and_then(|base| base.get("repo"))
            .and_then(|repository| repository.get("full_name"))
            .and_then(Value::as_str)
            == Some(expected_base_repository.as_str())
        && merged_pr
            .get("base")
            .and_then(|base| base.get("sha"))
            .and_then(Value::as_str)
            == main_sha
        && main_sha == merge_sha
        && merged_issue.get("number").and_then(Value::as_u64) == Some(pull_request_number)
        && merged_issue.get("state").and_then(Value::as_str) == Some("closed")
}

fn merge_commit_parent_binding_is_confirmed(
    validated_base_sha: &str,
    submitted_head_sha: &str,
    merged_pr: &Value,
    merge_commit: &Value,
) -> bool {
    let merge_sha = merged_pr
        .get("merge_commit_sha")
        .and_then(Value::as_str)
        .filter(|sha| valid_full_git_object_id(sha));
    let commit_sha = merge_commit
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| valid_full_git_object_id(sha));
    let Some(parents) = merge_commit.get("parents").and_then(Value::as_array) else {
        return false;
    };
    let parent_sha = |index: usize| {
        parents
            .get(index)
            .and_then(|parent| parent.get("sha"))
            .and_then(Value::as_str)
            .filter(|sha| valid_full_git_object_id(sha))
    };

    parents.len() == 2
        && merge_sha == commit_sha
        && parent_sha(0) == Some(validated_base_sha)
        && parent_sha(1) == Some(submitted_head_sha)
}

fn merge_commit_tree_manifest_binding_sha256(
    merged_pr: &Value,
    candidate_commit: &Value,
    merge_commit: &Value,
    merge_tree_entries: Option<&[GitTreeEntryFingerprintV1]>,
    candidate_tree_entries: Option<&[GitTreeEntryFingerprintV1]>,
) -> Option<(String, String)> {
    let candidate_sha = merged_pr
        .get("head")
        .and_then(|head| head.get("sha"))
        .and_then(Value::as_str)
        .filter(|sha| valid_full_git_object_id(sha));
    let candidate_commit_sha = candidate_commit
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| valid_full_git_object_id(sha));
    let merge_commit_sha = merged_pr
        .get("merge_commit_sha")
        .and_then(Value::as_str)
        .filter(|sha| valid_full_git_object_id(sha));
    let merge_commit_object_sha = merge_commit
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| valid_full_git_object_id(sha));
    let (
        Some(candidate_sha),
        Some(candidate_commit_sha),
        Some(merge_commit_sha),
        Some(merge_commit_object_sha),
    ) = (
        candidate_sha,
        candidate_commit_sha,
        merge_commit_sha,
        merge_commit_object_sha,
    )
    else {
        return None;
    };
    let (Some(merge_tree_entries), Some(candidate_tree_entries)) =
        (merge_tree_entries, candidate_tree_entries)
    else {
        return None;
    };
    if candidate_sha != candidate_commit_sha
        || merge_commit_sha != merge_commit_object_sha
        || candidate_tree_entries != merge_tree_entries
    {
        return None;
    }
    let candidate_manifest = serde_json::to_vec(candidate_tree_entries).ok()?;
    let merge_manifest = serde_json::to_vec(merge_tree_entries).ok()?;
    Some((
        agent_forge_core::sha256(&candidate_manifest),
        agent_forge_core::sha256(&merge_manifest),
    ))
}

fn finalize_merge_post_readback(
    pull_request_number: u64,
    expected_owner: &str,
    expected_repository: &str,
    validated_base_sha: &str,
    submitted_head_sha: &str,
    post_outcome: MergePostReadbackV1,
) -> (Value, bool) {
    if !post_outcome.post_attempted {
        return merge_authority_result(
            pull_request_number,
            "inconclusive",
            false,
            Some("merge_deadline_expired_before_post"),
            true,
            None,
        );
    }
    let merge_status = post_outcome.post_status;

    // The POST is never retried. Even a transport error or 5xx may have
    // committed; resolve only by authenticated readback while still locked.
    let (
        merged_pr,
        merge_commit,
        candidate_commit,
        merged_main,
        merged_issue,
        merge_tree_entries,
        candidate_tree_entries,
    ) = match post_outcome.readback {
        Ok(readback) => readback,
        Err(error) => {
            return merge_post_readback_failure_result(pull_request_number, merge_status, error);
        }
    };
    let readback_confirmed = merge_readback_is_confirmed(
        pull_request_number,
        expected_owner,
        expected_repository,
        submitted_head_sha,
        &merged_pr,
        &merged_main,
        &merged_issue,
    ) && merge_commit_parent_binding_is_confirmed(
        validated_base_sha,
        submitted_head_sha,
        &merged_pr,
        &merge_commit,
    );
    let tree_manifest_binding = merge_commit_tree_manifest_binding_sha256(
        &merged_pr,
        &candidate_commit,
        &merge_commit,
        merge_tree_entries.as_deref(),
        candidate_tree_entries.as_deref(),
    );
    if readback_confirmed
        && let Some((candidate_tree_manifest_sha256, merge_tree_manifest_sha256)) =
            tree_manifest_binding
    {
        let (mut result, success) = merge_authority_result(
            pull_request_number,
            "merged",
            true,
            None,
            false,
            merge_status,
        );
        result["merge_commit_sha"] = merged_pr["merge_commit_sha"].clone();
        result["main_head_sha"] = merged_main["commit"]["id"].clone();
        result["issue_state"] = merged_issue["state"].clone();
        result["submitted_head_sha"] = json!(submitted_head_sha);
        result["validated_base_sha"] = json!(validated_base_sha);
        result["postmerge_base_sha"] = merged_pr["base"]["sha"].clone();
        result["merge_commit_base_parent_sha"] = merge_commit["parents"][0]["sha"].clone();
        result["merge_commit_head_parent_sha"] = merge_commit["parents"][1]["sha"].clone();
        result["reviewed_candidate_tree_entries_sha256"] = json!(candidate_tree_manifest_sha256);
        result["merge_commit_tree_entries_sha256"] = json!(merge_tree_manifest_sha256);
        if let Some(error) = post_outcome.post_error {
            let (code, _) = gate_api_failure_code(&error);
            result["post_transport_error"] = json!(code);
        }
        return (result, success);
    }
    let refusal_confirmed = merged_pr.get("merged").and_then(Value::as_bool) == Some(false)
        && merge_status.is_some_and(|status| (400..500).contains(&status));
    merge_authority_result(
        pull_request_number,
        if refusal_confirmed {
            "rejected"
        } else {
            "inconclusive"
        },
        true,
        Some(if refusal_confirmed {
            "forgejo_merge_refused"
        } else {
            "merge_outcome_not_confirmed"
        }),
        !refusal_confirmed,
        merge_status,
    )
}

/// Host decision inputs for one merge-gate invocation. The files are read only
/// after the base-branch lock is held.
struct MergeAuthorityInputsV1 {
    pull_request_number: u64,
    proof_path: PathBuf,
    expected_path: PathBuf,
    key_path: PathBuf,
    policy_path: PathBuf,
    review_binding_path: PathBuf,
    mission_admission_path: Option<PathBuf>,
    mission_journal_path: Option<PathBuf>,
    execution_identity: String,
}

struct MissionMergeAttemptContextV1 {
    admission: agent_forge_core::mission_admission::MissionHostAdmissionV1,
    scope: BaseBranchScope,
    binding: agent_forge_core::agent_runtime::MergeAttemptBindingV1,
    admission_path: PathBuf,
    journal_path: PathBuf,
    admission_sha256: String,
}

enum PreparedMissionMergeAttemptV1 {
    NewAttempt,
    RecoverOnly(Box<Result<MergeReadbackV1, ApiReadError>>),
}

/// Splits the optional `--target-profile <profile>` prefix of
/// `--merge-local-test`. Any other `--` option is refused, so a typo never
/// falls back to the default target.
fn split_merge_target_profile(
    arguments: &[std::ffi::OsString],
) -> Result<(LocalForgejoTargetProfile, &[std::ffi::OsString]), String> {
    let is_option = |argument: &std::ffi::OsString| argument.as_encoded_bytes().starts_with(b"--");
    match arguments.first() {
        Some(first) if first == MERGE_TARGET_PROFILE_FLAG => {}
        Some(first) if is_option(first) => {
            return Err(format!(
                "unknown --merge-local-test option; select a target profile only with {MERGE_TARGET_PROFILE_FLAG} <profile>"
            ));
        }
        _ => {
            return Ok((LocalForgejoTargetProfile::Merge014AgentForgeCore, arguments));
        }
    }
    let selector = arguments
        .get(1)
        .ok_or_else(|| format!("{MERGE_TARGET_PROFILE_FLAG} requires a merge target profile"))?
        .to_str()
        .ok_or_else(|| "merge target profile must be valid UTF-8".to_owned())?;
    let profile = LocalForgejoTargetProfile::parse(selector)?;
    let rest = &arguments[2..];
    match rest.first() {
        Some(next) if next == MERGE_TARGET_PROFILE_FLAG => {
            Err("a merge target profile may be selected only once".to_owned())
        }
        Some(next) if is_option(next) => Err(
            "unknown --merge-local-test option after the target profile; expected the pull request number"
                .to_owned(),
        ),
        _ => Ok((profile, rest)),
    }
}

fn parse_merge_writer_admission_profile(
    selector: &str,
) -> Result<MergeWriterAdmissionProfileV1, String> {
    match selector {
        "local_single_writer_sandbox" => {
            Ok(MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox)
        }
        "forgejo_v17_multiwriter" => Ok(MergeWriterAdmissionProfileV1::ForgejoV17MultiWriter),
        _ => Err(format!(
            "unknown writer admission profile; expected local_single_writer_sandbox or forgejo_v17_multiwriter"
        )),
    }
}

fn merge_writer_admission_profile_name(profile: MergeWriterAdmissionProfileV1) -> &'static str {
    match profile {
        MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox => "local_single_writer_sandbox",
        MergeWriterAdmissionProfileV1::ForgejoV17MultiWriter => "forgejo_v17_multiwriter",
    }
}

/// Construct a credential-bearing API with the provider bound to the exact
/// origin from the already-validated target configuration.
fn api_with_host_token(
    config: LocalForgejoConfigV1,
    host_token: &ForgejoApiToken,
) -> LocalLoopbackForgejoApi {
    let credential_origin = config.origin.clone();
    let mut api = LocalLoopbackForgejoApi::new(config);
    api.credential_provider = Some(OriginBoundHostCredentialProvider {
        origin: credential_origin,
        provider: Box::new(FixedHostForgejoCredentialProvider(
            host_token.as_bytes().to_vec(),
        )),
    });
    api
}

/// Inbox reconciliation remains anonymous unless its host launcher explicitly
/// grants a private inherited descriptor. This API is only used through GETs
/// in the reconciliation path; the merge command keeps its separate selector.
fn reconciliation_api(
    config: LocalForgejoConfigV1,
    host_token: Option<ForgejoApiToken>,
) -> LocalLoopbackForgejoApi {
    let mut api = match host_token {
        Some(token) => api_with_host_token(config, &token),
        None => LocalLoopbackForgejoApi::new(config),
    };
    api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;
    api
}

/// Scope and credential origin both come from the one selected test profile.
fn merge_api_for_profile(
    profile: LocalForgejoTargetProfile,
    host_token: &ForgejoApiToken,
) -> LocalLoopbackForgejoApi {
    let mut api = api_with_host_token(profile.config(), host_token);
    api.request_policy = LocalApiRequestPolicy::MergeAuthority;
    api
}

/// Policy credentials are confined to the privileged read endpoint Forgejo v17 exposes.
fn merge_policy_api_for_profile(
    profile: LocalForgejoTargetProfile,
    policy_token: &ForgejoApiToken,
) -> LocalLoopbackForgejoApi {
    let mut api = api_with_host_token(profile.config(), policy_token);
    api.request_policy = LocalApiRequestPolicy::MergePolicyReadOnly;
    api
}

fn run_local_test_merge_authority(
    arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(Value, bool), String> {
    let raw_arguments = arguments.collect::<Vec<_>>();
    let mut positional_arguments = Vec::with_capacity(raw_arguments.len());
    let mut writer_admission_profile = None;
    let mut supervisor_capability_fd = None;
    let mut policy_credential_fd = None;
    let mut mission_admission_path = None;
    let mut mission_journal_path = None;
    let mut index = 0;
    while index < raw_arguments.len() {
        if raw_arguments[index] == MERGE_WRITER_ADMISSION_PROFILE_FLAG {
            if writer_admission_profile.is_some() {
                return Err(format!(
                    "{MERGE_WRITER_ADMISSION_PROFILE_FLAG} may be provided only once"
                ));
            }
            let selector = raw_arguments
                .get(index + 1)
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    format!("{MERGE_WRITER_ADMISSION_PROFILE_FLAG} requires a profile")
                })?;
            writer_admission_profile = Some(parse_merge_writer_admission_profile(selector)?);
            index += 2;
        } else if raw_arguments[index] == SUPERVISOR_CAPABILITY_FD_FLAG {
            if supervisor_capability_fd.is_some() {
                return Err(format!(
                    "{SUPERVISOR_CAPABILITY_FD_FLAG} may be provided only once"
                ));
            }
            let value = raw_arguments
                .get(index + 1)
                .ok_or_else(|| format!("{SUPERVISOR_CAPABILITY_FD_FLAG} requires a descriptor"))?
                .to_str()
                .ok_or_else(|| "supervisor capability descriptor must be valid UTF-8".to_owned())?;
            supervisor_capability_fd = Some(value.parse::<RawFd>().map_err(|_| {
                "supervisor capability descriptor must be a file descriptor number".to_owned()
            })?);
            index += 2;
        } else if raw_arguments[index] == "--policy-credential-fd" {
            if policy_credential_fd.is_some() {
                return Err("--policy-credential-fd may be provided only once".to_owned());
            }
            let value = raw_arguments
                .get(index + 1)
                .ok_or_else(|| "--policy-credential-fd requires a file descriptor".to_owned())?
                .to_str()
                .ok_or_else(|| "policy credential descriptor must be valid UTF-8".to_owned())?;
            policy_credential_fd = Some(value.parse::<RawFd>().map_err(|_| {
                "policy credential descriptor must be a file descriptor number".to_owned()
            })?);
            index += 2;
        } else if raw_arguments[index] == MISSION_ADMISSION_FILE_FLAG {
            if mission_admission_path.is_some() {
                return Err(format!(
                    "{MISSION_ADMISSION_FILE_FLAG} may be provided only once"
                ));
            }
            mission_admission_path =
                Some(PathBuf::from(raw_arguments.get(index + 1).ok_or_else(
                    || format!("{MISSION_ADMISSION_FILE_FLAG} requires a path"),
                )?));
            index += 2;
        } else if raw_arguments[index] == MISSION_EXECUTION_JOURNAL_FLAG {
            if mission_journal_path.is_some() {
                return Err(format!(
                    "{MISSION_EXECUTION_JOURNAL_FLAG} may be provided only once"
                ));
            }
            mission_journal_path =
                Some(PathBuf::from(raw_arguments.get(index + 1).ok_or_else(
                    || format!("{MISSION_EXECUTION_JOURNAL_FLAG} requires a path"),
                )?));
            index += 2;
        } else {
            positional_arguments.push(raw_arguments[index].clone());
            index += 1;
        }
    }
    let writer_admission_profile =
        writer_admission_profile.unwrap_or(MergeWriterAdmissionProfileV1::ForgejoV17MultiWriter);
    // The target is fixed before any credential, input file or network access.
    let (profile, arguments) = split_merge_target_profile(&positional_arguments)?;
    match (
        profile,
        mission_admission_path.is_some(),
        mission_journal_path.is_some(),
    ) {
        (LocalForgejoTargetProfile::Mission012Clamp, true, true)
        | (LocalForgejoTargetProfile::Merge014AgentForgeCore, false, false) => {}
        (LocalForgejoTargetProfile::Mission012Clamp, _, _) => {
            return Err(format!(
                "MISSION-012 merge requires both {MISSION_ADMISSION_FILE_FLAG} and {MISSION_EXECUTION_JOURNAL_FLAG}"
            ));
        }
        (LocalForgejoTargetProfile::Merge014AgentForgeCore, _, _) => {
            return Err(
                "mission admission inputs are valid only for the mission-012-clamp target"
                    .to_owned(),
            );
        }
    }
    if arguments.len() != 8 {
        return Err(usage());
    }
    let pull_request_number = arguments[0]
        .to_str()
        .ok_or_else(usage)?
        .parse::<u64>()
        .map_err(|_| "pull request number must be a positive integer".to_owned())?;
    if pull_request_number == 0 {
        return Err("pull request number must be a positive integer".to_owned());
    }
    // Multi-writer Forgejo v17 is refused before any inherited capability,
    // merge input or credential is read. The local single-writer path accepts
    // only a live kernel-peer-bound packet from the trusted supervisor.
    let supervisor_capability = match writer_admission_profile {
        MergeWriterAdmissionProfileV1::ForgejoV17MultiWriter => {
            if let Err(error) = validate_merge_writer_admission_v1(writer_admission_profile, false)
            {
                return Ok(merge_writer_admission_refusal_result(
                    profile,
                    pull_request_number,
                    writer_admission_profile,
                    error,
                ));
            }
            None
        }
        MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox => {
            let Some(fd) = supervisor_capability_fd else {
                return Ok(supervisor_capability_refusal_result(
                    profile,
                    pull_request_number,
                    SupervisorCapabilityErrorV1::MissingPacket,
                ));
            };
            let expected_target = MergeCapabilityTargetV1 {
                profile_selector: profile.selector().to_owned(),
                origin: profile.origin().to_owned(),
                owner: profile.owner().to_owned(),
                repository: profile.repository().to_owned(),
                pull_request_number,
                base_branch: MERGE_PREFLIGHT_BASE_BRANCH.to_owned(),
            };
            match VerifiedSupervisorMergeCapabilityV1::receive_inherited_fd(fd, expected_target) {
                Ok(capability) => Some(capability),
                Err(error) => {
                    return Ok(supervisor_capability_refusal_result(
                        profile,
                        pull_request_number,
                        error,
                    ));
                }
            }
        }
    };
    let proof_path = PathBuf::from(&arguments[1]);
    let expected_path = PathBuf::from(&arguments[2]);
    let key_path = PathBuf::from(&arguments[3]);
    let policy_path = PathBuf::from(&arguments[4]);
    let review_binding_path = PathBuf::from(&arguments[5]);
    let execution_identity = arguments[6].to_str().ok_or_else(usage)?.to_owned();
    if execution_identity.trim().is_empty() || execution_identity.len() > 255 {
        return Err("execution identity must be nonempty and bounded".to_owned());
    }
    let credential_fd = arguments[7]
        .to_str()
        .ok_or_else(usage)?
        .parse::<RawFd>()
        .map_err(|_| "host credential descriptor must be a file descriptor number".to_owned())?;
    let host_token = read_inherited_credential_pipe(credential_fd)?;
    let policy_api = match policy_credential_fd {
        Some(fd) => {
            let policy_token = read_inherited_credential_pipe(fd)?;
            merge_policy_api_for_profile(profile, &policy_token)
        }
        None => merge_policy_api_for_profile(profile, &host_token),
    };

    // The credential is bound to the selected profile's origin, exactly as it
    // was bound to the single fixed origin before profiles existed. The merge
    // actor token is used for ordinary reads and the merge POST; a separate
    // policy token, when supplied, can only read the exact branch rule.
    let api = merge_api_for_profile(profile, &host_token);
    let uid = unsafe { libc::geteuid() };
    run_merge_authority_for_apis_with_writer_profile(
        profile,
        &api,
        &policy_api,
        &MergeAuthorityInputsV1 {
            pull_request_number,
            proof_path,
            expected_path,
            key_path,
            policy_path,
            review_binding_path,
            mission_admission_path,
            mission_journal_path,
            execution_identity,
        },
        &merge_lock_root_for_uid(uid),
        writer_admission_profile,
        false,
        supervisor_capability.as_ref(),
    )
}

fn supervisor_capability_refusal_result(
    profile: LocalForgejoTargetProfile,
    pull_request_number: u64,
    error: SupervisorCapabilityErrorV1,
) -> (Value, bool) {
    let (mut result, success) = merge_authority_result(
        pull_request_number,
        "inconclusive",
        false,
        Some(error.code()),
        true,
        None,
    );
    result["error"]["action"] = json!(error.action());
    result["error"]["details"] = json!({ "stage": "writer_admission" });
    result["target"] = json!({
        "selector": profile.selector(),
        "origin": profile.origin(),
        "owner": profile.owner(),
        "repository": profile.repository(),
        "base_branch": MERGE_PREFLIGHT_BASE_BRANCH,
    });
    (result, success)
}

/// Gate body for one already-bound target, with that target attached to every
/// structured result. The repository must be the profile's own; the origin
/// comes from the API, which `merge_api_for_profile` binds to the profile.
#[cfg(test)]
fn run_merge_authority_for_api(
    profile: LocalForgejoTargetProfile,
    api: &LocalLoopbackForgejoApi,
    inputs: &MergeAuthorityInputsV1,
    lock_root: &Path,
) -> Result<(Value, bool), String> {
    run_merge_authority_for_apis(profile, api, api, inputs, lock_root)
}

#[cfg(test)]
fn run_merge_authority_for_apis(
    profile: LocalForgejoTargetProfile,
    api: &LocalLoopbackForgejoApi,
    policy_api: &LocalLoopbackForgejoApi,
    inputs: &MergeAuthorityInputsV1,
    lock_root: &Path,
) -> Result<(Value, bool), String> {
    run_merge_authority_for_apis_with_writer_profile(
        profile,
        api,
        policy_api,
        inputs,
        lock_root,
        MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox,
        true,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_merge_authority_for_apis_with_writer_profile(
    profile: LocalForgejoTargetProfile,
    api: &LocalLoopbackForgejoApi,
    policy_api: &LocalLoopbackForgejoApi,
    inputs: &MergeAuthorityInputsV1,
    lock_root: &Path,
    writer_profile: MergeWriterAdmissionProfileV1,
    host_attests_single_writer_exclusion: bool,
    supervisor_capability: Option<&VerifiedSupervisorMergeCapabilityV1>,
) -> Result<(Value, bool), String> {
    if api.config.owner != profile.owner() || api.config.repository != profile.repository() {
        return Err("merge API scope does not match the selected target profile".to_owned());
    }
    if policy_api.config.origin != api.config.origin
        || policy_api.config.owner != api.config.owner
        || policy_api.config.repository != api.config.repository
    {
        return Err("policy API scope does not match the merge API target".to_owned());
    }
    if api
        .credential_provider
        .as_ref()
        .is_some_and(|provider| provider.origin != api.config.origin)
    {
        return Err("host credential origin does not match the merge API origin".to_owned());
    }
    if policy_api
        .credential_provider
        .as_ref()
        .is_some_and(|provider| provider.origin != policy_api.config.origin)
    {
        return Err("policy credential origin does not match the policy API origin".to_owned());
    }
    if !cfg!(test) && api.config.origin != profile.origin() {
        return Err("merge API origin does not match the selected target profile".to_owned());
    }
    #[cfg(test)]
    let host_attested = supervisor_capability.is_some() || host_attests_single_writer_exclusion;
    #[cfg(not(test))]
    let host_attested = {
        let _ = host_attests_single_writer_exclusion;
        supervisor_capability.is_some()
    };
    if let Err(error) = validate_merge_writer_admission_v1(writer_profile, host_attested) {
        return Ok(merge_writer_admission_refusal_result(
            profile,
            inputs.pull_request_number,
            writer_profile,
            error,
        ));
    }
    let (mut result, success) = merge_authority_decision(
        profile,
        api,
        policy_api,
        inputs,
        lock_root,
        supervisor_capability,
    )?;
    if let Some(capability) = supervisor_capability {
        result["writer_admission"] = capability.audit_summary();
        result["writer_admission"]["profile"] =
            json!(merge_writer_admission_profile_name(writer_profile));
        result["external_writer_exclusion"] =
            json!("trusted_supervisor_policy_not_independently_verified");
    }
    result["target"] = json!({
        "selector": profile.selector(),
        "origin": api.config.origin,
        "owner": api.config.owner,
        "repository": api.config.repository,
        "base_branch": MERGE_PREFLIGHT_BASE_BRANCH,
    });
    Ok((result, success))
}

/// The lock scope, PR snapshot, reviews, protection, base checks, merge POST
/// and readback all derive from `api.config`; nothing below names a fixed
/// origin, owner or repository.
fn merge_authority_decision(
    profile: LocalForgejoTargetProfile,
    api: &LocalLoopbackForgejoApi,
    policy_api: &LocalLoopbackForgejoApi,
    inputs: &MergeAuthorityInputsV1,
    lock_root: &Path,
    supervisor_capability: Option<&VerifiedSupervisorMergeCapabilityV1>,
) -> Result<(Value, bool), String> {
    let pull_request_number = inputs.pull_request_number;
    let config = &api.config;
    let scope = BaseBranchScope::new(
        &config.origin,
        &config.owner,
        &config.repository,
        MERGE_PREFLIGHT_BASE_BRANCH,
    )
    .map_err(|_| "invalid local merge scope".to_owned())?;
    let _lock = match BaseBranchMergeLock::acquire(lock_root, &scope, MERGE_LOCK_WAIT) {
        Ok(lock) => lock,
        Err(error) => {
            let (code, retryable) = match &error {
                MergeLockError::TimedOut => ("merge_lock_timeout", true),
                MergeLockError::InvalidScope => ("invalid_merge_scope", false),
                MergeLockError::UntrustedLockDirectory => ("untrusted_merge_lock_directory", false),
                MergeLockError::UntrustedLockFile => ("untrusted_merge_lock_file", false),
                MergeLockError::Io(_) => ("merge_lock_io_failure", false),
            };
            return Ok(merge_authority_result(
                pull_request_number,
                "inconclusive",
                false,
                Some(code),
                retryable,
                None,
            ));
        }
    };

    // Load every host decision input only after acquiring the branch lock so
    // the protected proof/policy/review-binding read is in the serialized
    // final gate window too.
    let expected: ExpectedBinding = serde_json::from_slice(&read_adapter_input(
        &inputs.expected_path,
        MAX_EXPECTED_BINDING_BYTES,
        true,
    )?)
    .map_err(|_| "expected binding must be valid private JSON for schema v1".to_owned())?;
    let proof_bytes = read_adapter_input(&inputs.proof_path, MAX_PROOF_BYTES, true)?;
    let proof_value: Value = serde_json::from_slice(&proof_bytes)
        .map_err(|_| "target-bound proof must be valid private JSON".to_owned())?;
    let proof_schema = proof_value.get("schema_version").and_then(Value::as_u64);
    if proof_schema != Some(2) {
        let code = match proof_value.as_object() {
            None => "invalid_target_bound_proof",
            Some(object) => match object.get("schema_version") {
                None if object.contains_key("evidence") && object.contains_key("mac_sha256") => {
                    "target_bound_proof_required"
                }
                Some(Value::Number(version)) if version.as_u64().is_some() => {
                    "unsupported_target_bound_proof_version"
                }
                _ => "invalid_target_bound_proof",
            },
        };
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some(code),
            false,
            None,
        ));
    }
    let proof: TargetBoundProofV2 = match serde_json::from_value(proof_value) {
        Ok(proof) => proof,
        Err(_) => {
            return Ok(merge_authority_result(
                pull_request_number,
                "rejected",
                false,
                Some("invalid_target_bound_proof"),
                false,
                None,
            ));
        }
    };
    let verifier_key = read_adapter_input(&inputs.key_path, MAX_VERIFIER_KEY_BYTES, true)?;
    let expected_target = ProofTargetV2::new(
        profile.selector(),
        &config.origin,
        &config.owner,
        &config.repository,
        MERGE_PREFLIGHT_BASE_BRANCH,
    )
    .map_err(|_| "invalid compiled target profile".to_owned())?;
    if let Err(error) = agent_forge_core::verify_target_bound_proof_v2(
        Some(&proof),
        &verifier_key,
        &expected_target,
        &expected,
    ) {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some(target_proof_error_code(&error)),
            false,
            None,
        ));
    }
    let policy_bytes = read_adapter_input(&inputs.policy_path, MAX_POLICY_BYTES, true)?;
    let policy: Policy = serde_json::from_slice(&policy_bytes)
        .map_err(|_| "policy must be valid private host policy JSON".to_owned())?;
    let review_binding_bytes = read_adapter_input(
        &inputs.review_binding_path,
        MAX_REVIEW_BASE_BINDING_BYTES,
        true,
    )?;
    let host_binding: HostReviewBaseBindingFileV1 =
        serde_json::from_slice(&review_binding_bytes)
            .map_err(|_| "review base binding must be valid private host JSON".to_owned())?;
    let mission_admission_evidence = match verify_mission_admission_binding(
        profile,
        &config.origin,
        &config.owner,
        &config.repository,
        &expected,
        &host_binding,
        inputs,
    ) {
        Ok(evidence) => evidence,
        Err(error) => {
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("mission_admission_invalid");
            let mut result = merge_authority_result(
                pull_request_number,
                "rejected",
                false,
                Some(code),
                false,
                None,
            )
            .0;
            result["error"] = error;
            return Ok((result, false));
        }
    };

    let mission_merge_context = if mission_admission_evidence.is_null() {
        None
    } else {
        let (Some(admission_path), Some(journal_path)) = (
            inputs.mission_admission_path.as_deref(),
            inputs.mission_journal_path.as_deref(),
        ) else {
            return Ok(merge_authority_result(
                pull_request_number,
                "rejected",
                false,
                Some("mission_admission_missing"),
                false,
                None,
            ));
        };
        let admission_bytes =
            match read_adapter_input(admission_path, MAX_MISSION_ADMISSION_BYTES, true) {
                Ok(bytes) => bytes,
                Err(_) => {
                    return Ok(merge_authority_result(
                        pull_request_number,
                        "rejected",
                        false,
                        Some("mission_admission_unavailable"),
                        false,
                        None,
                    ));
                }
            };
        let admission_sha256 = agent_forge_core::sha256(&admission_bytes);
        if mission_admission_evidence
            .get("admission_sha256")
            .and_then(Value::as_str)
            != Some(admission_sha256.as_str())
        {
            return Ok(merge_authority_result(
                pull_request_number,
                "rejected",
                false,
                Some("mission_admission_binding_changed"),
                false,
                None,
            ));
        }
        let admission: agent_forge_core::mission_admission::MissionHostAdmissionV1 =
            match serde_json::from_slice::<
                agent_forge_core::mission_admission::MissionHostAdmissionV1,
            >(&admission_bytes)
            {
                Ok(admission) if admission.validate().is_ok() => admission,
                _ => {
                    return Ok(merge_authority_result(
                        pull_request_number,
                        "rejected",
                        false,
                        Some("mission_admission_invalid"),
                        false,
                        None,
                    ));
                }
            };
        let mission_scope = match BaseBranchScope::new(
            &admission.scope.origin,
            &admission.scope.owner,
            &admission.scope.repository,
            &admission.scope.branch,
        ) {
            Ok(scope) => scope,
            Err(_) => {
                return Ok(merge_authority_result(
                    pull_request_number,
                    "rejected",
                    false,
                    Some("mission_admission_scope_invalid"),
                    false,
                    None,
                ));
            }
        };
        let request_sha256 = agent_forge_core::sha256(
            &serde_json::to_vec(&admission.request)
                .map_err(|_| "mission admission request could not be serialized".to_owned())?,
        );
        let binding = match agent_forge_core::agent_runtime::MergeAttemptBindingV1::new(
            &mission_scope,
            admission.issue_number,
            pull_request_number,
            &expected.base_commit,
            &expected.candidate_commit,
            &expected.requirement_sha256,
            &agent_forge_core::sha256(&policy_bytes),
            &agent_forge_core::sha256(&proof_bytes),
            &agent_forge_core::sha256(&review_binding_bytes),
            &request_sha256,
            &admission_sha256,
        ) {
            Ok(binding) => binding,
            Err(error) => {
                return Ok(merge_attempt_runtime_error_result(
                    pull_request_number,
                    &error,
                    false,
                ));
            }
        };
        let context = MissionMergeAttemptContextV1 {
            admission,
            scope: mission_scope,
            binding,
            admission_path: admission_path.to_owned(),
            journal_path: journal_path.to_owned(),
            admission_sha256,
        };
        if let Err(error) = revalidate_mission_merge_inputs(profile, inputs, &context) {
            let (mut result, success) =
                merge_attempt_runtime_error_result(pull_request_number, &error, false);
            result["mission_admission"] = mission_admission_evidence.clone();
            return Ok((result, success));
        }
        let journal =
            agent_forge_core::agent_runtime::MissionExecutionJournalV1::open(&context.journal_path);
        let mut recovered_result = None;
        let recovery = journal.recover_merge_attempt_if_present(
            &context.scope,
            context.admission.issue_number,
            &context.admission.allocation.allocation_id,
            context.admission.allocation.execution_generation,
            &context.admission.request,
            context.binding.clone(),
            |verification| {
                revalidate_mission_merge_inputs(profile, inputs, &context)?;
                if verification.request_sha256
                    != mission_admission_evidence["request_sha256"]
                        .as_str()
                        .unwrap_or_default()
                {
                    return Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                        "merge_attempt_binding_mismatch",
                        false,
                        "restore the admission request recorded in the durable merge intent",
                        json!({}),
                    ));
                }
                let (result, success, resolution) = finalize_readback_only(
                    pull_request_number,
                    &api.config.owner,
                    &api.config.repository,
                    &context.binding.base_commit,
                    &context.binding.candidate_commit,
                    merged_readback(api, pull_request_number),
                );
                recovered_result = Some((result, success));
                Ok(resolution)
            },
        );
        match recovery {
            Ok(Some(resolution)) => {
                let (mut result, success) = recovered_result.unwrap_or_else(|| {
                    merge_attempt_resolution_result(pull_request_number, &resolution)
                });
                result["mission_admission"] = mission_admission_evidence.clone();
                result["merge_attempt_recovery"] = json!(true);
                result["merge_attempted"] = json!(false);
                result["merge_post_attempt_status"] = json!("unknown");
                return Ok((result, success));
            }
            Ok(None) => {}
            Err(error) => {
                let (mut result, success) =
                    merge_attempt_runtime_error_result(pull_request_number, &error, false);
                result["mission_admission"] = mission_admission_evidence.clone();
                return Ok((result, success));
            }
        }
        Some(context)
    };

    let resource = ResourceRef {
        kind: ResourceKind::PullRequest,
        owner: config.owner.clone(),
        repository: config.repository.clone(),
        id: ResourceId::Number(pull_request_number),
    };
    let current_json = match api.read_current(&resource) {
        Ok(current) => current,
        Err(error) => {
            return Ok(merge_authority_api_error_result(
                pull_request_number,
                "pull-request-read",
                &error,
            ));
        }
    };
    let current = match PullRequestSnapshotV1::from_forgejo_response(
        &config.origin,
        &config.owner,
        &config.repository,
        pull_request_number,
        &current_json,
    ) {
        Ok(current) => current,
        Err(_error) => {
            return Ok(merge_authority_result(
                pull_request_number,
                "rejected",
                false,
                Some("invalid_or_wrong_scope_pull_request"),
                false,
                None,
            ));
        }
    };
    if current.draft {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("draft_pull_request"),
            false,
            None,
        ));
    }
    let context = MergePreflightContextV1 {
        pull_request_number,
        execution_identity: inputs.execution_identity.clone(),
        expected,
    };
    let api_actor = match gate_api_json(api, "/user") {
        Ok(value) => value,
        Err(error) => {
            return Ok(merge_authority_api_error_result(
                pull_request_number,
                "authenticated-actor-read",
                &error,
            ));
        }
    };
    let actor_id = api_actor.get("id").and_then(Value::as_u64).unwrap_or(0);
    let actor_login = api_actor
        .get("login")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if actor_id == 0 || actor_login.is_empty() || actor_login.len() > 255 {
        return Ok(merge_authority_result(
            pull_request_number,
            "inconclusive",
            false,
            Some("authenticated_actor_invalid"),
            false,
            None,
        ));
    }

    let review_scope = PullRequestReviewScopeV1::new(
        &config.origin,
        &config.owner,
        &config.repository,
        pull_request_number,
    )
    .map_err(|_| "invalid review scope".to_owned())?;
    let reviews = match forgejo_review_pages(api, &review_scope) {
        Ok(reviews) => reviews,
        Err(error) => {
            return Ok(merge_authority_api_error_result(
                pull_request_number,
                "review-list-read",
                &error,
            ));
        }
    };
    let Some(approval_policy) = policy.merge_approval_policy.as_ref() else {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("merge_approval_policy_missing"),
            false,
            None,
        ));
    };
    let protection = match gate_api_json(
        policy_api,
        &policy_api.repository_api_path("/branch_protections"),
    ) {
        Ok(protection) => protection,
        Err(error) => {
            return Ok(merge_authority_api_error_result(
                pull_request_number,
                "branch-protection-read",
                &error,
            ));
        }
    };
    let protection_users = protection
        .as_array()
        .and_then(|rules| {
            let matching = rules
                .iter()
                .filter(|item| item.get("rule_name").and_then(Value::as_str) == Some("main"))
                .collect::<Vec<_>>();
            (matching.len() == 1).then(|| {
                matching[0]
                    .get("approvals_whitelist_username")
                    .and_then(Value::as_array)
            })?
        })
        .and_then(|users| users.iter().map(Value::as_str).collect::<Option<Vec<_>>>());
    let Some(protection_users) = protection_users else {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("invalid_approval_whitelist"),
            false,
            None,
        ));
    };
    if protection_users.len() != approval_policy.authorized_reviewer_ids.len() {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("approval_whitelist_policy_mismatch"),
            false,
            None,
        ));
    }
    let mut reviewer_identities = Vec::with_capacity(protection_users.len());
    for login in protection_users {
        match forgejo_user_by_login(api, login) {
            Ok(identity) => reviewer_identities.push(identity),
            Err(error) => {
                return Ok(merge_authority_api_error_result(
                    pull_request_number,
                    "reviewer-identity-read",
                    &error,
                ));
            }
        }
    }
    let mut actual_reviewer_ids = reviewer_identities
        .iter()
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    let mut allowed_reviewer_ids = approval_policy.authorized_reviewer_ids.clone();
    actual_reviewer_ids.sort_unstable();
    allowed_reviewer_ids.sort_unstable();
    if actual_reviewer_ids != allowed_reviewer_ids {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("approval_whitelist_policy_mismatch"),
            false,
            None,
        ));
    }
    let reviewer_logins = reviewer_identities
        .iter()
        .map(|(_, login)| login.clone())
        .collect::<Vec<_>>();
    let validated_branch_protection_rule = match validate_local_test_branch_protection(
        &protection,
        &policy,
        &reviewer_logins,
        actor_login,
    ) {
        Ok(rule) => rule.clone(),
        Err(code) => {
            return Ok(merge_authority_result(
                pull_request_number,
                "rejected",
                false,
                Some(code),
                false,
                None,
            ));
        }
    };

    let main = match gate_api_json(
        api,
        &api.repository_api_path(&format!("/branches/{MERGE_PREFLIGHT_BASE_BRANCH}")),
    ) {
        Ok(main) => main,
        Err(error) => {
            return Ok(merge_authority_api_error_result(
                pull_request_number,
                "base-branch-read",
                &error,
            ));
        }
    };
    let main_sha = main
        .get("commit")
        .and_then(|commit| commit.get("id"))
        .and_then(Value::as_str);
    if main_sha != Some(current.base_sha.as_str()) {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("stale_base"),
            false,
            None,
        ));
    }

    if host_binding.schema_version != 1
        || host_binding.review_id == 0
        || host_binding.reviewer_id == 0
        || host_binding.pull_request_number != pull_request_number
        || host_binding.base_branch != current.base_branch
        || host_binding.base_commit != current.base_sha
        || host_binding.candidate_commit != current.head_sha
        || host_binding.source.trim().is_empty()
    {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("invalid_review_base_binding"),
            false,
            None,
        ));
    }
    let review_bindings = PullRequestReviewBaseBindingsSnapshotV1 {
        scope: review_scope.clone(),
        bindings: vec![PullRequestReviewBaseBindingV1 {
            review_id: host_binding.review_id,
            base_branch: host_binding.base_branch.clone(),
            base_commit: host_binding.base_commit.clone(),
        }],
    };
    let readiness = match evaluate_target_bound_merge_readiness_v2(
        &scope,
        &current,
        &reviews,
        &review_bindings,
        Some(&proof),
        &verifier_key,
        &expected_target,
        &context,
        actor_id,
        &policy_bytes,
    ) {
        Ok(readiness) => readiness,
        Err(error) => {
            let code = match error {
                MergeReadinessError::Preflight(_) => "preflight_rejected",
                MergeReadinessError::Approval(MergeApprovalError::InsufficientApprovals {
                    ..
                }) => "review_approval_missing",
                MergeReadinessError::Approval(MergeApprovalError::StalePolicy) => "stale_policy",
                MergeReadinessError::Approval(MergeApprovalError::StaleReviewBaseBinding) => {
                    "stale_review_base_binding"
                }
                MergeReadinessError::Approval(MergeApprovalError::MissingReviewBaseBinding) => {
                    "missing_review_base_binding"
                }
                MergeReadinessError::Approval(_) => "review_policy_rejected",
            };
            return Ok(merge_authority_result(
                pull_request_number,
                "rejected",
                false,
                Some(code),
                false,
                None,
            ));
        }
    };
    let host_review = reviews
        .reviews
        .iter()
        .find(|review| review.id == host_binding.review_id);
    if host_review.and_then(|review| review.user.as_ref().map(|user| user.id))
        != Some(host_binding.reviewer_id)
        || !readiness
            .approved_review_ids
            .contains(&host_binding.review_id)
    {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("review_base_binding_reviewer_mismatch"),
            false,
            None,
        ));
    }
    let bound_reviewer_login = reviewer_identities
        .iter()
        .find(|(reviewer_id, _)| *reviewer_id == host_binding.reviewer_id)
        .map(|(_, login)| login.as_str());
    if bound_reviewer_login != Some(host_binding.reviewer_login.as_str()) {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("review_base_binding_login_mismatch"),
            false,
            None,
        ));
    }

    if mission_admission_evidence.is_null() {
        return Ok(run_final_merge_window(
            api,
            policy_api,
            &current,
            &validated_branch_protection_rule,
            pull_request_number,
            &mission_admission_evidence,
            || validate_supervisor_capability_before_post(supervisor_capability),
        ));
    }

    let Some(context) = mission_merge_context else {
        return Ok(merge_authority_result(
            pull_request_number,
            "rejected",
            false,
            Some("mission_admission_missing"),
            false,
            None,
        ));
    };
    let execution_journal =
        agent_forge_core::agent_runtime::MissionExecutionJournalV1::open(&context.journal_path);
    let mut merge_result = None;
    let mut recovered_during_gate = false;
    let locked_result = execution_journal.with_merge_attempt(
        &context.scope,
        context.admission.issue_number,
        &context.admission.allocation.allocation_id,
        context.admission.allocation.execution_generation,
        &context.admission.request,
        context.binding.clone(),
        |mode, _verification| {
            revalidate_mission_merge_inputs(profile, inputs, &context)?;
            match mode {
                agent_forge_core::agent_runtime::MergeAttemptModeV1::NewAttempt => {
                    validate_final_merge_state(
                        api,
                        policy_api,
                        &current,
                        &validated_branch_protection_rule,
                    )
                    .map_err(final_merge_state_runtime_error)?;
                    Ok(PreparedMissionMergeAttemptV1::NewAttempt)
                }
                agent_forge_core::agent_runtime::MergeAttemptModeV1::RecoverOnly => {
                    recovered_during_gate = true;
                    Ok(PreparedMissionMergeAttemptV1::RecoverOnly(Box::new(
                        merged_readback(api, pull_request_number),
                    )))
                }
            }
        },
        |mode, _verification, prepared| {
            let (result, success, resolution) = match (mode, prepared) {
                (
                    agent_forge_core::agent_runtime::MergeAttemptModeV1::NewAttempt,
                    PreparedMissionMergeAttemptV1::NewAttempt,
                ) => run_final_merge_window_with_resolution(
                    api,
                    policy_api,
                    &current,
                    &validated_branch_protection_rule,
                    pull_request_number,
                    &mission_admission_evidence,
                    || {
                        revalidate_mission_merge_inputs(profile, inputs, &context)?;
                        validate_supervisor_capability_before_post(supervisor_capability)
                    },
                ),
                (
                    agent_forge_core::agent_runtime::MergeAttemptModeV1::RecoverOnly,
                    PreparedMissionMergeAttemptV1::RecoverOnly(readback),
                ) => finalize_readback_only(
                    pull_request_number,
                    &api.config.owner,
                    &api.config.repository,
                    &context.binding.base_commit,
                    &context.binding.candidate_commit,
                    *readback,
                ),
                _ => {
                    return Err(agent_forge_core::agent_runtime::AgentRuntimeErrorV1::new(
                        "merge_attempt_mode_mismatch",
                        false,
                        "reload the durable merge attempt before performing any network effect",
                        json!({}),
                    ));
                }
            };
            merge_result = Some((result, success));
            Ok(resolution)
        },
    );
    match locked_result {
        Ok(resolution) => {
            let (mut result, success) = merge_result.unwrap_or_else(|| {
                merge_attempt_resolution_result(pull_request_number, &resolution)
            });
            result["mission_admission"] = mission_admission_evidence.clone();
            if recovered_during_gate {
                result["merge_attempt_recovery"] = json!(true);
                result["merge_attempted"] = json!(false);
                result["merge_post_attempt_status"] = json!("unknown");
            }
            Ok((result, success))
        }
        Err(error) => {
            let (mut result, success) =
                merge_attempt_runtime_error_result(pull_request_number, &error, false);
            result["mission_admission"] = mission_admission_evidence.clone();
            Ok((result, success))
        }
    }
}

fn target_proof_error_code(error: &GateError) -> &'static str {
    match error {
        GateError::MissingProof => "target_bound_proof_required",
        GateError::InvalidSignature => "target_proof_signature_invalid",
        GateError::InvalidVerifierKey => "invalid_verifier_key",
        GateError::InvalidTarget => "invalid_target_proof_target",
        GateError::WrongTarget => "target_proof_target_mismatch",
        GateError::UnsupportedProofVersion => "unsupported_target_bound_proof_version",
        GateError::StaleBinding("requirements") => "stale_requirements",
        GateError::StaleBinding("policy") => "stale_policy",
        GateError::StaleBinding("candidate") => "stale_candidate",
        GateError::StaleBinding("base") => "stale_base",
        GateError::StaleBinding("merge result") => "stale_merge_result",
        GateError::StaleBinding("verifier identity") => "stale_verifier_identity",
        GateError::StaleBinding(_) => "stale_proof_binding",
        GateError::OutcomeFailed => "verification_failed",
        GateError::OutcomeInconclusive => "verification_inconclusive",
    }
}

/// Static discovery built only from compiled-in constants: it reads no
/// configuration, journal or Forgejo state, so it needs no disk or network.
fn capabilities_document() -> Value {
    let retry_policy = LocalApiRetryPolicy::default();
    let api_path = format!("/api/v1/repos/{LOCAL_REPOSITORY_OWNER}/{LOCAL_REPOSITORY_NAME}");
    let event_contracts_v5 = serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V5)
        .expect("static webhook event contract v5 serializes");
    let event_contracts_v6 = serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V6)
        .expect("static webhook event contract v6 serializes");
    let event_contracts_v7 = serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V7)
        .expect("static webhook event contract v7 serializes");
    let event_contracts_v8 = serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V8)
        .expect("static webhook event contract v8 serializes");
    let event_contracts_v9 = serde_json::to_value(WEBHOOK_EVENT_CONTRACTS_V9)
        .expect("static webhook event contract v9 serializes");
    let mut reconcile = json!({
        "id": "reconcile_webhook_deliveries",
        "target_profile_selector": MERGE_TARGET_PROFILE_FLAG,
        "reconcile_only_profile_selector": RECONCILE_ONLY_TARGET_PROFILE_FLAG,
        "reconcile_only_profiles": [
            {
                "selector": ReconcileOnlyTargetProfile::OrgTeamReviewE2E.selector(),
                "origin": LOCAL_FORGEJO_ORIGIN,
                "owner": "forge-team-org",
                "repository": "team-review-readback",
                "merge_target": false,
                "forgejo_writes": false,
            },
        ],
        "target_profiles": [
            {
                "selector": LocalForgejoTargetProfile::Merge014AgentForgeCore.selector(),
                "origin": LocalForgejoTargetProfile::Merge014AgentForgeCore.origin(),
                "owner": LocalForgejoTargetProfile::Merge014AgentForgeCore.owner(),
                "repository": LocalForgejoTargetProfile::Merge014AgentForgeCore.repository(),
            },
            {
                "selector": LocalForgejoTargetProfile::Mission012Clamp.selector(),
                "origin": LocalForgejoTargetProfile::Mission012Clamp.origin(),
                "owner": LocalForgejoTargetProfile::Mission012Clamp.owner(),
                "repository": LocalForgejoTargetProfile::Mission012Clamp.repository(),
            },
        ],
        "supported_events": SUPPORTED_EVENTS,
        "event_contracts": WEBHOOK_EVENT_CONTRACTS_V1,
        "event_contracts_v2": WEBHOOK_EVENT_CONTRACTS_V2,
        "event_contracts_v3": WEBHOOK_EVENT_CONTRACTS_V3,
        "event_contracts_v4": WEBHOOK_EVENT_CONTRACTS_V4,
        "source_event_provenance_contract": source_event_provenance_contract(),
        "maximum_events": MAX_RECONCILE_BATCH_EVENTS,
        "maximum_cached_json_bytes": MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
        "cache_budget_measurement": "combined_uncompressed_serialized_json",
        "maximum_projected_output_bytes": MAX_RECONCILE_BATCH_OUTPUT_BYTES,
        "output_budget_measurement": "serialized_canonical_state_json_plus_2048_bytes_per_delivery_for_cli_envelope",
        "inbox_journal_capacity": {
            "maximum_bytes": MAX_WEBHOOK_INBOX_BYTES,
            "overflow_policy": "reject_without_truncation",
        },
        "forgejo_reads": [
            format!("{api_path}/issues/{{number}}"),
            format!("{api_path}/pulls/{{number}}"),
        ],
        "current_api_identity_contract": {
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
        },
        "credential_source": "optional_inherited_anonymous_pipe_fd_bound_to_configured_origin",
        "credential_contract": {
            "credential_flag": RECONCILE_HOST_CREDENTIAL_FD_FLAG,
            "source": "optional_inherited_anonymous_pipe_fd_bound_to_configured_origin",
            "trusted_attestation_public_key_flag": TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG,
            "maximum_trusted_attestation_public_keys": MAX_TRUSTED_WEBHOOK_ATTESTATION_KEYS,
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
        },
        "local_reads": ["optional_config_file", "optional_target_profile_selector", "inbox_journal"],
        // Replay may create missing parent directories, takes the journal lock
        // and may repair an unterminated final record; it never appends an event.
        "local_writes": [
            "inbox_journal_missing_parent_directories",
            "inbox_journal_lock_file",
            "inbox_journal_unterminated_tail_repair",
        ],
        "journal_event_appends": false,
        // Replay waits for the shared journal lock for a bounded interval
        // before any Forgejo read, so this wait is outside the read budget.
        "inbox_journal_lock": {
            "file": "<inbox>.lock",
            "wait": "bounded",
            "wait_timeout_ms": EVENT_JOURNAL_LOCK_WAIT_MS,
            "counts_toward_read_budget": false,
        },
        "forgejo_writes": false,
        "canonical_state_writes": false,
        "output_schema_versions": {
            "single": RECONCILE_SINGLE_SCHEMA_VERSION,
            "batch": RECONCILE_BATCH_SCHEMA_VERSION,
        },
        "workflow_job_parent_run_projection": {
            "resource": "workflow_job",
            "source": "second_current_forgejo_api_read",
            "projected_fields": ["run_id", "index_in_repo", "state", "updated_at", "html_url", "head_sha"],
            "state_semantics": "canonical_state.state_is_job_state; canonical_state.parent_workflow_run.state_is_parent_run_state",
            "read_failure": "inconclusive_without_canonical_state",
            "independent_verification": false,
            "merge_authorizing": false,
        },
        "pull_request_projection": {
            "draft": {
                "source": "current_forgejo_pull_request_api",
                "projected_fields": ["draft"],
                "required_type": "boolean",
                "missing_or_malformed": "inconclusive_invalid_current_state",
                "freshness": "current_api_read",
                "webhook_draft": "never_used_as_canonical_state",
                "merge_authorizing": false,
            },
            "requested_reviewers": {
                "source": "current_forgejo_pull_request_api",
                "projected_fields": ["id", "login"],
                "ordering": "ascending_user_id",
                "maximum_items": MAX_RECONCILED_REQUESTED_REVIEWERS,
                "missing_or_null": "null_unknown",
                "empty_array": "known_empty",
                "malformed_or_duplicate_id": "inconclusive_invalid_current_state",
            },
            "requested_reviewers_teams": {
                "source": "current_forgejo_pull_request_api",
                "projected_fields": ["id", "name"],
                "ordering": "ascending_team_id",
                "maximum_items": MAX_RECONCILED_REQUESTED_REVIEWER_TEAMS,
                "maximum_name_characters": MAX_RECONCILED_REVIEWER_TEAM_NAME_CHARS,
                "missing_or_null": "null_unknown",
                "empty_array": "known_empty",
                "malformed_or_duplicate_id": "inconclusive_invalid_current_state",
            },
        },
        "work_item_content_projection": {
            "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
            "source": "current_forgejo_resource_api",
            "projected_fields": ["title", "body"],
            "maximum_title_bytes": MAX_WORK_ITEM_TITLE_BYTES,
            "maximum_body_bytes": MAX_WORK_ITEM_BODY_BYTES,
            "missing_title_or_body": "null_unknown",
            "explicit_null_body": "known_null",
            "over_limit_content": "omit_content_and_keep_canonical_state",
            "malformed_field_type_or_title": "inconclusive_invalid_current_state",
            "trust": "untrusted_user_content",
            "webhook_content": "never_used_as_canonical_context",
        },
        "work_item_labels_projection": {
            "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
            "source": "current_forgejo_resource_api",
            "projected_fields": ["id", "name"],
            "ordering": "ascending_label_id",
            "maximum_items": MAX_RECONCILED_LABELS,
            "maximum_name_bytes": MAX_RECONCILED_LABEL_NAME_BYTES,
            "missing_or_null": "null_unknown",
            "empty_array": "known_empty",
            "malformed_duplicate_or_over_limit": "inconclusive_invalid_current_state",
            "freshness": "current_api_read",
            "trust": "untrusted_user_content",
            "webhook_labels": "never_used_as_canonical_context",
        },
        "work_item_assignees_projection": {
            "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
            "source": "current_forgejo_resource_api",
            "projected_fields": ["id", "login"],
            "ordering": "ascending_user_id",
            "maximum_items": MAX_RECONCILED_ASSIGNEES,
            "maximum_login_bytes": MAX_RECONCILED_ASSIGNEE_LOGIN_BYTES,
            "missing_field": "null_unknown",
            "null_by_resource": {
                "issue": "known_empty",
                "issue_comment_parent": "known_empty",
                "pull_request": "null_unknown",
                "pull_request_comment_parent": "null_unknown",
            },
            "empty_array": "known_empty",
            "malformed_duplicate_or_over_limit": "inconclusive_invalid_current_state",
            "freshness": "current_api_read",
            "trust": "untrusted_forgejo_identity",
            "webhook_assignees": "never_used_as_canonical_context",
        },
        "work_item_milestone_projection": {
            "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
            "source": "current_forgejo_resource_api",
            "observations": {
                "missing_field": "unknown",
                "null": "unassigned",
                "object": "assigned",
            },
            "projected_fields": ["id", "title", "state"],
            "maximum_title_bytes": MAX_RECONCILED_MILESTONE_TITLE_BYTES,
            "malformed_or_over_limit": "inconclusive_invalid_current_state",
            "freshness": "current_api_read",
            "trust": "untrusted_user_content",
            "webhook_milestone": "never_used_as_canonical_context",
        },
        "review_notification_projection": {
            "source": "forgejo_webhook_delivery",
            "projected_fields": [
                "review_type",
                "body",
                "body_truncated",
                "body_byte_length",
                "body_sha256",
                "pull_request_head_sha_at_event",
            ],
            "maximum_body_bytes": MAX_REVIEW_NOTIFICATION_BODY_BYTES,
            "oversized_body": "truncate_at_utf8_boundary",
            "body_byte_length_and_sha256": "complete_source_body_before_truncation",
            "body_sha256_semantics": "identity_only_not_correctness_or_approval",
            "trust": "untrusted_user_content",
            "freshness": "event_snapshot_not_canonical",
            "review_id_binding": "unavailable_in_forgejo_v17_webhook",
            "authorizing": false,
        },
        "release_projection": {
            "source": "current_forgejo_resource_api",
            "canonical_read": format!("{api_path}/releases/{{release_id}}"),
            "projected_fields": ["id", "tag_name", "draft", "prerelease"],
            "maximum_tag_name_bytes": MAX_RECONCILED_RELEASE_TAG_NAME_BYTES,
            "release_notes_included": false,
            "assets_included": false,
            "release_html_url": "derived_from_configured_origin_repository_and_current_tag_name",
            "missing_release": "inconclusive_resource_not_found",
            "updated_at": "unavailable_in_forgejo_v17_release_response",
            "independent_verification": false,
            "merge_authorizing": false,
        },
    });
    reconcile["event_contracts_v5"] = event_contracts_v5;
    reconcile["event_contracts_v6"] = event_contracts_v6;
    reconcile["event_contracts_v7"] = event_contracts_v7;
    reconcile["event_contracts_v8"] = event_contracts_v8;
    reconcile["event_contracts_v9"] = event_contracts_v9;
    let preflight = json!({
        "id": "read_only_merge_preflight",
        "target": "fixed_default_profile",
        "reads_configuration_file": false,
        "base_branch": MERGE_PREFLIGHT_BASE_BRANCH,
        "base_branch_configurable": false,
        "forgejo_reads": [format!("{api_path}/pulls/{{number}}")],
        "local_reads": ["optional_proof_file", "expected_binding_file", "verifier_key_file"],
        "local_writes": ["merge_lock_directory_if_missing", "merge_lock_file"],
        // The lock is taken before the Forgejo read; its wait is not part of
        // the read budget.
        "merge_lock": {
            "directory": format!("/tmp/{MERGE_LOCK_ROOT_PREFIX}<euid>"),
            "independent_of_tmpdir": true,
            "wait": "bounded",
            "wait_timeout_ms": MERGE_LOCK_WAIT.as_millis() as u64,
            "counts_toward_read_budget": false,
        },
        "input_max_bytes": {
            "expected_binding": MAX_EXPECTED_BINDING_BYTES,
            "proof": MAX_PROOF_BYTES,
            "verifier_key": MAX_VERIFIER_KEY_BYTES,
        },
        "forgejo_writes": false,
        "canonical_state_writes": false,
        "merge_authorized": false,
        "merge_attempted": false,
        "output_schema_version": MERGE_PREFLIGHT_SCHEMA_VERSION,
    });
    let target_profiles = LocalForgejoTargetProfile::ALL
        .into_iter()
        .map(|profile| {
            let repo = format!("/api/v1/repos/{}/{}", profile.owner(), profile.repository());
            let mission_inputs = if profile == LocalForgejoTargetProfile::Mission012Clamp {
                json!({
                    "required": true,
                    "flags": [MISSION_ADMISSION_FILE_FLAG, MISSION_EXECUTION_JOURNAL_FLAG],
                    "source_of_truth": "private mission-execution.jsonl durable allocation and claim journal",
                })
            } else {
                json!({"required": false, "flags": []})
            };
            json!({
                "selector": profile.selector(),
                "default": profile == LocalForgejoTargetProfile::Merge014AgentForgeCore,
                "origin": profile.origin(),
                "owner": profile.owner(),
                "repository": profile.repository(),
                "base_branch": MERGE_PREFLIGHT_BASE_BRANCH,
                "writer_admission_profile": "defaults_to_forgejo_v17_multiwriter; explicit_local_profile_requires_supervisor_capability",
                "writer_admission_source": "kernel_peer_credentials_and_live_process_bindings_from_inherited_supervisor_socket; supervisor_policy_remains_trusted_host_input",
                "supervisor_capability": {
                    "flag": SUPERVISOR_CAPABILITY_FD_FLAG,
                    "transport": "inherited_AF_UNIX_SOCK_SEQPACKET",
                    "packet_max_bytes": 8192,
                    "maximum_lifetime_seconds": 120,
                    "consumed_before_merge_credential": true,
                    "revalidated_immediately_before_merge_post": true,
                    "bindings": ["target_profile", "origin", "repository", "pull_request", "base_branch", "supervisor_peer_pid_uid_start_time_namespaces", "gate_pid_start_time_namespaces", "forgejo_pid_start_time_namespaces", "expiry", "nonce"],
                    "independent_supervisor_policy_verification": false,
                    "trusted_supervisor_launcher_implemented_here": false,
                },
                "mission_admission_inputs": mission_inputs,
                "forgejo_reads": [
                    format!("{repo}/pulls/{{number}}"),
                    format!("{repo}/pulls/{{number}}/reviews"),
                    format!("{repo}/branch_protections"),
                    format!("{repo}/branches/{MERGE_PREFLIGHT_BASE_BRANCH}"),
                    "/api/v1/user",
                    "/api/v1/users/{reviewer-login}",
                ],
                "forgejo_writes": [format!("{repo}/pulls/{{number}}/merge")],
                "readback": [
                    format!("{repo}/pulls/{{number}}"),
                    format!("{repo}/git/commits/{{merge-commit-sha}}"),
                    format!("{repo}/git/commits/{{candidate-head-sha}}"),
                    format!("{repo}/branches/{MERGE_PREFLIGHT_BASE_BRANCH}"),
                    format!("{repo}/issues/{{number}}"),
                    format!(
                        "{repo}/git/trees/{{merge-commit-sha}}?page={{page}}&per_page={GIT_TREE_REQUEST_PAGE_SIZE}&recursive=false"
                    ),
                    format!(
                        "{repo}/git/trees/{{candidate-head-sha}}?page={{page}}&per_page={GIT_TREE_REQUEST_PAGE_SIZE}&recursive=false"
                    ),
                ],
            })
        })
        .collect::<Vec<_>>();
    let local_test_merge_authority = json!({
        "id": "local_test_merge_authority",
        "profile": "fixed_loopback_test_only",
        // Compile-time literal targets, each with its own concrete paths. The
        // older top-level path fields are kept unchanged and describe only
        // the default profile.
        "target_profiles": target_profiles,
        "writer_admission": {
            "selector_flag": MERGE_WRITER_ADMISSION_PROFILE_FLAG,
            "default_profile": "forgejo_v17_multiwriter",
            "supervisor_capability_flag": SUPERVISOR_CAPABILITY_FD_FLAG,
            "supervisor_capability_protocol": "single bounded inherited AF_UNIX SOCK_SEQPACKET consumed before credential read and rechecked immediately before merge POST",
            "local_single_writer_sandbox": {
                "explicit_host_selection_required": true,
                "sandbox_requirement": "the trusted host must exclude every other Forgejo writer for the whole admission, verification, merge, and readback interval",
                "sandbox_exclusion_verified_by_this_adapter": false,
                "supervisor_policy_independently_verified": false,
                "trusted_supervisor_launcher_implemented_here": false,
            },
            "forgejo_v17_multiwriter": {
                "admitted": false,
                "verdict": "inconclusive",
                "error_code": "server_expected_base_precondition_unavailable",
                "server_expected_base_precondition": "not_mandatory_or_unavailable",
                "refuse_before": ["merge_lock", "host_input_read", "forgejo_read", "merge_post"],
            },
        },
        "mission_admission_validation": {
            "required_profile": LocalForgejoTargetProfile::Mission012Clamp.selector(),
            "admission_schema": "MissionHostAdmissionV1",
            "journal_read": "bounded_strict_hash_chain_replay_without_tail_repair",
            "generation_race_protection": {
                "lock_scope": "cooperative_per_mission_lock_shared_with_generation_allocation",
                "claim_revalidated_after_lock_acquisition": true,
                "lock_held_during": [
                    "final_pull_request_base_and_branch_policy_reads",
                    "merge_post",
                    "postmerge_readback",
                ],
                "concurrent_allocation_result": "retryable_execution_mission_busy",
                "excludes_noncooperating_journal_or_forgejo_writers": false,
            },
            "checks": [
                "compiled_profile_scope",
                "review_binding_admission_sha256_and_source_issue",
                "expected_requirement_and_base_commit",
                "exact_durable_allocation_and_request_claim",
                "current_execution_generation",
            ],
            "claim_state_is_copied": false,
        },
        // This fence is enforced by the host runtime's durable merge-attempt
        // journal. It is not a Forgejo server-side exclusion mechanism.
        "merge_attempt_scope_fence": {
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
        },
        "target_profile_selector": MERGE_TARGET_PROFILE_FLAG,
        "uses_configuration_allowed_origins": false,
        "credential_origin_binding": "selected_target_profile_origin",
        // Present on every JSON result of this operation (added by MERGE-015).
        "result_target_fields": ["selector", "origin", "owner", "repository", "base_branch"],
        // V2 is required at the only write authority. Read-only preflight
        // retains the backwards-compatible V1 proof contract.
        "proof_input_schema": "target_bound_proof_v2_only",
        "proof_binding_scope": [
            "profile_selector",
            "canonical_origin",
            "owner",
            "repository",
            "base_branch",
            "signed_evidence",
        ],
        "legacy_v1_merge_authority": "rejected_before_forgejo_read",
        "request_paths_shown_for_profile": LocalForgejoTargetProfile::Merge014AgentForgeCore.selector(),
        "base_branch": MERGE_PREFLIGHT_BASE_BRANCH,
        "forgejo_reads": [
            format!("{api_path}/pulls/{{number}}"),
            format!("{api_path}/pulls/{{number}}/reviews"),
            format!("{api_path}/branch_protections"),
            format!("{api_path}/branches/{MERGE_PREFLIGHT_BASE_BRANCH}"),
            "/api/v1/user",
            "/api/v1/users/{reviewer-id}",
        ],
        "forgejo_writes": [format!("{api_path}/pulls/{{number}}/merge")],
        "credential_source": "inherited_anonymous_pipe_fd_bound_to_fixed_loopback_origin",
        "credential_roles": {
            "merge_actor": "required_inherited_anonymous_pipe_fd",
            "branch_protection_reader": {
                "flag": "--policy-credential-fd",
                "optional": true,
                "allowed_requests": [format!("GET {api_path}/branch_protections")],
                "fallback": "merge_actor_credential_with_same_request_restriction",
            },
        },
        "credential_fd_validation_reference": "credential_boundary.credential_fd_validation",
        "credential_boundary_reference": "credential_boundary",
        "cooperative_branch_lock_through_post_and_readback": true,
        "prevents_external_forgejo_writers": false,
        "post_is_retried": false,
        "ambiguous_post_resolution": "authenticated_pull_request_main_branch_issue_and_equal_candidate_merge_tree_manifest_readback",
        "postmerge_pull_request_identity": {
            "field": "base.repo.full_name",
            "required": true,
            "must_equal": "selected_target_profile_owner_and_repository",
            "mismatch": "inconclusive",
        },
        "readback": [
            format!("{api_path}/pulls/{{number}}"),
            format!("{api_path}/git/commits/{{merge-commit-sha}}"),
            format!("{api_path}/git/commits/{{candidate-head-sha}}"),
            format!("{api_path}/branches/{MERGE_PREFLIGHT_BASE_BRANCH}"),
            format!("{api_path}/issues/{{number}}"),
            format!(
                "{api_path}/git/trees/{{merge-commit-sha}}?page={{page}}&per_page={GIT_TREE_REQUEST_PAGE_SIZE}&recursive=false"
            ),
            format!(
                "{api_path}/git/trees/{{candidate-head-sha}}?page={{page}}&per_page={GIT_TREE_REQUEST_PAGE_SIZE}&recursive=false"
            ),
        ],
    });
    let commit_status_snapshot = json!({
        "id": "read_commit_status_snapshot",
        "target_profile": LocalForgejoTargetProfile::Merge014AgentForgeCore.selector(),
        "forgejo_reads": [format!(
            "{api_path}/commits/{{full-commit-sha}}/status?limit={COMMIT_STATUS_PAGE_SIZE}&page={{page}}"
        )],
        "accepted_status_values": ["pending", "success", "failure", "error", "warning", "skipped"],
        "pagination": {
            "first_page_only": false,
            "maximum_pages": MAX_COMMIT_STATUS_PAGES,
            "maximum_statuses": MAX_COMMIT_STATUS_COUNT,
            "oversized_result": "inconclusive_after_bounded_pages",
            "multi_page_consistency": "sequential_non_atomic",
            "changed_total_count": "inconsistent_pagination_actionable_reread",
            "complete_multi_page_recheck": "bounded_second_pass_rejects_changed_status_pages_or_combined_state",
            "maximum_page_reads_without_retries": MAX_COMMIT_STATUS_PAGES * 2,
        },
        "evidence_scope": "forgejo_reported_contexts_only",
        "independent_verification": false,
        "merge_authorizing": false,
        "pull_request_head_binding": "caller_supplied_sha_only_not_revalidated",
        "exit_code_zero_means": "snapshot_read_complete_not_verification_passed",
        "credential_source": "optional_inherited_anonymous_pipe_fd",
    });
    let pull_request_status_snapshot = json!({
        "id": "read_pull_request_status_snapshot",
        "schema_version": "forgejo-pull-request-status-snapshot-v1",
        "target_profile": LocalForgejoTargetProfile::Merge014AgentForgeCore.selector(),
        "forgejo_reads": [
            format!("{api_path}/pulls/{{number}}"),
            format!("{api_path}/commits/{{head-sha}}/status?limit={COMMIT_STATUS_PAGE_SIZE}&page={{page}}"),
        ],
        "pull_request_head_binding": "server_read_before_and_after_status_pages;changed_binding_is_inconclusive",
        "pull_request_head_fields_rechecked": ["number", "state", "merged", "base_branch", "base_sha", "head_sha"],
        "binding_change_fields": ["number", "state", "merged", "base_branch", "base_sha", "head_sha"],
        "binding_stability_semantics": "all_listed_fields_must_match;head_sha_stability_is_reported_separately",
        "pull_request_identity_scope": "configured_repository_and_canonical_pull_request_url",
        "possible_observations": ["complete", "no_statuses", "inconclusive"],
        "empty_status_policy": "no_statuses_is_not_verification_success_requires_independent_verifier",
        "incomplete_status_policy": "inconclusive_requires_complete_reread",
        "status_pagination": {
            "maximum_pages": MAX_COMMIT_STATUS_PAGES,
            "maximum_statuses": MAX_COMMIT_STATUS_COUNT,
            "multi_page_consistency": "sequential_non_atomic",
            "complete_multi_page_recheck": "bounded_second_pass_rejects_changed_status_pages_or_combined_state",
            "maximum_page_reads_without_retries": MAX_COMMIT_STATUS_PAGES * 2,
        },
        "evidence_scope": "forgejo_reported_contexts_only",
        "independent_verification": false,
        "merge_authorizing": false,
        "exit_code_zero_means": "read_complete_pr_head_stable_at_two_observations_not_verification_passed",
        "credential_source": "optional_inherited_anonymous_pipe_fd",
    });
    json!({
        "schema_version": CAPABILITIES_SCHEMA_VERSION,
        "api_version": CAPABILITIES_API_VERSION,
        "adapter": "forgejo-reconcile-local",
        "process_lifecycle": {
            "curl_child_parent_exit": if cfg!(target_os = "linux") {
                "sigkill_via_linux_pr_set_pdeathsig"
            } else {
                "not_guaranteed"
            },
        },
        "credential_boundary": {
            "secret_bytes_in_argv_environment_journal_or_response": false,
            "secret_material_resides_in": [
                "adapter_process_memory",
                "curl_child_stdin_and_memory",
            ],
            "credential_fd_validation": {
                "descriptor_shape": "anonymous_pipe_checked",
                "content": ["bounded_read_to_eof", "token_syntax_validated"],
                "maximum_input_bytes": MAX_FORGEJO_API_TOKEN_BYTES,
                "read_timeout_ms": HOST_CREDENTIAL_PIPE_READ_TIMEOUT.as_millis() as u64,
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
        },
        "configuration": {
            "schema_version": 1,
            "allowed_origins": [LOCAL_FORGEJO_ORIGIN],
            // The merge operation lists its own target_profiles; this file
            // allowlist never authorizes a merge target.
            // Only `--config` reconciliation reads this file; preflight is fixed
            // to the default profile without reading it.
            "allowed_origins_scope": ["reconcile_webhook_deliveries"],
            "selects_merge_target_profile": false,
            "repository": {
                "owner": LOCAL_REPOSITORY_OWNER,
                "name": LOCAL_REPOSITORY_NAME,
            },
            "authentication": {
                "reconcile_webhook_deliveries": "optional_inherited_anonymous_pipe_fd",
                "read_only_merge_preflight": "anonymous_only",
                "read_only_commit_status_snapshot": "optional_inherited_anonymous_pipe_fd",
                "read_only_pull_request_status_snapshot": "optional_inherited_anonymous_pipe_fd",
                "local_test_merge_authority": "inherited_anonymous_pipe_fd",
                "local_test_merge_policy_reader": "optional_exact_route_inherited_anonymous_pipe_fd",
            },
            "file_fields": ["schema_version", "origin", "owner", "repository"],
            "file_max_bytes": MAX_CONFIG_BYTES,
        },
        "upstream": {
            "api_version": FORGEJO_REST_API_VERSION,
            "http_methods": ["GET", "POST_merge_only_in_local_test_profile"],
            "max_attempts_per_resource": retry_policy.max_attempts,
            "request_timeout_ms": retry_policy.request_timeout.as_millis() as u64,
            // Covers Forgejo reads only; there is no whole-operation deadline.
            "read_budget_ms": retry_policy.total_budget.as_millis() as u64,
            "read_budget_scope": "forgejo_reads_per_invocation",
            "max_response_bytes": MAX_RESPONSE_BYTES,
            "max_response_header_json_bytes": MAX_CURL_HEADER_JSON_BYTES,
            "oversized_response": {
                "error_code": "upstream_response_too_large",
                "retryable": false,
                "components": ["body", "header_json", "captured_output"],
            },
            "duplicate_x_total_count": "reject_response",
            "duplicate_retry_after": "disable_retry",
        },
        "operations": [
            reconcile,
            preflight,
            commit_status_snapshot,
            pull_request_status_snapshot,
            local_test_merge_authority,
        ],
    })
}

fn run() -> Result<(Value, bool), String> {
    let mut args = env::args_os().skip(1);
    let first_arg = args.next().ok_or_else(usage)?;
    if first_arg == "--capabilities" {
        // Refuse before any JSON output, configuration, journal or network access.
        // An inbox file named `--capabilities` stays reachable as `./--capabilities`.
        if args.next().is_some() {
            return Err("--capabilities accepts no additional arguments".to_owned());
        }
        return Ok((capabilities_document(), true));
    }
    if first_arg == "--quarantine-invalid-delivery" {
        return run_quarantine_invalid_delivery(args);
    }
    if first_arg == "--preflight" {
        return run_merge_preflight(args);
    }
    if first_arg == "--commit-status-snapshot" {
        return run_commit_status_snapshot(args);
    }
    if first_arg == "--pull-request-status-snapshot" {
        return run_pull_request_status_snapshot(args);
    }
    if first_arg == "--merge-local-test" {
        return run_local_test_merge_authority(args);
    }
    let invocation = parse_reconcile_invocation(std::iter::once(first_arg).chain(args))?;
    run_reconciliation(invocation)
}

fn run_quarantine_invalid_delivery(
    arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(Value, bool), String> {
    let arguments = arguments.collect::<Vec<_>>();
    if arguments.len() != 4 {
        return Err(usage());
    }
    let journal_path = PathBuf::from(&arguments[0]);
    let delivery_id = arguments[1]
        .to_str()
        .ok_or_else(|| "delivery ID must be valid UTF-8".to_owned())?;
    let sequence_text = arguments[2]
        .to_str()
        .ok_or_else(|| "source sequence must be valid UTF-8".to_owned())?;
    let source_sequence = sequence_text
        .parse::<u64>()
        .map_err(|_| "source sequence must be a positive canonical integer".to_owned())?;
    if source_sequence == 0 || source_sequence.to_string() != sequence_text {
        return Err("source sequence must be a positive canonical integer".to_owned());
    }
    let source_event_hash = arguments[3]
        .to_str()
        .ok_or_else(|| "source event hash must be valid UTF-8".to_owned())?;
    let result = quarantine_invalid_webhook_delivery(
        &EventJournal::open_webhook_inbox(journal_path),
        delivery_id,
        source_sequence,
        source_event_hash,
    )
    .map_err(|error| match error {
        WebhookError::InvalidDeliveryId => "delivery ID is not valid for the webhook inbox".to_owned(),
        WebhookError::Journal(JournalError::LockBusy) => {
            "inbox journal is busy; retry the host quarantine operation after the writer exits".to_owned()
        }
        WebhookError::Journal(_) => {
            "source row changed, is not eligible for quarantine, or the inbox could not be updated; preserve the journal and inspect it in the trusted host".to_owned()
        }
        _ => "quarantine disposition could not be validated; preserve the journal and inspect it in the trusted host".to_owned(),
    })?;
    Ok((
        json!({
            "schema_version": result.schema_version,
            "verdict": "delivery_quarantined",
            "canonical_state_created": false,
            "source_hmac_persisted": false,
            "source_hmac_reverified": false,
            "actor_identity_recorded": false,
            "upstream_read_attempts": 0,
            "delivery_id": result.delivery_id,
            "source_sequence": result.source_sequence,
            "source_event_hash": result.source_event_hash,
            "reason_code": result.reason_code,
            "reason_field": result.reason_field,
            "disposition_sequence": result.disposition_sequence,
            "duplicate": result.duplicate,
        }),
        true,
    ))
}

fn run_commit_status_snapshot(
    arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(Value, bool), String> {
    let arguments = arguments.collect::<Vec<_>>();
    let mut commit_sha = None;
    let mut credential_fd = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some(RECONCILE_HOST_CREDENTIAL_FD_FLAG) => {
                if credential_fd.is_some() {
                    return Err("--host-credential-fd may be supplied only once".to_owned());
                }
                let fd = arguments
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(usage)?
                    .parse::<RawFd>()
                    .map_err(|_| {
                        "host credential descriptor must be a file descriptor number".to_owned()
                    })?;
                if fd <= libc::STDERR_FILENO {
                    return Err(
                        "host credential descriptor must be an anonymous pipe above stderr"
                            .to_owned(),
                    );
                }
                credential_fd = Some(fd);
                index += 2;
            }
            Some(option) if option.starts_with('-') => {
                return Err("unknown commit status snapshot option".to_owned());
            }
            Some(value) => {
                if commit_sha.replace(value.to_owned()).is_some() {
                    return Err(usage());
                }
                index += 1;
            }
            None => return Err("commit SHA must be valid UTF-8".to_owned()),
        }
    }
    let commit_sha = commit_sha.ok_or_else(usage)?;
    if !valid_canonical_commit_status_sha(&commit_sha) {
        return Err("commit SHA must be a full hexadecimal object ID".to_owned());
    }
    let host_token = credential_fd
        .map(read_inherited_credential_pipe)
        .transpose()?;
    let api = reconciliation_api(LocalForgejoConfigV1::default(), host_token);
    let (snapshot, complete) = api
        .read_commit_status_snapshot(&commit_sha)
        .map_err(commit_status_snapshot_cli_error)?;
    Ok((snapshot, complete))
}

fn run_pull_request_status_snapshot(
    arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(Value, bool), String> {
    let arguments = arguments.collect::<Vec<_>>();
    let mut pull_request_number = None;
    let mut credential_fd = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some(RECONCILE_HOST_CREDENTIAL_FD_FLAG) => {
                if credential_fd.is_some() {
                    return Err("--host-credential-fd may be supplied only once".to_owned());
                }
                let fd = arguments
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or_else(usage)?
                    .parse::<RawFd>()
                    .map_err(|_| {
                        "host credential descriptor must be a file descriptor number".to_owned()
                    })?;
                if fd <= libc::STDERR_FILENO {
                    return Err(
                        "host credential descriptor must be an anonymous pipe above stderr"
                            .to_owned(),
                    );
                }
                credential_fd = Some(fd);
                index += 2;
            }
            Some(option) if option.starts_with('-') => {
                return Err("unknown pull request status snapshot option".to_owned());
            }
            Some(value) => {
                if pull_request_number.is_some() {
                    return Err(usage());
                }
                let parsed = value.parse::<u64>().map_err(|_| {
                    "pull request number must be a positive canonical integer".to_owned()
                })?;
                if parsed == 0 || parsed.to_string() != value {
                    return Err(
                        "pull request number must be a positive canonical integer".to_owned()
                    );
                }
                pull_request_number = Some(parsed);
                index += 1;
            }
            None => return Err("pull request number must be valid UTF-8".to_owned()),
        }
    }
    let pull_request_number = pull_request_number.ok_or_else(usage)?;
    let host_token = credential_fd
        .map(read_inherited_credential_pipe)
        .transpose()?;
    let api = reconciliation_api(LocalForgejoConfigV1::default(), host_token);
    let (snapshot, complete) = api
        .read_pull_request_status_snapshot(pull_request_number)
        .map_err(pull_request_status_snapshot_cli_error)?;
    Ok((snapshot, complete))
}

fn pull_request_status_snapshot_cli_error(error: ApiReadError) -> String {
    match error {
        ApiReadError::NotFound => {
            "the pull request is absent or not visible; verify its number and repository access"
                .to_owned()
        }
        ApiReadError::InvalidResponse => {
            "Forgejo returned a mismatched pull request or status snapshot; reread it and do not treat it as verification".to_owned()
        }
        other => commit_status_snapshot_cli_error(other),
    }
}

fn commit_status_snapshot_cli_error(error: ApiReadError) -> String {
    match error {
        ApiReadError::Unavailable => {
            "local Forgejo status read timed out or is unavailable; check the loopback instance and retry".to_owned()
        }
        ApiReadError::Unauthorized | ApiReadError::InvalidCredential => {
            "Forgejo rejected the optional host credential; refresh it in the trusted host process".to_owned()
        }
        ApiReadError::Forbidden => {
            "the local Forgejo credential lacks permission for this repository status read".to_owned()
        }
        ApiReadError::NotFound => {
            "the commit or repository is absent or not visible; verify the full SHA and repository access".to_owned()
        }
        ApiReadError::HttpStatus(status) => {
            format!("local Forgejo returned HTTP {status}; inspect the response and retry if transient")
        }
        ApiReadError::HttpStatusWithRetryAfter {
            status,
            retry_after_ms,
        } => format!(
            "local Forgejo returned HTTP {status}; wait at least {retry_after_ms} ms, then retry with a fresh read"
        ),
        ApiReadError::HttpStatusWithInvalidRetryAfter { status } => format!(
            "local Forgejo returned HTTP {status}; Retry-After is invalid or ambiguous, so do not retry automatically from it"
        ),
        ApiReadError::ResponseTooLarge {
            component,
            maximum_bytes,
        } => format!(
            "Forgejo response {component} exceeded the local {maximum_bytes}-byte limit; reduce the requested result or correct the upstream response, then reread"
        ),
        ApiReadError::InvalidResponse => {
            "Forgejo returned an invalid or mismatched status snapshot; do not treat it as verification".to_owned()
        }
        ApiReadError::BatchCacheBudgetExceeded { .. } => {
            "the status snapshot exceeded the configured local read budget".to_owned()
        }
        ApiReadError::InconsistentPagination => {
            "Forgejo status pages changed or counts are inconsistent; reread the complete snapshot before using it".to_owned()
        }
    }
}

fn run_reconciliation(invocation: ReconcileInvocationV1) -> Result<(Value, bool), String> {
    if invocation.delivery_ids.len() > MAX_RECONCILE_BATCH_EVENTS {
        return Ok(batch_reconciliation_failure(
            &ReconcileError::BatchTooLarge,
            invocation.delivery_ids.len(),
            0,
        ));
    }
    let mut unique_delivery_ids = BTreeSet::new();
    if invocation
        .delivery_ids
        .iter()
        .any(|delivery_id| !unique_delivery_ids.insert(delivery_id))
    {
        return Ok(duplicate_delivery_selection_failure(
            invocation.delivery_ids.len(),
        ));
    }
    let events = match EventJournal::open_webhook_inbox(&invocation.journal_path)
        .replay_read_only()
    {
        Ok(events) => events,
        Err(JournalError::Corrupt { .. }) => {
            return Ok(invalid_webhook_inbox_failure(&invocation.delivery_ids));
        }
        Err(error) => return Err(match error {
            JournalError::LockBusy => {
                "inbox journal is busy; retry reconciliation after the active journal operation completes"
                    .to_owned()
            }
            JournalError::InboxTooLarge { maximum_bytes } => format!(
                "inbox_capacity_exceeded: inbox exceeds {maximum_bytes} bytes; preserve a backup and apply a verified compaction or migration before retrying"
            ),
            error => format!("cannot replay inbox: {error:?}"),
        }),
    };
    let mut selected = Vec::with_capacity(invocation.delivery_ids.len());
    for delivery_id in &invocation.delivery_ids {
        let idempotency_key = format!("forgejo-delivery-v1:{delivery_id}");
        let Some(event) = events
            .iter()
            .find(|event| event.idempotency_key == idempotency_key)
        else {
            // Resolve the entire request before reading a host credential or
            // making any upstream call. A partial batch is refused as a unit.
            return Ok(delivery_selection_failure(
                &invocation.delivery_ids,
                delivery_id,
            ));
        };
        selected.push(event.clone());
    }

    let quarantines = match selected
        .iter()
        .map(|event| quarantine_for_source(&events, event))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(quarantines) => quarantines,
        Err(_) => {
            let (mut failure, exit_success) =
                invalid_quarantine_disposition_failure(&invocation.delivery_ids);
            attach_selected_event_sources(
                &mut failure,
                &invocation.delivery_ids,
                &selected,
                &BTreeSet::new(),
                &BTreeSet::new(),
            );
            return Ok((failure, exit_success));
        }
    };
    let quarantined_delivery_ids = invocation
        .delivery_ids
        .iter()
        .zip(&quarantines)
        .filter_map(|(delivery_id, quarantine)| quarantine.as_ref().map(|_| delivery_id.clone()))
        .collect::<BTreeSet<_>>();
    let mut attestation_verified_delivery_ids = BTreeSet::new();

    // Read the host credential only when at least one selected event can name a
    // resource inside this adapter's configured repository. Malformed and
    // out-of-scope events are rejected without consuming a one-shot credential.
    // The FD itself is never passed through to curl.
    let needs_host_credential = invocation.credential_fd.is_some()
        && selected
            .iter()
            .zip(&quarantines)
            .any(|(event, quarantine)| {
                quarantine.is_none()
                    && event_resource_ref(event).is_ok_and(|resource| {
                        resource.owner == invocation.config.owner
                            && resource.repository == invocation.config.repository
                    })
            });
    if needs_host_credential && invocation.trusted_attestation_keys.is_empty() {
        let (mut failure, exit_success) = host_attestation_refusal_failure(
            &invocation.delivery_ids,
            None,
            "host_journal_attestation_policy_missing",
            "configure one to eight trusted receiver attestation public keys in the trusted host launcher, then retry; do not provide a host credential to the legacy unauthenticated journal reader",
        );
        attach_selected_event_sources(
            &mut failure,
            &invocation.delivery_ids,
            &selected,
            &quarantined_delivery_ids,
            &attestation_verified_delivery_ids,
        );
        return Ok((failure, exit_success));
    }
    if needs_host_credential || !invocation.trusted_attestation_keys.is_empty() {
        for ((delivery_id, event), quarantine) in invocation
            .delivery_ids
            .iter()
            .zip(&selected)
            .zip(&quarantines)
        {
            if quarantine.is_some() {
                continue;
            }
            if let Err(error) =
                verify_journal_event_attestation(event, &invocation.trusted_attestation_keys)
            {
                let code = attestation_refusal_code(&error);
                let (mut failure, exit_success) = host_attestation_refusal_failure(
                    &invocation.delivery_ids,
                    Some(delivery_id),
                    code,
                    "preserve the inbox, confirm the trusted receiver public-key set and delivery signature, and retry only after a trusted receiver creates a valid attested delivery; keep the host credential withheld until then",
                );
                attach_selected_event_sources(
                    &mut failure,
                    &invocation.delivery_ids,
                    &selected,
                    &quarantined_delivery_ids,
                    &attestation_verified_delivery_ids,
                );
                return Ok((failure, exit_success));
            }
            attestation_verified_delivery_ids.insert(delivery_id.clone());
        }
    }
    let host_token = if needs_host_credential {
        match invocation
            .credential_fd
            .map(read_inherited_credential_pipe)
            .transpose()
        {
            Ok(token) => token,
            Err(reader_error) => {
                let (mut failure, exit_success) =
                    host_credential_pipe_read_failure(&invocation.delivery_ids, &reader_error);
                if invocation.delivery_ids.len() == 1 {
                    attach_journal_source_observation(
                        &mut failure,
                        &selected[0],
                        attestation_verified_delivery_ids.contains(&invocation.delivery_ids[0]),
                        quarantined_delivery_ids.contains(&invocation.delivery_ids[0]),
                    );
                } else {
                    attach_selected_event_sources(
                        &mut failure,
                        &invocation.delivery_ids,
                        &selected,
                        &quarantined_delivery_ids,
                        &attestation_verified_delivery_ids,
                    );
                }
                return Ok((failure, exit_success));
            }
        }
    } else {
        None
    };
    let configured_owner = invocation.config.owner.clone();
    let configured_repository = invocation.config.repository.clone();
    let api = reconciliation_api(invocation.config, host_token);

    if selected.len() == 1 {
        let result = match quarantines[0].as_ref() {
            Some(disposition) => Err(ReconcileError::DeliveryQuarantined {
                source_sequence: disposition.source_sequence,
                source_event_hash: disposition.source_event_hash.clone(),
                disposition_sequence: disposition.disposition_sequence,
            }),
            None => reconcile_inbox_event(&selected[0], &api),
        };
        let upstream_read_attempts = api.upstream_read_attempts.load(Ordering::Relaxed);
        let (mut value, exit_success) = single_reconciliation_result(
            &invocation.delivery_ids[0],
            result,
            upstream_read_attempts,
        );
        attach_journal_source_observation(
            &mut value,
            &selected[0],
            attestation_verified_delivery_ids.contains(&invocation.delivery_ids[0]),
            quarantines[0].is_some(),
        );
        if let Some(error) = value.get_mut("error") {
            classify_out_of_scope_delivery_error(
                error,
                &selected[0],
                &configured_owner,
                &configured_repository,
            );
            attach_scoped_resource_detail(
                error,
                &selected[0],
                &configured_owner,
                &configured_repository,
            );
            attach_journal_source_identity(error, &selected[0]);
        }
        return Ok((value, exit_success));
    }

    let active_indexes = quarantines
        .iter()
        .enumerate()
        .filter_map(|(index, quarantine)| quarantine.is_none().then_some(index))
        .collect::<Vec<_>>();
    let active_events = active_indexes
        .iter()
        .map(|index| selected[*index].clone())
        .collect::<Vec<_>>();
    let active_results = match if active_events.is_empty() {
        Ok(Vec::new())
    } else {
        reconcile_inbox_events(&active_events, &api)
    } {
        Ok(results) => results,
        Err(error) => {
            let upstream_read_attempts = api.upstream_read_attempts.load(Ordering::Relaxed);
            let (mut failure, exit_success) = batch_reconciliation_failure(
                &error,
                invocation.delivery_ids.len(),
                upstream_read_attempts,
            );
            attach_selected_event_sources(
                &mut failure,
                &invocation.delivery_ids,
                &selected,
                &quarantined_delivery_ids,
                &attestation_verified_delivery_ids,
            );
            return Ok((failure, exit_success));
        }
    };
    let mut active_results = active_results.into_iter();
    let results = quarantines
        .iter()
        .map(|quarantine| match quarantine {
            Some(disposition) => Err(ReconcileError::DeliveryQuarantined {
                source_sequence: disposition.source_sequence,
                source_event_hash: disposition.source_event_hash.clone(),
                disposition_sequence: disposition.disposition_sequence,
            }),
            None => active_results
                .next()
                .expect("each non-quarantined delivery has one reconciliation result"),
        })
        .collect::<Vec<_>>();
    let has_errors = results.iter().any(Result::is_err);
    let upstream_read_attempts = api.upstream_read_attempts.load(Ordering::Relaxed);
    let results = results
        .into_iter()
        .zip(invocation.delivery_ids)
        .zip(selected.iter())
        .zip(quarantines.iter())
        .map(|(((result, requested_delivery_id), event), quarantine)| {
            let mut value = match result {
                Ok(reconciled) => {
                    let mut value = reconciled_state_value(&reconciled);
                    value["verdict"] = Value::String("current_state_read".to_owned());
                    value
                }
                Err(error) => {
                    let mut error_value = reconciliation_error_value(&error);
                    classify_out_of_scope_delivery_error(
                        &mut error_value,
                        event,
                        &configured_owner,
                        &configured_repository,
                    );
                    attach_scoped_resource_detail(
                        &mut error_value,
                        event,
                        &configured_owner,
                        &configured_repository,
                    );
                    attach_journal_source_identity(&mut error_value, event);
                    json!({
                        "verdict": "inconclusive",
                        "delivery_id": requested_delivery_id,
                        "error": error_value,
                    })
                }
            };
            attach_journal_source_observation(
                &mut value,
                event,
                attestation_verified_delivery_ids.contains(&requested_delivery_id),
                quarantine.is_some(),
            );
            value
        })
        .collect::<Vec<_>>();

    Ok((
        json!({
            "schema_version": RECONCILE_BATCH_SCHEMA_VERSION,
            "api_version": FORGEJO_REST_API_VERSION,
            "source_event_provenance_contract": source_event_provenance_contract(),
            "verdict": if has_errors { "inconclusive" } else { "current_state_read" },
            "upstream_read_attempts": upstream_read_attempts,
            "results": results,
        }),
        !has_errors,
    ))
}

fn usage() -> String {
    "usage: forgejo-reconcile-local [--config <file>|--target-profile merge-014-agent-forge-core|mission-012-clamp|--reconcile-target-profile org-team-review-e2e] [--trusted-webhook-attestation-public-key-hex <canonical-key> ...] [--host-credential-fd <fd>] <inbox.jsonl> <delivery-id> [<delivery-id> ...] | forgejo-reconcile-local --quarantine-invalid-delivery <inbox.jsonl> <delivery-id> <source-sequence> <source-sha256> | forgejo-reconcile-local --preflight <pr-number> <proof.json|-> <expected.json> <verifier-key> <execution-identity> | forgejo-reconcile-local --commit-status-snapshot <full-commit-sha> [--host-credential-fd <fd>] | forgejo-reconcile-local --pull-request-status-snapshot <pr-number> [--host-credential-fd <fd>] | forgejo-reconcile-local --merge-local-test [--writer-admission-profile local_single_writer_sandbox|forgejo_v17_multiwriter] [--supervisor-capability-fd <fd>] [--target-profile merge-014-agent-forge-core|mission-012-clamp] [--mission-admission-file <private-json>] [--mission-execution-journal <private-jsonl>] [--policy-credential-fd <fd>] <pr-number> <proof.json> <expected.json> <verifier-key> <policy.json> <review-base-binding.json> <execution-identity> <host-credential-fd> | forgejo-reconcile-local --capabilities (--config, --target-profile, --reconcile-target-profile, --trusted-webhook-attestation-public-key-hex, --host-credential-fd, --preflight, --commit-status-snapshot, --pull-request-status-snapshot, --merge-local-test, --quarantine-invalid-delivery and --capabilities are reserved first arguments; pass an inbox file with one of those names as ./<name>)".to_owned()
}

fn main() {
    match run() {
        Ok((value, exit_success)) => {
            println!(
                "{}",
                serde_json::to_string(&value).expect("JSON value serializes")
            );
            if !exit_success {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_forge_core::{
        AppendResult, ContextManifest, ContextSource, JournalEvent,
        agent_runtime::{
            AgentExecutionRequestV1, ContextCitationV1, ContextPacketV1, ExecutionClaimResultV1,
            MissionExecutionJournalV1, RuntimeBudgetV1, RuntimeCapabilitiesV1, WorkspaceRefV1,
            derive_mission_identity_v1,
        },
        forgejo::MergeWriterAdmissionProfileV1,
        mission_admission::{
            IssueSnapshotV1, MissionAdmissionBindingV2, MissionAdmissionSourceV1,
            MissionHostAdmissionV1, MissionHostAllocationV1, MissionHostScopeV1,
            MissionIssueStateV1, ProtectedBranchHeadV1, SourceFailureV1,
            WebhookMissionAdmissionErrorV1, admit_reconciled_issue_v2,
        },
        sha256,
        webhook::{
            ingest_forgejo_delivery, ingest_forgejo_delivery_attested, read_journaled_webhook_event,
        },
    };
    use ed25519_dalek::SigningKey;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::{
        cell::Cell,
        ffi::CString,
        fs,
        io::{self, Read, Write},
        net::{Shutdown, SocketAddr, TcpListener, TcpStream},
        os::unix::ffi::OsStrExt,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        path::PathBuf,
        process::{Child, ExitStatus},
        sync::{Arc, Mutex},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    type FixtureHmacSha256 = Hmac<Sha256>;

    struct ScratchJournalDir(PathBuf);

    impl ScratchJournalDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is after UNIX_EPOCH")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "forgejo-reconcile-http-e2e-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create private journal directory");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("restrict journal directory permissions");
            Self(path)
        }
    }

    impl Drop for ScratchJournalDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_legacy_webhook_fixture(
        journal_path: &std::path::Path,
        delivery_id: &str,
        event_name: &str,
        body_payload: Value,
    ) {
        let idempotency_key = format!("forgejo-delivery-v1:{delivery_id}");
        let kind = format!("forgejo.webhook.v1.{event_name}");
        let previous_hash = "0".repeat(64);
        let fields = json!({
            "sequence": 1,
            "event_id": "evt-1",
            "idempotency_key": idempotency_key,
            "kind": kind,
            "payload": {
                "schema_version": 1,
                "provider": "forgejo",
                "delivery_id": delivery_id,
                "event": event_name,
                "payload": body_payload,
            },
            "previous_hash": previous_hash,
        });
        let hash =
            sha256(&serde_json::to_vec(&fields).expect("serialize legacy event hash material"));
        let mut event = fields;
        event["hash"] = Value::String(hash);
        let mut bytes = serde_json::to_vec(&event).expect("serialize legacy webhook fixture");
        bytes.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(journal_path)
            .expect("create private legacy webhook fixture");
        file.write_all(&bytes)
            .expect("write legacy webhook fixture");
        file.sync_all().expect("flush legacy webhook fixture");
    }

    struct FixtureHostCredentialProvider(String);

    impl FixtureHostCredentialProvider {
        fn new(token: &str) -> Self {
            Self(token.to_owned())
        }
    }

    impl HostForgejoCredentialProviderV1 for FixtureHostCredentialProvider {
        fn api_token(&self) -> Result<Option<ForgejoApiToken>, ApiReadError> {
            ForgejoApiToken::parse(self.0.as_bytes().to_vec()).map(Some)
        }
    }

    struct CountingHostCredentialProvider {
        token: String,
        calls: Arc<AtomicUsize>,
    }

    impl HostForgejoCredentialProviderV1 for CountingHostCredentialProvider {
        fn api_token(&self) -> Result<Option<ForgejoApiToken>, ApiReadError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            ForgejoApiToken::parse(self.token.as_bytes().to_vec()).map(Some)
        }
    }

    fn start_http_fixture(responses: Vec<(u16, Duration)>) -> (String, thread::JoinHandle<usize>) {
        start_http_fixture_with_bodies(
            responses
                .into_iter()
                .map(|(status, delay)| {
                    let body = match status {
                        200 => br#"{"number":11}"#.to_vec(),
                        401 => br#"{"message":"unauthorized"}"#.to_vec(),
                        429 => br#"{"message":"rate limited"}"#.to_vec(),
                        _ => br#"{"message":"temporary"}"#.to_vec(),
                    };
                    (status, delay, body)
                })
                .collect(),
        )
    }

    fn start_http_fixture_with_bodies(
        responses: Vec<(u16, Duration, Vec<u8>)>,
    ) -> (String, thread::JoinHandle<usize>) {
        let (origin, server, _) = start_http_fixture_with_bodies_recording_requests(responses);
        (origin, server)
    }

    fn start_http_fixture_with_bodies_recording_requests(
        responses: Vec<(u16, Duration, Vec<u8>)>,
    ) -> (String, thread::JoinHandle<usize>, Arc<Mutex<Vec<String>>>) {
        start_http_fixture_with_extra_headers_recording_requests(
            responses
                .into_iter()
                .map(|(status, delay, body)| (status, delay, body, Vec::new()))
                .collect(),
        )
    }

    fn start_http_fixture_with_extra_headers_recording_requests(
        responses: Vec<(u16, Duration, Vec<u8>, Vec<String>)>,
    ) -> (String, thread::JoinHandle<usize>, Arc<Mutex<Vec<String>>>) {
        start_http_fixture_with_extra_headers_recording_requests_for_origin(|_| responses)
    }

    fn start_http_fixture_with_extra_headers_recording_requests_for_origin(
        response_factory: impl FnOnce(&str) -> Vec<(u16, Duration, Vec<u8>, Vec<String>)>,
    ) -> (String, thread::JoinHandle<usize>, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let responses = response_factory(&origin);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        let server = thread::spawn(move || {
            let mut served = 0;
            for (status, delay, body, extra_headers) in responses {
                let idle_deadline = std::time::Instant::now() + Duration::from_millis(500);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() >= idle_deadline {
                                return served;
                            }
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => return served,
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_millis(250)))
                    .unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                let request_line = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                server_requests.lock().unwrap().push(request_line);
                thread::sleep(delay);

                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    429 => "Too Many Requests",
                    _ => "Service Unavailable",
                };
                let mut header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for extra_header in extra_headers {
                    header.push_str(&extra_header);
                    header.push_str("\r\n");
                }
                header.push_str("\r\n");
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                served += 1;
            }
            served
        });
        (origin, server, requests)
    }

    fn start_issue_and_pull_same_number_fixture() -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server_origin = origin.clone();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let idle_deadline = Instant::now() + Duration::from_secs(1);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if Instant::now() >= idle_deadline {
                                return requests;
                            }
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => return requests,
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_millis(250)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer);
                let request_line = String::from_utf8_lossy(&buffer)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                requests.push(request_line);
                let (status, body) = match path.as_str() {
                    "/api/v1/repos/forge-admin/agent-forge-core/issues/19" => (
                        200,
                        json!({
                            "number": 19,
                            "state": "closed",
                            "updated_at": "2026-10-25T09:30:00Z",
                            "html_url": format!("{server_origin}/forge-admin/agent-forge-core/issues/19"),
                            "title": "current issue title",
                            "body": "current issue body"
                        }),
                    ),
                    "/api/v1/repos/forge-admin/agent-forge-core/pulls/19" => (
                        200,
                        json!({
                            "number": 19,
                            "state": "open",
                            "merged": false,
                            "draft": false,
                            "updated_at": "2026-10-25T09:31:00Z",
                            "html_url": format!("{server_origin}/forge-admin/agent-forge-core/pulls/19"),
                            "head": {"sha": "e7a0672a04f51c3f430a57780a2de5fb2fdf0958"},
                            "base": {"sha": "f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0"},
                            "title": "current pull request title",
                            "body": "current pull request body",
                            "requested_reviewers": [
                                {"id": 7, "login": "reviewer-z", "email": "private-z@example.invalid"},
                                {"id": 3, "login": "reviewer-a", "email": "private-a@example.invalid"}
                            ],
                            "requested_reviewers_teams": [
                                {"id": 29, "name": "team-z", "description": "private team details"},
                                {"id": 4, "name": "team-a", "organization": {"username": "private-org"}}
                            ]
                        }),
                    ),
                    _ => (404, json!({"message":"unexpected fixture path"})),
                };
                let body = serde_json::to_vec(&body).unwrap();
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
            requests
        });
        (origin, server)
    }

    fn start_redirect_http_fixture() -> (String, thread::JoinHandle<(usize, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let redirect_origin = origin.clone();
        let server = thread::spawn(move || {
            let mut requests = 0;
            let mut first_request = String::new();
            let first_deadline = Instant::now() + Duration::from_secs(1);
            loop {
                let deadline = if requests == 0 {
                    first_deadline
                } else {
                    Instant::now() + Duration::from_secs(1)
                };
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if Instant::now() >= deadline {
                                return (requests, first_request);
                            }
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => return (requests, first_request),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_millis(250)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 512];
                while request.len() < 16 * 1024
                    && !request.windows(4).any(|window| window == b"\r\n\r\n")
                {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => request.extend_from_slice(&buffer[..count]),
                    }
                }
                if requests == 0 {
                    first_request = String::from_utf8_lossy(&request).into_owned();
                    let response = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {redirect_origin}/redirect-target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes());
                } else {
                    let body = br#"{"number":12}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(body);
                }
                requests += 1;
            }
        });
        (origin, server)
    }

    fn start_current_issue_http_fixture() -> (String, thread::JoinHandle<String>) {
        let (origin, server) = start_current_issue_http_fixture_for_requests(1);
        let single_request = thread::spawn(move || {
            let requests = server.join().expect("current issue fixture completed");
            assert_eq!(requests.len(), 1);
            requests.into_iter().next().expect("one expected request")
        });
        (origin, single_request)
    }

    fn start_current_issue_http_fixture_for_requests(
        expected_requests: usize,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        assert!(expected_requests > 0);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let body = serde_json::to_vec(&json!({
            "number": 41,
            "state": "open",
            "updated_at": "2026-10-22T16:00:00Z",
            "html_url": format!("{origin}/forge-admin/agent-forge-core/issues/41")
        }))
        .unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut requests = Vec::with_capacity(expected_requests);
            while requests.len() < expected_requests {
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "adapter made too few HTTP requests"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept API fixture request: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 512];
                while request.len() < 16 * 1024
                    && !request.windows(4).any(|window| window == b"\r\n\r\n")
                {
                    match stream.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => request.extend_from_slice(&buffer[..count]),
                        Err(error) => panic!("read adapter request headers: {error}"),
                    }
                }
                assert!(
                    request.windows(4).any(|window| window == b"\r\n\r\n"),
                    "adapter request headers are incomplete or oversized"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
                requests.push(String::from_utf8(request).unwrap());
            }
            requests
        });
        (origin, server)
    }

    fn start_batch_cache_http_fixture(
        expected_requests: usize,
    ) -> (String, thread::JoinHandle<Vec<(String, usize)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let response_padding = "x".repeat(MAX_RECONCILE_BATCH_CACHED_JSON_BYTES / 16);
        let server_origin = origin.clone();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut requests = Vec::with_capacity(expected_requests);
            while requests.len() < expected_requests {
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "adapter made too few HTTP requests for the cache boundary"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept batch cache API fixture request: {error}"),
                    }
                };
                let (request_line, _) = read_fixture_http_request(&mut stream);
                let number = request_line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|target| target.rsplit('/').next())
                    .and_then(|value| value.parse::<u64>().ok())
                    .expect("issue request ends in a numeric issue number");
                let body = serde_json::to_vec(&json!({
                    "number": number,
                    "state": "open",
                    "updated_at": "2026-10-22T16:00:00Z",
                    "html_url": format!(
                        "{server_origin}/forge-admin/agent-forge-core/issues/{number}"
                    ),
                    "cache_fixture": response_padding,
                }))
                .expect("serialize bounded loopback issue response");
                assert!(
                    body.len() <= MAX_RESPONSE_BYTES,
                    "every individual fixture response stays below the HTTP response cap"
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
                requests.push((request_line, body.len()));
            }
            requests
        });
        (origin, server)
    }

    const ADMISSION_BASE_COMMIT: &str = "c9d4bd2a70d10f4bc5d233d37318a8b0d42a0001";
    const ADMISSION_REQUIREMENTS: &str = r#"{"schema_version":2,"id":"REQ-001","revision":1,"source_issue_number":41,"request":"Implement a duration parser.","acceptance":["250ms parses to 250 milliseconds."],"ambiguities":[]}"#;

    struct FixtureMissionAdmissionSource {
        origin: String,
        issue_state: Cell<MissionIssueStateV1>,
        issue_reads: Cell<u32>,
        requirement_reads: Cell<u32>,
    }

    impl FixtureMissionAdmissionSource {
        fn new(origin: String, issue_state: MissionIssueStateV1) -> Self {
            Self {
                origin,
                issue_state: Cell::new(issue_state),
                issue_reads: Cell::new(0),
                requirement_reads: Cell::new(0),
            }
        }
    }

    impl MissionAdmissionSourceV1 for FixtureMissionAdmissionSource {
        fn read_issue(
            &self,
            scope: &BaseBranchScope,
            issue_number: u64,
        ) -> Result<IssueSnapshotV1, SourceFailureV1> {
            self.issue_reads.set(self.issue_reads.get() + 1);
            if scope.origin() != self.origin
                || issue_number != 41
                || scope.owner() != LOCAL_REPOSITORY_OWNER
                || scope.repository() != LOCAL_REPOSITORY_NAME
            {
                return Err(SourceFailureV1::NotFound);
            }
            Ok(IssueSnapshotV1 {
                number: issue_number,
                html_url: format!(
                    "{}/{}/{}/issues/{issue_number}",
                    self.origin, LOCAL_REPOSITORY_OWNER, LOCAL_REPOSITORY_NAME
                ),
                state: self.issue_state.get(),
                work_item_content: None,
            })
        }

        fn read_protected_branch_head(
            &self,
            scope: &BaseBranchScope,
        ) -> Result<ProtectedBranchHeadV1, SourceFailureV1> {
            if scope.origin() != self.origin
                || scope.owner() != LOCAL_REPOSITORY_OWNER
                || scope.repository() != LOCAL_REPOSITORY_NAME
            {
                return Err(SourceFailureV1::Forbidden);
            }
            Ok(ProtectedBranchHeadV1 {
                branch: scope.branch().to_owned(),
                commit: ADMISSION_BASE_COMMIT.to_owned(),
                protected: true,
            })
        }

        fn read_requirement_file_at(
            &self,
            scope: &BaseBranchScope,
            commit: &str,
            path: &str,
        ) -> Result<Vec<u8>, SourceFailureV1> {
            self.requirement_reads.set(self.requirement_reads.get() + 1);
            if scope.origin() != self.origin
                || commit != ADMISSION_BASE_COMMIT
                || path != "requirements/REQ-001.json"
            {
                return Err(SourceFailureV1::Forbidden);
            }
            Ok(ADMISSION_REQUIREMENTS.as_bytes().to_vec())
        }

        fn list_requirement_paths_at(
            &self,
            scope: &BaseBranchScope,
            commit: &str,
        ) -> Result<Vec<String>, SourceFailureV1> {
            if scope.origin() != self.origin || commit != ADMISSION_BASE_COMMIT {
                return Err(SourceFailureV1::Forbidden);
            }
            Ok(vec!["requirements/REQ-001.json".to_owned()])
        }
    }

    fn admission_context() -> ContextPacketV1 {
        let text = "Inspect src/lib.rs:1 before working on the duration parser.".to_owned();
        ContextPacketV1 {
            manifest: ContextManifest {
                schema_version: 1,
                source_commit: ADMISSION_BASE_COMMIT.to_owned(),
                requirement_sha256: sha256(ADMISSION_REQUIREMENTS.as_bytes()),
                token_budget: 100,
                estimated_tokens: 20,
                sources: vec![ContextSource {
                    path: "src/lib.rs".to_owned(),
                    line_start: 1,
                    line_end: 1,
                    file_sha256: "b".repeat(64),
                    provenance: format!("git:{ADMISSION_BASE_COMMIT}:src/lib.rs#L1"),
                }],
            },
            citations: vec![ContextCitationV1 {
                text_start_byte: 0,
                text_end_byte: text.len(),
                source_index: 0,
            }],
            text,
        }
    }

    fn admission_capabilities() -> RuntimeCapabilitiesV1 {
        RuntimeCapabilitiesV1 {
            read_paths: vec![
                "requirements/REQ-001.json".to_owned(),
                "src/lib.rs".to_owned(),
            ],
            write_paths: vec!["src/lib.rs".to_owned()],
            tools: vec!["workspace.read_text".to_owned(), "cargo.test".to_owned()],
            budget: RuntimeBudgetV1 {
                max_tool_calls: 4,
                timeout_ms: 10_000,
                max_output_tokens: 1000,
            },
        }
    }

    fn api_for_origin(origin: String) -> LocalLoopbackForgejoApi {
        LocalLoopbackForgejoApi::new(LocalForgejoConfigV1 {
            origin,
            owner: LOCAL_REPOSITORY_OWNER.to_owned(),
            repository: LOCAL_REPOSITORY_NAME.to_owned(),
        })
    }

    fn api_for_origin_with_policy(
        origin: String,
        policy: LocalApiRetryPolicy,
    ) -> LocalLoopbackForgejoApi {
        LocalLoopbackForgejoApi::with_policy(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            policy,
        )
    }

    fn issue_resource() -> ResourceRef {
        ResourceRef {
            kind: ResourceKind::Issue,
            owner: LOCAL_REPOSITORY_OWNER.to_owned(),
            repository: LOCAL_REPOSITORY_NAME.to_owned(),
            id: ResourceId::Number(11),
        }
    }

    struct CountingReader {
        remaining: usize,
        bytes_read: usize,
    }

    impl Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.remaining);
            buffer[..count].fill(b'x');
            self.remaining -= count;
            self.bytes_read += count;
            Ok(count)
        }
    }

    #[test]
    fn local_adapter_refuses_repositories_outside_the_sandbox_allowlist() {
        let resource = ResourceRef {
            kind: ResourceKind::Issue,
            owner: "other-owner".into(),
            repository: "other-repo".into(),
            id: ResourceId::Number(1),
        };

        assert_eq!(
            LocalLoopbackForgejoApi::default().read_current(&resource),
            Err(ApiReadError::Forbidden)
        );
    }

    #[test]
    fn local_adapter_retries_transient_http_failure_then_returns_current_state() {
        let (origin, server) =
            start_http_fixture(vec![(503, Duration::ZERO), (200, Duration::ZERO)]);
        let api = api_for_origin(origin);

        let current = api.read_current(&issue_resource());

        assert_eq!(current.unwrap(), json!({"number": 11}));
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 2);
        assert_eq!(server.join().unwrap(), 2);
    }

    #[test]
    fn local_adapter_uses_host_credential_provider_for_loopback_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "request ended before headers completed");
                request.extend_from_slice(&buffer[..count]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"number\":11}",
                )
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let credential = "synthetic-token-0123456789";
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(credential)),
        );

        assert_eq!(
            api.read_current(&issue_resource()),
            Ok(json!({"number": 11}))
        );
        let request = server.join().unwrap();
        assert!(request.contains("GET /api/v1/repos/forge-admin/agent-forge-core/issues/11 "));
        assert!(request.contains(&format!("Authorization: token {credential}\r\n")));
    }

    #[test]
    fn reconcile_invocation_accepts_a_host_pipe_fd_and_fixed_config() {
        let scratch = ScratchJournalDir::new();
        let config_path = scratch.0.join("adapter.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "origin": LOCAL_FORGEJO_ORIGIN,
                "owner": LOCAL_REPOSITORY_OWNER,
                "repository": LOCAL_REPOSITORY_NAME,
            }))
            .unwrap(),
        )
        .unwrap();

        for arguments in [
            vec![
                "--config".into(),
                config_path.as_os_str().to_owned(),
                RECONCILE_HOST_CREDENTIAL_FD_FLAG.into(),
                "9".into(),
                "inbox.jsonl".into(),
                "delivery-1".into(),
            ],
            vec![
                RECONCILE_HOST_CREDENTIAL_FD_FLAG.into(),
                "9".into(),
                "--config".into(),
                config_path.as_os_str().to_owned(),
                "inbox.jsonl".into(),
                "delivery-1".into(),
            ],
        ] {
            let invocation = parse_reconcile_invocation(arguments).unwrap();
            assert_eq!(invocation.config.origin, LOCAL_FORGEJO_ORIGIN);
            assert_eq!(invocation.config.owner, LOCAL_REPOSITORY_OWNER);
            assert_eq!(invocation.config.repository, LOCAL_REPOSITORY_NAME);
            assert_eq!(invocation.credential_fd, Some(9));
            assert_eq!(invocation.journal_path, "inbox.jsonl");
            assert_eq!(invocation.delivery_ids, ["delivery-1"]);
        }
    }

    #[test]
    fn reconcile_invocation_accepts_bounded_canonical_attestation_keys() {
        let first = SigningKey::from_bytes(&[0x41; 32]).verifying_key();
        let second = SigningKey::from_bytes(&[0x42; 32]).verifying_key();
        let first_hex = hex::encode(first.as_bytes());
        let second_hex = hex::encode(second.as_bytes());
        let invocation = parse_reconcile_invocation([
            TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG.into(),
            first_hex.clone().into(),
            TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG.into(),
            second_hex.into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ])
        .unwrap();
        assert_eq!(invocation.trusted_attestation_keys, [first, second]);

        let duplicate = parse_reconcile_invocation([
            TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG.into(),
            first_hex.clone().into(),
            TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG.into(),
            first_hex.clone().into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(duplicate, Err(message) if message.contains("must be unique")));

        let uppercase = first_hex.to_ascii_uppercase();
        let noncanonical = parse_reconcile_invocation([
            TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG.into(),
            uppercase.into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(noncanonical, Err(message) if message.contains("canonical lowercase")));

        let mut too_many = Vec::new();
        for seed in 0x50..0x59 {
            too_many.push(TRUSTED_WEBHOOK_ATTESTATION_PUBLIC_KEY_FLAG.into());
            too_many.push(
                hex::encode(
                    SigningKey::from_bytes(&[seed; 32])
                        .verifying_key()
                        .as_bytes(),
                )
                .into(),
            );
        }
        too_many.push("inbox.jsonl".into());
        too_many.push("delivery-1".into());
        let over_limit = parse_reconcile_invocation(too_many);
        assert!(matches!(over_limit, Err(message) if message.contains("at most 8")));
    }

    #[test]
    fn reconcile_invocation_accepts_the_mission_012_target_profile() {
        let invocation = parse_reconcile_invocation([
            "--target-profile".into(),
            "mission-012-clamp".into(),
            RECONCILE_HOST_CREDENTIAL_FD_FLAG.into(),
            "9".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ])
        .unwrap();

        assert_eq!(invocation.config.origin, MISSION_012_FORGEJO_ORIGIN);
        assert_eq!(invocation.config.owner, MISSION_012_REPOSITORY_OWNER);
        assert_eq!(invocation.config.repository, MISSION_012_REPOSITORY_NAME);
        assert_eq!(invocation.credential_fd, Some(9));
        assert_eq!(invocation.journal_path, "inbox.jsonl");
        assert_eq!(invocation.delivery_ids, ["delivery-1"]);
    }

    #[test]
    fn reconcile_invocation_accepts_the_org_team_reviewer_read_only_profile() {
        let invocation = parse_reconcile_invocation([
            "--reconcile-target-profile".into(),
            "org-team-review-e2e".into(),
            RECONCILE_HOST_CREDENTIAL_FD_FLAG.into(),
            "9".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ])
        .unwrap();

        assert_eq!(invocation.config.origin, LOCAL_FORGEJO_ORIGIN);
        assert_eq!(invocation.config.owner, "forge-team-org");
        assert_eq!(invocation.config.repository, "team-review-readback");
        assert_eq!(invocation.credential_fd, Some(9));
        assert_eq!(invocation.journal_path, "inbox.jsonl");
        assert_eq!(invocation.delivery_ids, ["delivery-1"]);
    }

    #[test]
    fn reconcile_invocation_rejects_unknown_duplicate_and_combined_reconcile_profiles() {
        let unknown = parse_reconcile_invocation([
            "--reconcile-target-profile".into(),
            "http://127.0.0.1:3131/forge-team-org/team-review-readback".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(
            matches!(unknown, Err(message) if message.contains("unknown Forgejo reconciliation profile"))
        );

        let duplicate = parse_reconcile_invocation([
            "--reconcile-target-profile".into(),
            "org-team-review-e2e".into(),
            "--reconcile-target-profile".into(),
            "org-team-review-e2e".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(duplicate, Err(message) if message.contains("only once")));

        let with_legacy_profile = parse_reconcile_invocation([
            "--reconcile-target-profile".into(),
            "org-team-review-e2e".into(),
            "--target-profile".into(),
            "mission-012-clamp".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(
            matches!(with_legacy_profile, Err(message) if message.contains("cannot be combined"))
        );

        let scratch = ScratchJournalDir::new();
        let config_path = scratch.0.join("adapter.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "origin": LOCAL_FORGEJO_ORIGIN,
                "owner": LOCAL_REPOSITORY_OWNER,
                "repository": LOCAL_REPOSITORY_NAME,
            }))
            .unwrap(),
        )
        .unwrap();
        let with_config = parse_reconcile_invocation([
            "--reconcile-target-profile".into(),
            "org-team-review-e2e".into(),
            "--config".into(),
            config_path.as_os_str().to_owned(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(with_config, Err(message) if message.contains("cannot be combined")));
    }

    #[test]
    fn reconcile_invocation_rejects_ambiguous_or_unknown_target_profiles() {
        let duplicate = parse_reconcile_invocation([
            "--target-profile".into(),
            "mission-012-clamp".into(),
            "--target-profile".into(),
            "merge-014-agent-forge-core".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(duplicate, Err(message) if message.contains("only once")));

        let unknown = parse_reconcile_invocation([
            "--target-profile".into(),
            "http://127.0.0.1:3132".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(
            matches!(unknown, Err(message) if message.contains("unknown Forgejo target profile"))
        );

        let scratch = ScratchJournalDir::new();
        let config_path = scratch.0.join("adapter.json");
        fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "origin": LOCAL_FORGEJO_ORIGIN,
                "owner": LOCAL_REPOSITORY_OWNER,
                "repository": LOCAL_REPOSITORY_NAME,
            }))
            .unwrap(),
        )
        .unwrap();
        let combined = parse_reconcile_invocation([
            "--config".into(),
            config_path.as_os_str().to_owned(),
            "--target-profile".into(),
            "mission-012-clamp".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(combined, Err(message) if message.contains("cannot be combined")));
    }

    #[test]
    fn reconcile_invocation_rejects_duplicate_or_standard_credential_fds() {
        let duplicate = parse_reconcile_invocation([
            RECONCILE_HOST_CREDENTIAL_FD_FLAG.into(),
            "9".into(),
            RECONCILE_HOST_CREDENTIAL_FD_FLAG.into(),
            "10".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(duplicate, Err(message) if message.contains("only once")));

        let standard_fd = parse_reconcile_invocation([
            RECONCILE_HOST_CREDENTIAL_FD_FLAG.into(),
            "2".into(),
            "inbox.jsonl".into(),
            "delivery-1".into(),
        ]);
        assert!(matches!(standard_fd, Err(message) if message.contains("above stderr")));
    }

    #[test]
    fn synthetic_credential_is_not_added_to_curl_arguments_or_environment() {
        let credential = "synthetic-token-0123456789";
        let token = ForgejoApiToken::parse(credential.as_bytes().to_vec()).unwrap();
        let (command, stdin) = build_curl_request(
            "http://127.0.0.1:3130",
            "http://127.0.0.1:3130/api/v1/repos/forge-admin/agent-forge-core/issues/11",
            Duration::from_secs(1),
            Some(&token),
        )
        .unwrap();

        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().all(|argument| !argument.contains(credential)));
        assert!(command.get_envs().all(|(key, value)| {
            !key.to_string_lossy().contains(credential)
                && value
                    .map(|value| !value.to_string_lossy().contains(credential))
                    .unwrap_or(true)
        }));
        assert_eq!(
            stdin.unwrap(),
            format!(r#"header = "Authorization: token {credential}"{}"#, '\n').into_bytes()
        );
    }

    #[test]
    fn host_credential_pipe_read_is_bounded_when_writer_stays_open() {
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let read_fd = descriptors[0];
        let write_fd = descriptors[1];
        let observed_fd = unsafe { libc::dup(read_fd) };
        assert!(observed_fd >= 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            let result = read_inherited_credential_pipe(read_fd);
            sender.send(result).unwrap();
        });

        let result = receiver.recv_timeout(Duration::from_secs(3));
        let status_flags = unsafe { libc::fcntl(observed_fd, libc::F_GETFL) };
        unsafe { libc::close(observed_fd) };
        unsafe { libc::close(write_fd) };
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                reader.join().unwrap();
                panic!("credential-pipe reader did not return within its bound: {error}");
            }
        };
        reader.join().unwrap();

        assert!(matches!(result, Err(message) if message.contains("timed out")));
        assert_ne!(status_flags & libc::O_NONBLOCK, 0);
    }

    #[test]
    fn host_credential_pipe_retries_when_duplicate_reader_drains_after_poll() {
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let read_fd = descriptors[0];
        let write_fd = descriptors[1];
        let rival_fd = unsafe { libc::dup(read_fd) };
        assert!(rival_fd >= 0);
        let byte = [b'x'];
        assert_eq!(
            unsafe { libc::write(write_fd, byte.as_ptr().cast(), byte.len()) },
            1
        );
        let mut file = unsafe { File::from_raw_fd(read_fd) };
        let mut drained_by_rival = false;

        let result =
            read_bounded_pipe_until_eof_after_poll(&mut file, Duration::from_millis(25), || {
                if !drained_by_rival {
                    let mut rival_byte = [0];
                    assert_eq!(
                        unsafe { libc::read(rival_fd, rival_byte.as_mut_ptr().cast(), 1) },
                        1
                    );
                    drained_by_rival = true;
                }
            });

        unsafe { libc::close(rival_fd) };
        unsafe { libc::close(write_fd) };
        assert!(drained_by_rival);
        assert!(matches!(result, Err(message) if message.contains("timed out")));
    }

    #[test]
    fn host_credential_reader_rejects_named_fifo() {
        let scratch = ScratchJournalDir::new();
        let path = scratch.0.join("named-credential-fifo");
        let path_c = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path_c.as_ptr(), 0o600) }, 0);
        let fd = unsafe {
            libc::open(
                path_c.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        assert!(fd >= 0);

        let result = read_inherited_credential_pipe(fd);

        assert!(matches!(result, Err(message) if message.contains("anonymous pipe")));
    }

    #[test]
    fn host_merge_post_uses_bound_credential_stdin_and_parses_total_count() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            let header_end = loop {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "merge request ended before body was received");
                request.extend_from_slice(&buffer[..count]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
                assert!(
                    request.len() < 16 * 1024,
                    "merge request headers are oversized"
                );
            };
            let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|length| length.trim().parse::<usize>().ok())
                })
                .expect("content length header");
            while request.len() - header_end < content_length {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "merge request body is truncated");
                request.extend_from_slice(&buffer[..count]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nX-Total-Count: 1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .unwrap();
            (
                headers,
                request[header_end..header_end + content_length].to_vec(),
            )
        });
        let credential = "synthetic-token-0123456789";
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(credential)),
        );
        let body = br#"{"Do":"merge","head_commit_id":"1111111111111111111111111111111111111111"}"#;

        let response = api
            .request_api_once(
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                "POST",
                Some(body),
                Duration::from_secs(2),
            )
            .unwrap();
        let (headers, request_body) = server.join().unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.total_count, Some(1));
        assert_eq!(response.body, Some(json!({})));
        assert!(
            headers.starts_with("POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge ")
        );
        assert!(headers.contains(&format!("Authorization: token {credential}\r\n")));
        assert_eq!(request_body, body);

        let token = ForgejoApiToken::parse(credential.as_bytes().to_vec()).unwrap();
        let url = format!(
            "{}/api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge",
            api.config.origin
        );
        let (command, stdin) = build_curl_api_request(
            &api.config.origin,
            &url,
            "POST",
            Some(body),
            Duration::from_secs(1),
            Some(&token),
            true,
        )
        .unwrap();
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|argument| argument == "POST"));
        assert!(
            args.iter()
                .any(|argument| argument == std::str::from_utf8(body).unwrap())
        );
        assert!(args.iter().all(|argument| !argument.contains(credential)));
        assert_eq!(
            stdin.unwrap(),
            format!(r#"header = "Authorization: token {credential}"{}"#, '\n').into_bytes()
        );
    }

    #[test]
    fn merge_lock_root_is_stable_outside_tmpdir() {
        assert_eq!(
            merge_lock_root_for_uid(1234),
            PathBuf::from("/tmp/agent-forge-merge-locks-1234")
        );
    }

    #[test]
    fn final_main_read_compares_the_live_branch_head_to_the_bound_base() {
        fn check(reported_sha: &str, expected_sha: &str) -> (bool, String) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let reported_sha = reported_sha.to_owned();
            let expected_sha = expected_sha.to_owned();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 512];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert_ne!(count, 0, "main branch request ended before headers");
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    assert!(request.len() < 16 * 1024, "request headers are oversized");
                }
                let request_line = String::from_utf8(request).unwrap();
                let response =
                    serde_json::to_vec(&json!({"commit": {"id": reported_sha}})).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                )
                .unwrap();
                stream.write_all(&response).unwrap();
                request_line.lines().next().unwrap().to_owned()
            });
            let api = api_for_origin(origin);
            let matches = main_commit_matches_validated_base(&api, &expected_sha).unwrap();
            (matches, server.join().unwrap())
        }

        let base_sha = "1111111111111111111111111111111111111111";
        let advanced_sha = "2222222222222222222222222222222222222222";
        let (matches_base, request) = check(base_sha, base_sha);
        assert!(matches_base);
        assert_eq!(
            request,
            "GET /api/v1/repos/forge-admin/agent-forge-core/branches/main HTTP/1.1"
        );

        let (matches_advanced, request) = check(advanced_sha, base_sha);
        assert!(!matches_advanced);
        assert_eq!(
            request,
            "GET /api/v1/repos/forge-admin/agent-forge-core/branches/main HTTP/1.1"
        );
    }

    #[test]
    fn final_merge_state_refuses_an_advanced_base_without_a_merge_post() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let expected_origin = origin.clone();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            let base_sha = "1111111111111111111111111111111111111111";
            let head_sha = "2222222222222222222222222222222222222222";
            let advanced_sha = "3333333333333333333333333333333333333333";
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 512];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert_ne!(count, 0, "final-state request ended before headers");
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    assert!(request.len() < 16 * 1024, "request headers are oversized");
                }
                let request_text = String::from_utf8(request).unwrap();
                let request_line = request_text.lines().next().unwrap().to_owned();
                let response = if request_line
                    .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 ")
                {
                    json!({
                        "number": 23,
                        "html_url": format!("{expected_origin}/forge-admin/agent-forge-core/pulls/23"),
                        "state": "open",
                        "merged": false,
                        "draft": false,
                        "mergeable": true,
                        "base": {
                            "ref": "main", "sha": base_sha,
                            "repo": {"full_name": "forge-admin/agent-forge-core"}
                        },
                        "head": {
                            "sha": head_sha,
                            "repo": {"full_name": "forge-admin/agent-forge-core"}
                        }
                    })
                } else if request_line
                    .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/branches/main ")
                {
                    json!({"commit": {"id": advanced_sha}})
                } else {
                    panic!("unexpected final-state request: {request_line}");
                };
                let response = serde_json::to_vec(&response).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                )
                .unwrap();
                stream.write_all(&response).unwrap();
                requests.push(request_line);
            }
            requests
        });

        let expected = PullRequestSnapshotV1 {
            origin: origin.clone(),
            owner: LOCAL_REPOSITORY_OWNER.to_owned(),
            repository: LOCAL_REPOSITORY_NAME.to_owned(),
            number: 23,
            state: "open".to_owned(),
            merged: false,
            draft: false,
            mergeable: Some(true),
            base_branch: "main".to_owned(),
            base_sha: "1111111111111111111111111111111111111111".to_owned(),
            head_sha: "2222222222222222222222222222222222222222".to_owned(),
        };
        let api = api_for_origin(origin);
        let mut post_called = false;
        let final_state = validate_then_post_merge_with_host_check(
            &api,
            &api,
            &expected,
            &json!({}),
            b"{}",
            || Ok(()),
            |_, _, _| {
                post_called = true;
                MergePostReadbackV1 {
                    post_attempted: true,
                    post_status: Some(200),
                    post_error: None,
                    readback: Err(ApiReadError::Unavailable),
                }
            },
        );
        assert!(matches!(final_state, Err(FinalMergeStateError::StaleBase)));
        assert!(
            !post_called,
            "stale base must stop before the merge callback"
        );
        assert_eq!(
            server.join().unwrap(),
            [
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/branches/main HTTP/1.1",
            ]
        );
    }

    #[test]
    fn final_merge_state_runtime_error_preserves_oversized_response_details() {
        let error = final_merge_state_runtime_error(FinalMergeStateError::Api(
            ApiReadError::ResponseTooLarge {
                component: "header_json",
                maximum_bytes: MAX_CURL_HEADER_JSON_BYTES,
            },
        ));

        assert_eq!(error.code, "upstream_response_too_large");
        assert!(!error.retryable);
        assert!(error.action.contains("reduce the requested resource"));
        assert_eq!(error.details["upstream"], "forgejo");
        assert_eq!(error.details["component"], "header_json");
        assert_eq!(error.details["maximum_bytes"], MAX_CURL_HEADER_JSON_BYTES);
    }

    fn exact_merge_readback_for_recovery() -> (&'static str, &'static str, MergeReadbackV1) {
        let base_sha = "0000000000000000000000000000000000000000";
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        (
            base_sha,
            head_sha,
            (
                json!({
                    "number": 23,
                    "merged": true,
                    "state": "closed",
                    "head": {"sha": head_sha},
                    "base": {
                        "ref": "main",
                        "sha": merge_sha,
                        "repo": {"full_name": "forge-admin/agent-forge-core"}
                    },
                    "merge_commit_sha": merge_sha,
                }),
                json!({
                    "sha": merge_sha,
                    "parents": [{"sha": base_sha}, {"sha": head_sha}],
                }),
                json!({"sha": head_sha}),
                json!({"commit": {"id": merge_sha}}),
                json!({"number": 23, "state": "closed"}),
                Some(vec![(
                    "src/lib.rs".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "3333333333333333333333333333333333333333".to_owned(),
                    12,
                )]),
                Some(vec![(
                    "src/lib.rs".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "3333333333333333333333333333333333333333".to_owned(),
                    12,
                )]),
            ),
        )
    }

    #[test]
    fn readback_only_recovery_accepts_exact_merged_state() {
        let (base_sha, head_sha, readback) = exact_merge_readback_for_recovery();

        let (result, success, resolution) = finalize_readback_only(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            base_sha,
            head_sha,
            Ok(readback),
        );

        assert!(success);
        assert_eq!(result["verdict"], "merged");
        assert_eq!(result["merge_attempted"], false);
        assert_eq!(result["merge_attempt_recovery"], true);
        assert_eq!(result["merge_post_attempt_status"], "unknown");
        assert_eq!(
            resolution.outcome,
            agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Merged
        );
        assert_eq!(
            resolution.merge_commit.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        assert!(resolution.merge_tree_manifest_sha256.is_some());
        assert!(resolution.observation_sha256.is_some());
    }

    #[test]
    fn readback_only_recovery_refuses_an_open_pr() {
        let base_sha = "0000000000000000000000000000000000000000";
        let head_sha = "1111111111111111111111111111111111111111";
        let open_readback = (
            json!({
                "number": 23,
                "merged": false,
                "state": "open",
                "head": {"sha": head_sha},
                "base": {"ref": "main", "sha": base_sha},
            }),
            Value::Null,
            Value::Null,
            json!({"commit": {"id": base_sha}}),
            json!({"number": 23, "state": "open"}),
            None,
            None,
        );

        let (result, success, resolution) = finalize_readback_only(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            base_sha,
            head_sha,
            Ok(open_readback),
        );

        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(
            resolution.outcome,
            agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Inconclusive
        );
        assert_eq!(
            resolution.reason_code.as_deref(),
            Some("merge_outcome_not_confirmed")
        );
    }

    #[test]
    fn readback_only_recovery_refuses_each_changed_binding() {
        let (base_sha, head_sha, exact) = exact_merge_readback_for_recovery();
        let mut divergent = Vec::new();

        let mut changed = exact.clone();
        changed.0["head"]["sha"] = json!("4444444444444444444444444444444444444444");
        divergent.push(changed);

        let mut changed = exact.clone();
        changed.1["parents"][0]["sha"] = json!("5555555555555555555555555555555555555555");
        divergent.push(changed);

        let mut changed = exact.clone();
        changed.3["commit"]["id"] = json!("6666666666666666666666666666666666666666");
        divergent.push(changed);

        let mut changed = exact.clone();
        changed.4["state"] = json!("open");
        divergent.push(changed);

        let mut changed = exact.clone();
        changed.5 = Some(vec![(
            "src/lib.rs".to_owned(),
            "100644".to_owned(),
            "blob".to_owned(),
            "7777777777777777777777777777777777777777".to_owned(),
            12,
        )]);
        divergent.push(changed);

        for readback in divergent {
            let (result, success, resolution) = finalize_readback_only(
                23,
                LOCAL_REPOSITORY_OWNER,
                LOCAL_REPOSITORY_NAME,
                base_sha,
                head_sha,
                Ok(readback),
            );
            assert!(!success);
            assert_eq!(result["verdict"], "inconclusive");
            assert_eq!(
                resolution.outcome,
                agent_forge_core::agent_runtime::MergeAttemptOutcomeV1::Inconclusive
            );
        }
    }

    #[test]
    fn ambiguous_merge_post_is_resolved_by_readback_without_retry() {
        fn read_http_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            let header_end = loop {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "HTTP request ended before its headers");
                request.extend_from_slice(&buffer[..count]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
                assert!(
                    request.len() < 16 * 1024,
                    "HTTP request headers are oversized"
                );
            };
            let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|length| length.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while request.len() - header_end < content_length {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "HTTP request body is truncated");
                request.extend_from_slice(&buffer[..count]);
            }
            let request_line = headers.lines().next().unwrap().to_owned();
            (
                request_line,
                request[header_end..header_end + content_length].to_vec(),
            )
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let head_sha = "1111111111111111111111111111111111111111";
            let merge_sha = "2222222222222222222222222222222222222222";
            let mut requests = Vec::new();
            let mut merge_body = Vec::new();
            for _ in 0..8 {
                let accept_deadline = Instant::now() + Duration::from_secs(5);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < accept_deadline, "HTTP fixture timed out");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("HTTP fixture accept failed: {error}"),
                    }
                };
                let (request_line, body) = read_http_request(&mut stream);
                requests.push(request_line.clone());
                if request_line.starts_with("POST ") {
                    merge_body = body;
                    // Simulate Forgejo committing the merge and losing the response.
                    drop(stream);
                    continue;
                }
                let response_body = if request_line
                    .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 ")
                {
                    json!({
                        "number": 23,
                        "merged": true,
                        "state": "closed",
                        "head": {"sha": head_sha},
                        "base": {"ref": "main", "sha": merge_sha, "repo": {"full_name": "forge-admin/agent-forge-core"}},
                        "merge_commit_sha": merge_sha,
                    })
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/2222222222222222222222222222222222222222 ",
                ) {
                    json!({
                        "sha": merge_sha,
                        "parents": [{"sha": "0000000000000000000000000000000000000000"}, {"sha": head_sha}],
                    })
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/1111111111111111111111111111111111111111 ",
                ) {
                    json!({"sha": head_sha})
                } else if request_line
                    .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/branches/main ")
                {
                    json!({"commit": {"id": merge_sha}})
                } else if request_line
                    .starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/issues/23 ")
                {
                    json!({"number": 23, "state": "closed"})
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/2222222222222222222222222222222222222222?",
                ) {
                    json!({
                        "sha": merge_sha,
                        "tree": [{"path": "src", "mode": "100644", "type": "blob", "size": 1, "sha": "3333333333333333333333333333333333333333"}],
                        "truncated": false, "page": 1, "total_count": 1,
                    })
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/1111111111111111111111111111111111111111?",
                ) {
                    json!({
                        "sha": head_sha,
                        "tree": [{"path": "src", "mode": "100644", "type": "blob", "size": 1, "sha": "3333333333333333333333333333333333333333"}],
                        "truncated": false, "page": 1, "total_count": 1,
                    })
                } else {
                    panic!("unexpected HTTP request: {request_line}");
                };
                let response_body = serde_json::to_vec(&response_body).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response_body.len()
                )
                .unwrap();
                stream.write_all(&response_body).unwrap();
            }
            (requests, merge_body)
        });
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(
                "synthetic-token-0123456789",
            )),
        );
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_body = serde_json::to_vec(&json!({
            "Do": "merge",
            "head_commit_id": head_sha,
        }))
        .unwrap();

        let outcome = post_merge_once_then_readback(&api, 23, &merge_body);
        assert!(outcome.post_attempted);
        assert_eq!(outcome.post_status, None);
        assert_eq!(outcome.post_error, Some(ApiReadError::Unavailable));
        let (
            merged_pr,
            merge_commit,
            candidate_commit,
            merged_main,
            merged_issue,
            merge_tree_entries,
            candidate_tree_entries,
        ) = outcome.readback.unwrap();
        assert!(merge_readback_is_confirmed(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            head_sha,
            &merged_pr,
            &merged_main,
            &merged_issue,
        ));
        assert!(merge_commit_parent_binding_is_confirmed(
            "0000000000000000000000000000000000000000",
            head_sha,
            &merged_pr,
            &merge_commit,
        ));
        assert!(
            merge_commit_tree_manifest_binding_sha256(
                &merged_pr,
                &candidate_commit,
                &merge_commit,
                merge_tree_entries.as_deref(),
                candidate_tree_entries.as_deref(),
            )
            .is_some()
        );

        let (requests, merge_body) = server.join().unwrap();
        assert_eq!(
            requests,
            [
                "POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/2222222222222222222222222222222222222222 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/1111111111111111111111111111111111111111 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/branches/main HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/issues/23 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/2222222222222222222222222222222222222222?page=1&per_page=1000&recursive=false HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/1111111111111111111111111111111111111111?page=1&per_page=1000&recursive=false HTTP/1.1",
            ]
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&merge_body).unwrap(),
            json!({"Do": "merge", "head_commit_id": head_sha})
        );
    }

    #[test]
    fn postmerge_readback_rejects_duplicate_merged_flag_before_followup_reads() {
        let (origin, server) = start_http_fixture_with_bodies(vec![(
            200,
            Duration::ZERO,
            br#"{"number":23,"merged":true,"merged":false}"#.to_vec(),
        )]);
        let api = api_for_origin(origin);

        let readback = merged_readback(&api, 23);

        assert!(matches!(readback, Err(ApiReadError::InvalidResponse)));
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn postmerge_parent_mismatch_is_inconclusive_after_single_post() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let head_sha = "1111111111111111111111111111111111111111";
            let merge_sha = "2222222222222222222222222222222222222222";
            let wrong_base_sha = "3333333333333333333333333333333333333333";
            let mut requests = Vec::new();
            for _ in 0..8 {
                let deadline = Instant::now() + Duration::from_secs(5);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "HTTP fixture timed out");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("HTTP fixture accept failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert_ne!(count, 0, "HTTP request ended before its headers");
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    assert!(request.len() < 16 * 1024, "HTTP headers are oversized");
                }
                let request_text = String::from_utf8_lossy(&request);
                let request_line = request_text.lines().next().unwrap().to_owned();
                requests.push(request_line.clone());
                let response_body = if request_line.starts_with("POST ") {
                    json!({"merged": true})
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 ",
                ) {
                    json!({
                        "number": 23,
                        "merged": true,
                        "state": "closed",
                        "head": {"sha": head_sha},
                        "base": {"ref": "main", "sha": merge_sha, "repo": {"full_name": "forge-admin/agent-forge-core"}},
                        "merge_commit_sha": merge_sha,
                    })
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/2222222222222222222222222222222222222222 ",
                ) {
                    json!({
                        "sha": merge_sha,
                        "parents": [{"sha": wrong_base_sha}, {"sha": head_sha}],
                    })
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/1111111111111111111111111111111111111111 ",
                ) {
                    json!({"sha": head_sha})
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/branches/main ",
                ) {
                    json!({"commit": {"id": merge_sha}})
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/issues/23 ",
                ) {
                    json!({"number": 23, "state": "closed"})
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/2222222222222222222222222222222222222222?",
                ) {
                    json!({
                        "sha": merge_sha,
                        "tree": [{"path": "src", "mode": "100644", "type": "blob", "size": 1, "sha": "3333333333333333333333333333333333333333"}],
                        "truncated": false, "page": 1, "total_count": 1,
                    })
                } else if request_line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/1111111111111111111111111111111111111111?",
                ) {
                    json!({
                        "sha": head_sha,
                        "tree": [{"path": "src", "mode": "100644", "type": "blob", "size": 1, "sha": "3333333333333333333333333333333333333333"}],
                        "truncated": false, "page": 1, "total_count": 1,
                    })
                } else {
                    panic!("unexpected HTTP request: {request_line}");
                };
                let response_body = serde_json::to_vec(&response_body).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response_body.len()
                )
                .unwrap();
                stream.write_all(&response_body).unwrap();
            }
            requests
        });
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(
                "synthetic-token-0123456789",
            )),
        );
        let head_sha = "1111111111111111111111111111111111111111";
        let validated_base_sha = "0000000000000000000000000000000000000000";
        let merge_body = serde_json::to_vec(&json!({
            "Do": "merge",
            "head_commit_id": head_sha,
        }))
        .unwrap();

        let outcome = post_merge_once_then_readback(&api, 23, &merge_body);
        assert!(outcome.post_attempted);
        assert_eq!(outcome.post_status, Some(200));
        assert!(outcome.post_error.is_none());
        let (result, success) = finalize_merge_post_readback(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            validated_base_sha,
            head_sha,
            outcome,
        );
        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_attempted"], true);
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["error"]["code"], "merge_outcome_not_confirmed");
        assert_eq!(result["error"]["retryable"], true);
        assert_eq!(result["http_status"], 200);

        let requests = server.join().unwrap();
        assert_eq!(
            requests[0],
            "POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge HTTP/1.1"
        );
        assert_eq!(
            requests,
            [
                "POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/2222222222222222222222222222222222222222 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/commits/1111111111111111111111111111111111111111 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/branches/main HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/issues/23 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/2222222222222222222222222222222222222222?page=1&per_page=1000&recursive=false HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/1111111111111111111111111111111111111111?page=1&per_page=1000&recursive=false HTTP/1.1",
            ]
        );
        assert_eq!(
            requests
                .iter()
                .filter(|line| line.starts_with("POST "))
                .count(),
            1
        );
    }

    #[test]
    fn ambiguous_merge_post_with_unavailable_readback_is_not_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let expected = [
                "POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1",
            ];
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut requests = Vec::new();
            for _ in expected {
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "HTTP fixture timed out");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("HTTP fixture accept failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 4096];
                let count = stream.read(&mut request).unwrap();
                assert_ne!(count, 0, "HTTP request must include its request line");
                let request_line = String::from_utf8_lossy(&request[..count])
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                requests.push(request_line);
                // Closing without response models an uncertain POST followed
                // by an unavailable authenticated readback.
            }
            requests
        });
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(
                "synthetic-token-0123456789",
            )),
        );
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_body = serde_json::to_vec(&json!({
            "Do": "merge",
            "head_commit_id": head_sha,
        }))
        .unwrap();

        let outcome = post_merge_once_then_readback(&api, 23, &merge_body);
        assert!(outcome.post_attempted);
        assert_eq!(outcome.post_status, None);
        assert_eq!(outcome.post_error, Some(ApiReadError::Unavailable));
        let readback_error = outcome.readback.unwrap_err();
        assert_eq!(readback_error, ApiReadError::Unavailable);
        let (result, success) =
            merge_post_readback_failure_result(23, outcome.post_status, readback_error);
        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_attempted"], true);
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["error"]["code"], "upstream_unavailable");
        assert_eq!(result["error"]["retryable"], true);
        let requests = server.join().unwrap();
        assert_eq!(
            requests,
            [
                "POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1",
            ]
        );
    }

    #[test]
    fn merge_readback_refuses_divergent_commit_and_issue_state() {
        let submitted_head = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        let valid = || {
            (
                json!({
                    "number": 23,
                    "merged": true,
                    "state": "closed",
                    "head": {"sha": submitted_head},
                    "base": {
                        "ref": "main",
                        "sha": merge_sha,
                        "repo": {"full_name": "forge-admin/agent-forge-core"}
                    },
                    "merge_commit_sha": merge_sha,
                }),
                json!({"commit": {"id": merge_sha}}),
                json!({"number": 23, "state": "closed"}),
            )
        };
        let (pull, main, issue) = valid();
        assert!(merge_readback_is_confirmed(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            submitted_head,
            &pull,
            &main,
            &issue,
        ));

        let mut cases = Vec::new();
        let (mut pull, main, issue) = valid();
        pull["head"]["sha"] = json!("3333333333333333333333333333333333333333");
        cases.push((pull, main, issue));
        let (mut pull, main, issue) = valid();
        pull["number"] = json!(24);
        cases.push((pull, main, issue));
        let (mut pull, main, issue) = valid();
        pull["base"]["ref"] = json!("release");
        cases.push((pull, main, issue));
        let (mut pull, main, issue) = valid();
        pull["base"]["sha"] = json!("3333333333333333333333333333333333333333");
        cases.push((pull, main, issue));
        let (mut pull, main, issue) = valid();
        pull["base"]["repo"]["full_name"] = json!("other-owner/other-repository");
        cases.push((pull, main, issue));
        let (mut pull, main, issue) = valid();
        pull["base"].as_object_mut().unwrap().remove("repo");
        cases.push((pull, main, issue));
        let (pull, mut main, issue) = valid();
        main["commit"]["id"] = json!("3333333333333333333333333333333333333333");
        cases.push((pull, main, issue));
        let (mut pull, main, issue) = valid();
        pull["merge_commit_sha"] = json!("3333333333333333333333333333333333333333");
        cases.push((pull, main, issue));
        let (pull, main, mut issue) = valid();
        issue["state"] = json!("open");
        cases.push((pull, main, issue));
        let (pull, main, mut issue) = valid();
        issue["number"] = json!(24);
        cases.push((pull, main, issue));

        for (pull, main, issue) in cases {
            assert!(!merge_readback_is_confirmed(
                23,
                LOCAL_REPOSITORY_OWNER,
                LOCAL_REPOSITORY_NAME,
                submitted_head,
                &pull,
                &main,
                &issue,
            ));
        }
    }

    #[test]
    fn confirmed_merge_result_includes_matching_candidate_tree_manifest_bindings() {
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        let post_outcome = MergePostReadbackV1 {
            post_attempted: true,
            post_status: Some(200),
            post_error: None,
            readback: Ok((
                json!({
                    "number": 23,
                    "merged": true,
                    "state": "closed",
                    "head": {"sha": head_sha},
                    "base": {"ref": "main", "sha": merge_sha, "repo": {"full_name": "forge-admin/agent-forge-core"}},
                    "merge_commit_sha": merge_sha,
                }),
                json!({
                    "sha": merge_sha,
                    "parents": [
                        {"sha": "0000000000000000000000000000000000000000"},
                        {"sha": head_sha},
                    ],
                }),
                json!({"sha": head_sha}),
                json!({"commit": {"id": merge_sha}}),
                json!({"number": 23, "state": "closed"}),
                Some(vec![(
                    "src".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "3333333333333333333333333333333333333333".to_owned(),
                    12,
                )]),
                Some(vec![(
                    "src".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "3333333333333333333333333333333333333333".to_owned(),
                    12,
                )]),
            )),
        };

        let (result, success) = finalize_merge_post_readback(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            "0000000000000000000000000000000000000000",
            head_sha,
            post_outcome,
        );

        assert!(success);
        assert_eq!(result["verdict"], "merged");
        let candidate_manifest_sha = result["reviewed_candidate_tree_entries_sha256"].as_str();
        let merge_manifest_sha = result["merge_commit_tree_entries_sha256"].as_str();
        let is_sha256 = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        assert!(candidate_manifest_sha.is_some_and(is_sha256));
        assert_eq!(candidate_manifest_sha, merge_manifest_sha);
    }

    #[test]
    fn postmerge_tree_mismatch_is_inconclusive_with_valid_merge_parents() {
        let base_sha = "0000000000000000000000000000000000000000";
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        let outcome = MergePostReadbackV1 {
            post_attempted: true,
            post_status: Some(200),
            post_error: None,
            readback: Ok((
                json!({
                    "number": 23,
                    "merged": true,
                    "state": "closed",
                    "head": {"sha": head_sha},
                    "base": {"ref": "main", "sha": merge_sha, "repo": {"full_name": "forge-admin/agent-forge-core"}},
                    "merge_commit_sha": merge_sha,
                }),
                json!({
                    "sha": merge_sha,
                    "parents": [{"sha": base_sha}, {"sha": head_sha}],
                }),
                json!({"sha": head_sha}),
                json!({"commit": {"id": merge_sha}}),
                json!({"number": 23, "state": "closed"}),
                Some(vec![(
                    "src".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "3333333333333333333333333333333333333333".to_owned(),
                    1,
                )]),
                Some(vec![(
                    "src".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "4444444444444444444444444444444444444444".to_owned(),
                    1,
                )]),
            )),
        };

        let (result, success) = finalize_merge_post_readback(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            base_sha,
            head_sha,
            outcome,
        );
        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_attempted"], true);
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["error"]["code"], "merge_outcome_not_confirmed");
        assert_eq!(result["http_status"], 200);
    }

    #[test]
    fn merged_readback_with_tree_mismatch_and_rejected_post_is_inconclusive() {
        let base_sha = "0000000000000000000000000000000000000000";
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        let outcome = MergePostReadbackV1 {
            post_attempted: true,
            post_status: Some(409),
            post_error: None,
            readback: Ok((
                json!({
                    "number": 23,
                    "merged": true,
                    "state": "closed",
                    "head": {"sha": head_sha},
                    "base": {"ref": "main", "sha": merge_sha, "repo": {"full_name": "forge-admin/agent-forge-core"}},
                    "merge_commit_sha": merge_sha,
                }),
                json!({
                    "sha": merge_sha,
                    "parents": [{"sha": base_sha}, {"sha": head_sha}],
                }),
                json!({"sha": head_sha}),
                json!({"commit": {"id": merge_sha}}),
                json!({"number": 23, "state": "closed"}),
                Some(vec![(
                    "src".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "3333333333333333333333333333333333333333".to_owned(),
                    1,
                )]),
                Some(vec![(
                    "src".to_owned(),
                    "100644".to_owned(),
                    "blob".to_owned(),
                    "4444444444444444444444444444444444444444".to_owned(),
                    1,
                )]),
            )),
        };

        let (result, success) = finalize_merge_post_readback(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            base_sha,
            head_sha,
            outcome,
        );

        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_attempted"], true);
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["error"]["code"], "merge_outcome_not_confirmed");
        assert_eq!(result["http_status"], 409);
    }

    #[test]
    fn postmerge_missing_tree_manifest_is_inconclusive() {
        let base_sha = "0000000000000000000000000000000000000000";
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        let outcome = MergePostReadbackV1 {
            post_attempted: true,
            post_status: Some(200),
            post_error: None,
            readback: Ok((
                json!({
                    "number": 23,
                    "merged": true,
                    "state": "closed",
                    "head": {"sha": head_sha},
                    "base": {"ref": "main", "sha": merge_sha, "repo": {"full_name": "forge-admin/agent-forge-core"}},
                    "merge_commit_sha": merge_sha,
                }),
                json!({
                    "sha": merge_sha,
                    "parents": [{"sha": base_sha}, {"sha": head_sha}],
                }),
                json!({"sha": head_sha}),
                json!({"commit": {"id": merge_sha}}),
                json!({"number": 23, "state": "closed"}),
                Some(vec![]),
                None,
            )),
        };

        let (result, success) = finalize_merge_post_readback(
            23,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            base_sha,
            head_sha,
            outcome,
        );
        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["error"]["code"], "merge_outcome_not_confirmed");
    }

    #[test]
    fn forgejo_tree_manifest_reads_all_pages_with_v17_truncated_semantics() {
        let commit_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let entries_for = |start: usize, end: usize| {
            (start..end)
                .map(|index| {
                    json!({
                        "path": format!("entry-{index:04}"),
                        "mode": "100644",
                        "type": "blob",
                        "size": 0,
                        "sha": format!("{index:040x}"),
                    })
                })
                .collect::<Vec<_>>()
        };
        let responses = [
            json!({
                "sha": commit_sha,
                "page": 1,
                "total_count": 1001,
                "truncated": true,
                "tree": entries_for(0, 1000),
            }),
            json!({
                "sha": commit_sha,
                "page": 2,
                "total_count": 1001,
                // Forgejo v17 reports truncation from total entries > per_page,
                // including on the final partial page.
                "truncated": true,
                "tree": entries_for(1000, 1001),
            }),
        ]
        .into_iter()
        .map(|body| (200, Duration::ZERO, serde_json::to_vec(&body).unwrap()))
        .collect();
        let (origin, server, requests) =
            start_http_fixture_with_bodies_recording_requests(responses);
        let api = api_for_origin(origin);

        let entries = forgejo_commit_tree_entries(&api, commit_sha).unwrap();

        assert_eq!(entries.len(), 1001);
        assert_eq!(entries.first().unwrap().0, "entry-0000");
        assert_eq!(entries.last().unwrap().0, "entry-1000");
        assert_eq!(server.join().unwrap(), 2);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/{commit_sha}?page=1&per_page=1000&recursive=false HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/{commit_sha}?page=2&per_page=1000&recursive=false HTTP/1.1"
                ),
            ]
        );
    }

    #[test]
    fn forgejo_tree_manifest_accepts_v17_null_tree_only_for_an_empty_root() {
        let commit_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let empty_response = json!({
            "sha": commit_sha,
            "page": 1,
            "total_count": 0,
            "truncated": false,
            "tree": null,
        });
        let (origin, server, requests) = start_http_fixture_with_bodies_recording_requests(vec![(
            200,
            Duration::ZERO,
            serde_json::to_vec(&empty_response).unwrap(),
        )]);
        let api = api_for_origin(origin);

        assert_eq!(
            forgejo_commit_tree_entries(&api, commit_sha).unwrap(),
            Vec::new()
        );
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(requests.lock().unwrap().len(), 1);

        let inconsistent_response = json!({
            "sha": commit_sha,
            "page": 1,
            "total_count": 1,
            "truncated": false,
            "tree": null,
        });
        let (origin, server, requests) = start_http_fixture_with_bodies_recording_requests(vec![(
            200,
            Duration::ZERO,
            serde_json::to_vec(&inconsistent_response).unwrap(),
        )]);
        let api = api_for_origin(origin);

        assert_eq!(
            forgejo_commit_tree_entries(&api, commit_sha),
            Err(ApiReadError::InvalidResponse)
        );
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn forgejo_tree_manifest_refuses_a_missing_later_page() {
        let commit_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let first_page = json!({
            "sha": commit_sha,
            "page": 1,
            "total_count": 1001,
            "truncated": true,
            "tree": (0..1000).map(|index| json!({
                "path": format!("entry-{index:04}"),
                "mode": "100644",
                "type": "blob",
                "size": 0,
                "sha": format!("{index:040x}"),
            })).collect::<Vec<_>>(),
        });
        let responses = vec![
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&first_page).unwrap(),
            ),
            (404, Duration::ZERO, br#"{"message":"not found"}"#.to_vec()),
        ];
        let (origin, server, requests) =
            start_http_fixture_with_bodies_recording_requests(responses);
        let api = api_for_origin(origin);

        assert!(forgejo_commit_tree_entries(&api, commit_sha).is_err());
        assert_eq!(server.join().unwrap(), 2);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/{commit_sha}?page=1&per_page=1000&recursive=false HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/git/trees/{commit_sha}?page=2&per_page=1000&recursive=false HTTP/1.1"
                ),
            ]
        );
    }

    #[test]
    fn unmerged_pull_request_readback_does_not_require_a_merge_commit() {
        let responses = vec![
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&json!({
                    "number": 23,
                    "merged": false,
                    "state": "open",
                    "merge_commit_sha": null,
                }))
                .unwrap(),
            ),
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(
                    &json!({"commit": {"id": "1111111111111111111111111111111111111111"}}),
                )
                .unwrap(),
            ),
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&json!({"number": 23, "state": "open"})).unwrap(),
            ),
        ];
        let (origin, server) = start_http_fixture_with_bodies(responses);
        let api = api_for_origin(origin);

        let (pull, commit, candidate, _main, issue, merge_tree, candidate_tree) =
            merged_readback(&api, 23).unwrap();
        assert_eq!(pull["merged"], false);
        assert!(commit.is_null());
        assert!(candidate.is_null());
        assert!(merge_tree.is_none());
        assert!(candidate_tree.is_none());
        assert_eq!(issue["state"], "open");
        assert_eq!(server.join().unwrap(), 3);
    }

    #[test]
    fn merge_commit_must_have_exact_validated_base_and_submitted_head_parents() {
        let base_sha = "0000000000000000000000000000000000000000";
        let head_sha = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        let pull = json!({"merge_commit_sha": merge_sha});
        let valid = || {
            json!({
                "sha": merge_sha,
                "parents": [{"sha": base_sha}, {"sha": head_sha}],
            })
        };
        assert!(merge_commit_parent_binding_is_confirmed(
            base_sha,
            head_sha,
            &pull,
            &valid(),
        ));

        let wrong_base = json!({
            "sha": merge_sha,
            "parents": [{"sha": head_sha}, {"sha": base_sha}],
        });
        let wrong_head = json!({
            "sha": merge_sha,
            "parents": [{"sha": base_sha}, {"sha": "3333333333333333333333333333333333333333"}],
        });
        let extra_parent = json!({
            "sha": merge_sha,
            "parents": [{"sha": base_sha}, {"sha": head_sha}, {"sha": "3333333333333333333333333333333333333333"}],
        });
        let mismatched_commit =
            json!({"sha": head_sha, "parents": [{"sha": base_sha}, {"sha": head_sha}]});
        let malformed_parent =
            json!({"sha": merge_sha, "parents": [{"sha": "../main"}, {"sha": head_sha}]});

        for commit in [
            wrong_base,
            wrong_head,
            extra_parent,
            mismatched_commit,
            malformed_parent,
        ] {
            assert!(!merge_commit_parent_binding_is_confirmed(
                base_sha, head_sha, &pull, &commit,
            ));
        }
    }

    #[test]
    fn merged_readback_rejects_a_different_pull_request_number_before_issue_read() {
        let (origin, server) = start_current_issue_http_fixture();
        let api = api_for_origin(origin);

        let result = merged_readback(&api, 23);
        let request = server.join().unwrap();
        assert!(
            request.contains("GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1")
        );
        assert_eq!(result.unwrap_err(), ApiReadError::InvalidResponse);
    }

    #[test]
    fn merged_readback_rejects_a_different_issue_number_after_fetching_the_issue() {
        let submitted_head = "1111111111111111111111111111111111111111";
        let merge_sha = "2222222222222222222222222222222222222222";
        let responses = vec![
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&json!({
                    "number": 23,
                    "merged": true,
                    "state": "closed",
                    "head": {"sha": submitted_head},
                    "base": {"ref": "main", "sha": merge_sha, "repo": {"full_name": "forge-admin/agent-forge-core"}},
                    "merge_commit_sha": merge_sha,
                }))
                .unwrap(),
            ),
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&json!({
                    "sha": merge_sha,
                    "parents": [
                        {"sha": "0000000000000000000000000000000000000000"},
                        {"sha": submitted_head},
                    ],
                }))
                .unwrap(),
            ),
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&json!({"sha": submitted_head})).unwrap(),
            ),
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&json!({"commit": {"id": merge_sha}})).unwrap(),
            ),
            (
                200,
                Duration::ZERO,
                serde_json::to_vec(&json!({"number": 24, "state": "closed"})).unwrap(),
            ),
        ];
        let (origin, server) = start_http_fixture_with_bodies(responses);
        let api = api_for_origin(origin);

        assert_eq!(
            merged_readback(&api, 23),
            Err(ApiReadError::InvalidResponse)
        );
        assert_eq!(server.join().unwrap(), 5);
    }

    #[test]
    fn token_parser_rejects_empty_oversized_and_header_injection_values() {
        let invalid_values = [
            Vec::new(),
            b"token with spaces".to_vec(),
            b"token\r\nX-Injected: yes".to_vec(),
            b"bad=padding-position".to_vec(),
            vec![b'a'; MAX_FORGEJO_API_TOKEN_BYTES + 1],
        ];

        for value in invalid_values {
            assert!(matches!(
                ForgejoApiToken::parse(value),
                Err(ApiReadError::InvalidCredential)
            ));
        }
    }

    #[test]
    fn origin_bound_provider_withholds_token_for_a_different_origin() {
        let provider = OriginBoundHostCredentialProvider {
            origin: "http://127.0.0.1:3130".to_owned(),
            provider: Box::new(FixtureHostCredentialProvider::new(
                "synthetic-token-0123456789",
            )),
        };

        assert!(
            provider
                .token_for("http://127.0.0.1:3131")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn local_adapter_rejects_foreign_request_url_before_sending_host_credential() {
        let (foreign_origin, server) = start_http_fixture(vec![(200, Duration::ZERO)]);
        let credential_reads = Arc::new(AtomicUsize::new(0));
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin: "http://127.0.0.1:3130".to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(CountingHostCredentialProvider {
                token: "synthetic-token-0123456789".to_owned(),
                calls: Arc::clone(&credential_reads),
            }),
        );
        let foreign_url = format!(
            "{foreign_origin}/api/v1/repos/{}/{}/issues/11",
            LOCAL_REPOSITORY_OWNER, LOCAL_REPOSITORY_NAME
        );

        assert_eq!(
            api.read_current_once(&foreign_url, Duration::from_secs(1))
                .map_err(|failure| failure.error),
            Err(ApiReadError::Forbidden)
        );
        assert_eq!(credential_reads.load(Ordering::Relaxed), 0);
        assert_eq!(server.join().unwrap(), 0);
    }

    #[test]
    fn reconciliation_policy_rejects_write_method_and_nested_issue_route_before_network() {
        let (origin, server) = start_http_fixture(vec![
            (200, Duration::ZERO),
            (200, Duration::ZERO),
            (200, Duration::ZERO),
        ]);
        let api = reconciliation_api(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            Some(ForgejoApiToken::parse(b"synthetic-token-0123456789".to_vec()).unwrap()),
        );

        let post = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/issues/11",
            "POST",
            Some(b"{}"),
            Duration::from_secs(1),
        );
        let nested_issue_route = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/issues/11/labels",
            "GET",
            None,
            Duration::from_secs(1),
        );
        let allowed_issue_read = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/issues/11",
            "GET",
            None,
            Duration::from_secs(1),
        );
        let served = server.join().unwrap();

        assert!(matches!(post, Err(ApiReadError::Forbidden)));
        assert!(matches!(nested_issue_route, Err(ApiReadError::Forbidden)));
        assert_eq!(allowed_issue_read.map(|response| response.status), Ok(200));
        assert_eq!(served, 1, "only the permitted issue read may connect");
    }

    #[test]
    fn reconciliation_policy_allows_only_canonical_encoded_branch_segments() {
        let api = LocalLoopbackForgejoApi::default();
        let allowed =
            "/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2Fagent-proof";
        assert!(api.reconciliation_request_is_allowed(allowed, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(allowed));

        for invalid in [
            "/repos/forge-admin/agent-forge-core/branches/feature%2frelease",
            "/repos/forge-admin/agent-forge-core/branches/%2E%2E",
            "/repos/forge-admin/agent-forge-core/branches/feature%2Frelease/extra",
            "/repos/another-owner/agent-forge-core/branches/feature%2Frelease",
            "/repos/forge-admin/agent-forge-core/branches/feature%ZZrelease",
        ] {
            assert!(
                !api.reconciliation_request_is_allowed(invalid, "GET", None),
                "unexpectedly allowed {invalid}"
            );
            assert!(
                !api.request_path_stays_in_configured_repository(invalid),
                "unexpectedly accepted {invalid} into the repository scope"
            );
        }
    }

    #[test]
    fn reconciliation_policy_allows_only_exact_issue_comment_read_route() {
        let api = LocalLoopbackForgejoApi::default();
        let allowed = "/repos/forge-admin/agent-forge-core/issues/comments/916";
        assert!(api.reconciliation_request_is_allowed(allowed, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(allowed));

        for invalid in [
            "/repos/forge-admin/agent-forge-core/issues/comments/0",
            "/repos/forge-admin/agent-forge-core/issues/comments/0916",
            "/repos/forge-admin/agent-forge-core/issues/comments/916/",
            "/repos/forge-admin/agent-forge-core/issues/comments/916/reactions",
            "/repos/another-owner/agent-forge-core/issues/comments/916",
            "/repos/forge-admin/agent-forge-core/issues/comments/916?redirect=1",
        ] {
            assert!(
                !api.reconciliation_request_is_allowed(invalid, "GET", None),
                "unexpectedly allowed {invalid}"
            );
            if invalid.contains("another-owner") || invalid.ends_with("/916/") {
                assert!(!api.request_path_stays_in_configured_repository(invalid));
            } else {
                assert!(api.request_path_stays_in_configured_repository(invalid));
            }
        }
        assert!(!api.reconciliation_request_is_allowed(allowed, "POST", None));
        assert!(!api.reconciliation_request_is_allowed(allowed, "GET", Some(b"{}")));
    }

    #[test]
    fn reconciliation_policy_allows_only_exact_workflow_run_read_route() {
        let api = LocalLoopbackForgejoApi::default();
        let allowed = "/repos/forge-admin/agent-forge-core/actions/runs/7001";
        assert!(api.reconciliation_request_is_allowed(allowed, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(allowed));

        for invalid in [
            "/repos/forge-admin/agent-forge-core/actions/runs/0",
            "/repos/forge-admin/agent-forge-core/actions/runs/07001",
            "/repos/forge-admin/agent-forge-core/actions/runs/7001/",
            "/repos/forge-admin/agent-forge-core/actions/runs/7001/logs",
            "/repos/another-owner/agent-forge-core/actions/runs/7001",
            "/repos/forge-admin/other-repo/actions/runs/7001",
            "/repos/forge-admin/agent-forge-core/actions/runs/7001?redirect=1",
            "/repos/forge-admin/agent-forge-core/actions/runs/7001#fragment",
        ] {
            assert!(
                !api.reconciliation_request_is_allowed(invalid, "GET", None),
                "unexpectedly allowed {invalid}"
            );
        }
        assert!(!api.reconciliation_request_is_allowed(allowed, "POST", None));
        assert!(!api.reconciliation_request_is_allowed(allowed, "GET", Some(b"{}")));
    }

    #[test]
    fn reconciliation_policy_allows_only_exact_workflow_job_read_route() {
        let api = LocalLoopbackForgejoApi::default();
        let allowed = "/repos/forge-admin/agent-forge-core/actions/jobs/9001";
        assert!(api.reconciliation_request_is_allowed(allowed, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(allowed));

        for invalid in [
            "/repos/forge-admin/agent-forge-core/actions/jobs/0",
            "/repos/forge-admin/agent-forge-core/actions/jobs/09001",
            "/repos/forge-admin/agent-forge-core/actions/jobs/9001/",
            "/repos/forge-admin/agent-forge-core/actions/jobs/9001/logs",
            "/repos/another-owner/agent-forge-core/actions/jobs/9001",
            "/repos/forge-admin/other-repo/actions/jobs/9001",
            "/repos/forge-admin/agent-forge-core/actions/jobs/9001?attempt=1",
            "/repos/forge-admin/agent-forge-core/actions/jobs/9001#fragment",
        ] {
            assert!(
                !api.reconciliation_request_is_allowed(invalid, "GET", None),
                "unexpectedly allowed {invalid}"
            );
        }
        assert!(!api.reconciliation_request_is_allowed(allowed, "POST", None));
        assert!(!api.reconciliation_request_is_allowed(allowed, "GET", Some(b"{}")));
    }

    #[test]
    fn reconciliation_policy_allows_only_exact_release_read_route() {
        let api = LocalLoopbackForgejoApi::default();
        let allowed = "/repos/forge-admin/agent-forge-core/releases/610";
        assert!(api.reconciliation_request_is_allowed(allowed, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(allowed));

        for invalid in [
            "/repos/forge-admin/agent-forge-core/releases/0",
            "/repos/forge-admin/agent-forge-core/releases/0610",
            "/repos/forge-admin/agent-forge-core/releases/610/",
            "/repos/forge-admin/agent-forge-core/releases/610/assets",
            "/repos/another-owner/agent-forge-core/releases/610",
            "/repos/forge-admin/other-repo/releases/610",
            "/repos/forge-admin/agent-forge-core/releases/610?redirect=1",
            "/repos/forge-admin/agent-forge-core/releases/610#fragment",
        ] {
            assert!(
                !api.reconciliation_request_is_allowed(invalid, "GET", None),
                "unexpectedly allowed {invalid}"
            );
        }
        assert!(!api.reconciliation_request_is_allowed(allowed, "POST", None));
        assert!(!api.reconciliation_request_is_allowed(allowed, "GET", Some(b"{}")));
    }

    #[test]
    fn local_reconciliation_adapter_reads_workflow_run_by_internal_id() {
        let body = br#"{"id":7001,"index_in_repo":41,"status":"failure"}"#.to_vec();
        let (origin, server, requests) =
            start_http_fixture_with_bodies_recording_requests(vec![(200, Duration::ZERO, body)]);
        let mut api = api_for_origin(origin.clone());
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;
        let resource = ResourceRef {
            kind: ResourceKind::WorkflowRun,
            owner: LOCAL_REPOSITORY_OWNER.to_owned(),
            repository: LOCAL_REPOSITORY_NAME.to_owned(),
            id: ResourceId::WorkflowRun {
                run_id: 7001,
                index_in_repo: 41,
            },
        };

        let current = api
            .read_current(&resource)
            .expect("read canonical workflow run");
        assert_eq!(current["status"], "failure");
        assert_eq!(
            api.expected_html_url(&resource),
            format!(
                "{origin}/{}/{}/actions/runs/41",
                LOCAL_REPOSITORY_OWNER, LOCAL_REPOSITORY_NAME
            )
        );
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(
            *requests.lock().unwrap(),
            vec![format!(
                "GET /api/v1/repos/{}/{}/actions/runs/7001 HTTP/1.1",
                LOCAL_REPOSITORY_OWNER, LOCAL_REPOSITORY_NAME
            )]
        );
    }

    #[test]
    fn local_reconciliation_adapter_reads_workflow_job_by_internal_id() {
        let body = br#"{"id":9001,"run_id":7001,"status":"failure"}"#.to_vec();
        let (origin, server, requests) =
            start_http_fixture_with_bodies_recording_requests(vec![(200, Duration::ZERO, body)]);
        let mut api = api_for_origin(origin.clone());
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;
        let resource = ResourceRef {
            kind: ResourceKind::WorkflowJob,
            owner: LOCAL_REPOSITORY_OWNER.to_owned(),
            repository: LOCAL_REPOSITORY_NAME.to_owned(),
            id: ResourceId::WorkflowJob {
                job_id: 9001,
                run_id: 7001,
                index_in_repo: 41,
                job_index: 1,
            },
        };

        let current = api
            .read_current(&resource)
            .expect("read canonical workflow job");
        assert_eq!(current["status"], "failure");
        assert_eq!(
            api.expected_html_url(&resource),
            format!(
                "{origin}/{}/{}/actions/runs/41",
                LOCAL_REPOSITORY_OWNER, LOCAL_REPOSITORY_NAME
            )
        );
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(
            *requests.lock().unwrap(),
            vec![format!(
                "GET /api/v1/repos/{}/{}/actions/jobs/9001 HTTP/1.1",
                LOCAL_REPOSITORY_OWNER, LOCAL_REPOSITORY_NAME
            )]
        );
    }

    #[test]
    fn reconciliation_policy_allows_only_exact_commit_status_snapshot_route() {
        let api = LocalLoopbackForgejoApi::default();
        let sha = "a".repeat(40);
        let allowed = format!(
            "/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit={COMMIT_STATUS_PAGE_SIZE}&page=1"
        );
        assert!(api.reconciliation_request_is_allowed(&allowed, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(&allowed));
        let second_page = format!(
            "/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit={COMMIT_STATUS_PAGE_SIZE}&page=2"
        );
        assert!(api.reconciliation_request_is_allowed(&second_page, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(&second_page));
        let final_page = format!(
            "/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit={COMMIT_STATUS_PAGE_SIZE}&page={MAX_COMMIT_STATUS_PAGES}"
        );
        assert!(api.reconciliation_request_is_allowed(&final_page, "GET", None));
        let sha256 = "b".repeat(64);
        let sha256_path = format!(
            "/repos/forge-admin/agent-forge-core/commits/{sha256}/status?limit={COMMIT_STATUS_PAGE_SIZE}&page=1"
        );
        assert!(api.reconciliation_request_is_allowed(&sha256_path, "GET", None));
        assert!(api.request_path_stays_in_configured_repository(&sha256_path));
        let uppercase_sha = "A".repeat(40);

        for invalid in [
            format!("/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=01"),
            format!("/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=0"),
            format!("/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=11"),
            format!("/repos/forge-admin/agent-forge-core/commits/{sha}/status?page=1&limit=50"),
            format!("/repos/forge-admin/agent-forge-core/commits/{sha}/statuses?limit=50&page=1"),
            format!("/repos/another-owner/agent-forge-core/commits/{sha}/status?limit=50&page=1"),
            format!(
                "/repos/forge-admin/agent-forge-core/commits/{}/status?limit=50&page=1",
                "a".repeat(39)
            ),
            format!(
                "/repos/forge-admin/agent-forge-core/commits/{}/status?limit=50&page=1",
                format!("{}z", "a".repeat(39))
            ),
            format!(
                "/repos/forge-admin/agent-forge-core/commits/{uppercase_sha}/status?limit=50&page=1"
            ),
            format!("/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=51&page=1"),
            format!("/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=02"),
        ] {
            assert!(
                !api.reconciliation_request_is_allowed(&invalid, "GET", None),
                "unexpectedly allowed {invalid}"
            );
            // Query and SHA validation belong to the exact route policy. The
            // broader repository-scope predicate intentionally checks only
            // which repository a URL names.
            if invalid.contains("another-owner") {
                assert!(
                    !api.request_path_stays_in_configured_repository(&invalid),
                    "unexpectedly accepted {invalid} into the repository scope"
                );
            } else {
                assert!(api.request_path_stays_in_configured_repository(&invalid));
            }
        }
        assert!(!api.reconciliation_request_is_allowed(&allowed, "POST", Some(b"{}")));
    }

    fn pull_request_status_fixture(
        origin: &str,
        number: u64,
        base_sha: &str,
        head_sha: &str,
    ) -> Value {
        json!({
            "number": number,
            "html_url": format!("{origin}/forge-admin/agent-forge-core/pulls/{number}"),
            "state": "open",
            "merged": false,
            "draft": false,
            "mergeable": true,
            "base": {
                "ref": "main",
                "sha": base_sha,
                "repo": {"full_name": "forge-admin/agent-forge-core"},
            },
            "head": {"sha": head_sha},
        })
    }

    #[test]
    fn pull_request_status_snapshot_binds_statuses_to_a_rechecked_head() {
        let base_sha = "b".repeat(40);
        let head_sha = "a".repeat(40);
        let (origin, server, requests) =
            start_http_fixture_with_extra_headers_recording_requests_for_origin(|origin| {
                let status_body = serde_json::to_vec(&json!({
                    "state":"success",
                    "sha":head_sha,
                    "total_count":1,
                    "statuses":[{"status":"success","context":"ci/test"}],
                    "repository":{"full_name":"forge-admin/agent-forge-core"},
                }))
                .unwrap();
                vec![
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin, 23, &base_sha, &head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        status_body,
                        vec!["X-Total-Count: 1".to_owned()],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin, 23, &base_sha, &head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                ]
            });
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, complete) = api.read_pull_request_status_snapshot(23).unwrap();

        assert!(complete);
        assert_eq!(
            snapshot["schema_version"],
            "forgejo-pull-request-status-snapshot-v1"
        );
        assert_eq!(snapshot["observation"], "complete");
        assert_eq!(snapshot["pull_request_head_rechecked_after_statuses"], true);
        assert_eq!(snapshot["pull_request_head_stable_between_reads"], true);
        assert_eq!(snapshot["current_pr_head_checked"], true);
        assert_eq!(snapshot["pull_request_binding_changed_fields"], json!([]));
        assert_eq!(snapshot["pull_request_head_before"]["head_sha"], head_sha);
        assert_eq!(snapshot["pull_request_head_after"]["head_sha"], head_sha);
        assert_eq!(snapshot["status_snapshot"]["commit_sha"], head_sha);
        assert_eq!(
            snapshot["status_snapshot"]["statuses"][0]["status"],
            "success"
        );
        assert_eq!(snapshot["independent_verification"], false);
        assert_eq!(snapshot["merge_authorizing"], false);
        assert_eq!(server.join().unwrap(), 3);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1".to_owned(),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{head_sha}/status?limit=50&page=1 HTTP/1.1"
                ),
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1".to_owned(),
            ]
        );
    }

    #[test]
    fn pull_request_status_snapshot_explains_absent_status_evidence() {
        let base_sha = "b".repeat(40);
        let head_sha = "a".repeat(40);
        let (origin, server, _requests) =
            start_http_fixture_with_extra_headers_recording_requests_for_origin(|origin| {
                vec![
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin, 23, &base_sha, &head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&json!({
                            "state":"",
                            "sha":"",
                            "total_count":0,
                            "statuses":null,
                            "repository":null,
                        }))
                        .unwrap(),
                        vec!["X-Total-Count: 0".to_owned()],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin, 23, &base_sha, &head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                ]
            });
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, read_complete) = api.read_pull_request_status_snapshot(23).unwrap();

        assert!(read_complete);
        assert_eq!(snapshot["observation"], "no_statuses");
        assert_eq!(snapshot["reason"], "no_statuses");
        assert_eq!(
            snapshot["action"],
            "no Forgejo status evidence exists for this PR head; run an independent verifier and do not report verification success"
        );
        assert_eq!(snapshot["current_pr_head_checked"], true);
        assert_eq!(snapshot["pull_request_binding_changed_fields"], json!([]));
        assert_eq!(snapshot["status_snapshot"]["observation"], "no_statuses");
        assert_eq!(snapshot["status_snapshot"]["status_count"], 0);
        assert_eq!(snapshot["independent_verification"], false);
        assert_eq!(snapshot["merge_authorizing"], false);
        assert_eq!(server.join().unwrap(), 3);
    }

    #[test]
    fn pull_request_status_snapshot_explains_incomplete_status_pages() {
        let base_sha = "b".repeat(40);
        let head_sha = "a".repeat(40);
        let (origin, server, _requests) =
            start_http_fixture_with_extra_headers_recording_requests_for_origin(|origin| {
                let mut responses = vec![(
                    200,
                    Duration::ZERO,
                    serde_json::to_vec(&pull_request_status_fixture(
                        origin, 23, &base_sha, &head_sha,
                    ))
                    .unwrap(),
                    vec![],
                )];
                for page in 1..=MAX_COMMIT_STATUS_PAGES {
                    let statuses = (0..COMMIT_STATUS_PAGE_SIZE)
                        .map(|index| {
                            json!({
                                "status":"success",
                                "context":format!("ci/{page}/{index}"),
                            })
                        })
                        .collect::<Vec<_>>();
                    let body = serde_json::to_vec(&json!({
                        "state":"success",
                        "sha":head_sha,
                        "total_count":COMMIT_STATUS_PAGE_SIZE,
                        "statuses":statuses,
                        "repository":{"full_name":"forge-admin/agent-forge-core"},
                    }))
                    .unwrap();
                    responses.push((
                        200,
                        Duration::ZERO,
                        body,
                        vec!["X-Total-Count: 501".to_owned()],
                    ));
                }
                responses.push((
                    200,
                    Duration::ZERO,
                    serde_json::to_vec(&pull_request_status_fixture(
                        origin, 23, &base_sha, &head_sha,
                    ))
                    .unwrap(),
                    vec![],
                ));
                responses
            });
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, read_complete) = api.read_pull_request_status_snapshot(23).unwrap();

        assert!(!read_complete);
        assert_eq!(snapshot["observation"], "inconclusive");
        assert_eq!(snapshot["reason"], "status_snapshot_incomplete");
        assert_eq!(
            snapshot["action"],
            "Forgejo did not provide a complete status snapshot; reread the status pages and do not treat this as verification success"
        );
        assert_eq!(snapshot["current_pr_head_checked"], true);
        assert_eq!(snapshot["pull_request_head_stable_between_reads"], true);
        assert_eq!(snapshot["pull_request_binding_changed_fields"], json!([]));
        assert_eq!(snapshot["status_snapshot"]["status_count"], 500);
        assert_eq!(snapshot["status_snapshot"]["total_count"], 501);
        assert_eq!(snapshot["status_snapshot"]["pages_read"], 10);
        assert_eq!(snapshot["independent_verification"], false);
        assert_eq!(snapshot["merge_authorizing"], false);
        assert_eq!(server.join().unwrap(), 12);
    }

    #[test]
    fn pull_request_status_snapshot_refuses_to_claim_a_head_that_changed_mid_read() {
        let base_sha = "b".repeat(40);
        let initial_head_sha = "a".repeat(40);
        let changed_head_sha = "c".repeat(40);
        let (origin, server, _requests) =
            start_http_fixture_with_extra_headers_recording_requests_for_origin(|origin| {
                let status_body = serde_json::to_vec(&json!({
                    "state":"success",
                    "sha":initial_head_sha,
                    "total_count":1,
                    "statuses":[{"status":"success","context":"ci/test"}],
                    "repository":{"full_name":"forge-admin/agent-forge-core"},
                }))
                .unwrap();
                vec![
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin,
                            23,
                            &base_sha,
                            &initial_head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        status_body,
                        vec!["X-Total-Count: 1".to_owned()],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin,
                            23,
                            &base_sha,
                            &changed_head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                ]
            });
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, complete) = api.read_pull_request_status_snapshot(23).unwrap();

        assert!(!complete);
        assert_eq!(snapshot["observation"], "inconclusive");
        assert_eq!(snapshot["reason"], "pull_request_changed_during_snapshot");
        assert_eq!(
            snapshot["action"],
            "the pull request binding changed during the read; reread the pull request and status snapshot before treating these statuses as current"
        );
        assert_eq!(snapshot["pull_request_head_rechecked_after_statuses"], true);
        assert_eq!(snapshot["pull_request_head_stable_between_reads"], false);
        assert_eq!(snapshot["current_pr_head_checked"], false);
        assert_eq!(
            snapshot["pull_request_binding_changed_fields"],
            json!(["head_sha"])
        );
        assert_eq!(
            snapshot["pull_request_head_before"]["head_sha"],
            initial_head_sha
        );
        assert_eq!(
            snapshot["pull_request_head_after"]["head_sha"],
            changed_head_sha
        );
        assert_eq!(snapshot["status_snapshot"]["commit_sha"], initial_head_sha);
        assert_eq!(snapshot["status_snapshot"]["observation"], "complete");
        assert_eq!(snapshot["independent_verification"], false);
        assert_eq!(snapshot["merge_authorizing"], false);
        assert_eq!(server.join().unwrap(), 3);
    }

    #[test]
    fn pull_request_status_snapshot_reports_base_sha_drift_with_stable_head() {
        let initial_base_sha = "b".repeat(40);
        let changed_base_sha = "c".repeat(40);
        let head_sha = "a".repeat(40);
        let (origin, server, _requests) =
            start_http_fixture_with_extra_headers_recording_requests_for_origin(|origin| {
                let status_body = serde_json::to_vec(&json!({
                    "state":"success",
                    "sha":head_sha,
                    "total_count":1,
                    "statuses":[{"status":"success","context":"ci/test"}],
                    "repository":{"full_name":"forge-admin/agent-forge-core"},
                }))
                .unwrap();
                vec![
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin,
                            23,
                            &initial_base_sha,
                            &head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        status_body,
                        vec!["X-Total-Count: 1".to_owned()],
                    ),
                    (
                        200,
                        Duration::ZERO,
                        serde_json::to_vec(&pull_request_status_fixture(
                            origin,
                            23,
                            &changed_base_sha,
                            &head_sha,
                        ))
                        .unwrap(),
                        vec![],
                    ),
                ]
            });
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, read_complete) = api.read_pull_request_status_snapshot(23).unwrap();

        assert!(!read_complete);
        assert_eq!(snapshot["observation"], "inconclusive");
        assert_eq!(snapshot["reason"], "pull_request_changed_during_snapshot");
        assert_eq!(snapshot["pull_request_head_before"]["head_sha"], head_sha);
        assert_eq!(snapshot["pull_request_head_after"]["head_sha"], head_sha);
        assert_eq!(snapshot["pull_request_head_stable_between_reads"], true);
        assert_eq!(snapshot["pull_request_binding_stable_between_reads"], false);
        assert_eq!(snapshot["current_pr_head_checked"], true);
        assert_eq!(
            snapshot["pull_request_head_before"]["base_sha"],
            initial_base_sha
        );
        assert_eq!(
            snapshot["pull_request_head_after"]["base_sha"],
            changed_base_sha
        );
        assert_eq!(
            snapshot["pull_request_binding_changed_fields"],
            json!(["base_sha"])
        );
        assert_eq!(snapshot["status_snapshot"]["commit_sha"], head_sha);
        assert_eq!(snapshot["independent_verification"], false);
        assert_eq!(snapshot["merge_authorizing"], false);
        assert_eq!(server.join().unwrap(), 3);
    }

    #[test]
    fn commit_status_snapshot_keeps_forgejo_success_consultative() {
        let sha = "a".repeat(40);
        let response = ForgejoHttpResponse {
            status: 200,
            total_count: Some(1),
            retry_after: None,
            body: Some(json!({
                "state": "success",
                "sha": sha,
                "total_count": 1,
                "statuses": [{"status": "success", "context": "ci/test"}],
                "repository": {"full_name": "forge-admin/agent-forge-core"},
            })),
        };

        let (snapshot, complete) = parse_commit_status_snapshot_response(
            &sha,
            "forge-admin",
            "agent-forge-core",
            response,
        )
        .unwrap();

        assert!(complete);
        assert_eq!(snapshot["observation"], "complete");
        assert_eq!(snapshot["repository"], "forge-admin/agent-forge-core");
        assert_eq!(snapshot["current_pr_head_checked"], false);
        assert_eq!(snapshot["forgejo_reported_combined_state"], "success");
        assert_eq!(snapshot["commit_sha"], sha);
        assert_eq!(snapshot["statuses"][0]["context"], "ci/test");
        assert_eq!(snapshot["independent_verification"], false);
        assert_eq!(snapshot["merge_authorizing"], false);
    }

    #[test]
    fn commit_status_snapshot_preserves_warning_and_skipped_as_consultative_states() {
        let sha = "a".repeat(40);
        for state in ["warning", "skipped"] {
            let response = ForgejoHttpResponse {
                status: 200,
                total_count: Some(1),
                retry_after: None,
                body: Some(json!({
                    "state": state,
                    "sha": sha,
                    "total_count": 1,
                    "statuses": [{"status": state, "context": format!("ci/{state}")}],
                    "repository": {"full_name": "forge-admin/agent-forge-core"},
                })),
            };

            let (snapshot, complete) = parse_commit_status_snapshot_response(
                &sha,
                "forge-admin",
                "agent-forge-core",
                response,
            )
            .unwrap_or_else(|error| panic!("Forgejo state {state} should parse: {error:?}"));

            assert!(complete, "single-row {state} snapshot is complete");
            assert_eq!(snapshot["observation"], "complete");
            assert_eq!(snapshot["forgejo_reported_combined_state"], state);
            assert_eq!(snapshot["statuses"][0]["status"], state);
            assert_eq!(snapshot["independent_verification"], false);
            assert_eq!(snapshot["merge_authorizing"], false);
        }
    }

    #[test]
    fn empty_commit_status_response_is_not_fabricated_as_success() {
        let sha = "a".repeat(40);
        let response = ForgejoHttpResponse {
            status: 200,
            total_count: None,
            retry_after: None,
            body: Some(json!({
                "state": "",
                "sha": "",
                "total_count": 0,
                "statuses": null,
                "repository": null,
            })),
        };

        let (snapshot, complete) = parse_commit_status_snapshot_response(
            &sha,
            "forge-admin",
            "agent-forge-core",
            response,
        )
        .unwrap();

        assert!(complete);
        assert_eq!(snapshot["observation"], "no_statuses");
        assert_eq!(snapshot["repository"], "forge-admin/agent-forge-core");
        assert_eq!(snapshot["current_pr_head_checked"], false);
        assert!(snapshot["forgejo_reported_combined_state"].is_null());
        assert_eq!(snapshot["commit_sha"], sha);
        assert_eq!(snapshot["merge_authorizing"], false);
    }

    #[test]
    fn commit_status_snapshot_rejects_duplicate_contexts_in_a_page() {
        let sha = "a".repeat(40);
        let response = ForgejoHttpResponse {
            status: 200,
            total_count: Some(2),
            retry_after: None,
            body: Some(json!({
                "state": "success",
                "sha": sha,
                "total_count": 2,
                "statuses": [
                    {"status": "success", "context": "ci/test"},
                    {"status": "success", "context": "ci/test"},
                ],
                "repository": {"full_name": "forge-admin/agent-forge-core"},
            })),
        };

        let error = parse_commit_status_snapshot_response(
            &sha,
            "forge-admin",
            "agent-forge-core",
            response,
        )
        .unwrap_err();
        assert_eq!(error, ApiReadError::InconsistentPagination);
        assert_eq!(
            commit_status_snapshot_cli_error(error),
            "Forgejo status pages changed or counts are inconsistent; reread the complete snapshot before using it"
        );
    }

    #[test]
    fn commit_status_snapshot_marks_truncated_page_inconclusive_and_rejects_wrong_scope() {
        let sha = "a".repeat(40);
        let statuses = (0..COMMIT_STATUS_PAGE_SIZE)
            .map(|index| json!({"status": "success", "context": format!("ci/{index}")}))
            .collect::<Vec<_>>();
        let response = ForgejoHttpResponse {
            status: 200,
            total_count: Some(COMMIT_STATUS_PAGE_SIZE + 1),
            retry_after: None,
            body: Some(json!({
                "state": "success",
                "sha": sha,
                "total_count": COMMIT_STATUS_PAGE_SIZE,
                "statuses": statuses,
                "repository": {"full_name": "forge-admin/agent-forge-core"},
            })),
        };

        let (snapshot, complete) = parse_commit_status_snapshot_response(
            &sha,
            "forge-admin",
            "agent-forge-core",
            response,
        )
        .unwrap();
        assert!(!complete);
        assert_eq!(snapshot["observation"], "inconclusive");
        assert_eq!(snapshot["merge_authorizing"], false);

        let wrong_sha = "b".repeat(40);
        let mismatch = ForgejoHttpResponse {
            status: 200,
            total_count: Some(1),
            retry_after: None,
            body: Some(json!({
                "state": "success",
                "sha": wrong_sha,
                "total_count": 1,
                "statuses": [{"status": "success", "context": "ci/test"}],
                "repository": {"full_name": "forge-admin/agent-forge-core"},
            })),
        };
        assert_eq!(
            parse_commit_status_snapshot_response(
                &sha,
                "forge-admin",
                "agent-forge-core",
                mismatch,
            )
            .unwrap_err(),
            ApiReadError::InvalidResponse
        );

        let false_success = ForgejoHttpResponse {
            status: 200,
            total_count: Some(0),
            retry_after: None,
            body: Some(json!({
                "state": "success",
                "sha": sha,
                "total_count": 0,
                "statuses": [],
                "repository": {"full_name": "forge-admin/agent-forge-core"},
            })),
        };
        assert_eq!(
            parse_commit_status_snapshot_response(
                &sha,
                "forge-admin",
                "agent-forge-core",
                false_success,
            )
            .unwrap_err(),
            ApiReadError::InvalidResponse
        );

        let wrong_repository = ForgejoHttpResponse {
            status: 200,
            total_count: Some(1),
            retry_after: None,
            body: Some(json!({
                "state": "success",
                "sha": sha,
                "total_count": 1,
                "statuses": [{"status": "success", "context": "ci/test"}],
                "repository": {"full_name": "someone-else/agent-forge-core"},
            })),
        };
        assert_eq!(
            parse_commit_status_snapshot_response(
                &sha,
                "forge-admin",
                "agent-forge-core",
                wrong_repository,
            )
            .unwrap_err(),
            ApiReadError::InvalidResponse
        );
    }

    #[test]
    fn commit_status_snapshot_uses_the_exact_sha_scoped_read_route() {
        let sha = "a".repeat(40);
        let body = serde_json::to_vec(&json!({
            "state": "success",
            "sha": sha,
            "total_count": 1,
            "statuses": [{"status": "success", "context": "ci/test"}],
            "repository": {"full_name": "forge-admin/agent-forge-core"},
        }))
        .unwrap();
        let (origin, server, requests) =
            start_http_fixture_with_extra_headers_recording_requests(vec![(
                200,
                Duration::ZERO,
                body,
                vec!["X-Total-Count: 1".to_owned()],
            )]);
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, complete) = api.read_commit_status_snapshot(&sha).unwrap();
        assert!(complete);
        assert_eq!(snapshot["observation"], "complete");
        assert_eq!(snapshot["repository"], "forge-admin/agent-forge-core");
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [format!(
                "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=1 HTTP/1.1"
            )]
        );
    }

    #[test]
    fn commit_status_snapshot_reads_all_bounded_pages_without_claiming_atomicity() {
        let sha = "a".repeat(40);
        let first_page = (0..COMMIT_STATUS_PAGE_SIZE)
            .map(|index| json!({"status":"success","context":format!("ci/{index}")}))
            .collect::<Vec<_>>();
        let second_page = json!({"status":"failure","context":"ci/security"});
        let first_body = serde_json::to_vec(&json!({
            "state":"success",
            "sha":sha,
            "total_count":COMMIT_STATUS_PAGE_SIZE,
            "statuses":first_page,
            "repository":{"full_name":"forge-admin/agent-forge-core"},
        }))
        .unwrap();
        let second_body = serde_json::to_vec(&json!({
            "state":"failure",
            "sha":sha,
            "total_count":1,
            "statuses":[second_page],
            "repository":{"full_name":"forge-admin/agent-forge-core"},
        }))
        .unwrap();
        let (origin, server, requests) =
            start_http_fixture_with_extra_headers_recording_requests(vec![
                (
                    200,
                    Duration::ZERO,
                    first_body.clone(),
                    vec!["X-Total-Count: 51".to_owned()],
                ),
                (
                    200,
                    Duration::ZERO,
                    second_body.clone(),
                    vec!["X-Total-Count: 51".to_owned()],
                ),
                (
                    200,
                    Duration::ZERO,
                    first_body,
                    vec!["X-Total-Count: 51".to_owned()],
                ),
                (
                    200,
                    Duration::ZERO,
                    second_body,
                    vec!["X-Total-Count: 51".to_owned()],
                ),
            ]);
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, complete) = api.read_commit_status_snapshot(&sha).unwrap();
        assert!(complete);
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
        assert_eq!(snapshot["statuses"].as_array().unwrap().len(), 51);
        assert_eq!(snapshot["statuses"][50]["context"], "ci/security");
        assert_eq!(snapshot["independent_verification"], false);
        assert_eq!(snapshot["merge_authorizing"], false);
        assert_eq!(server.join().unwrap(), 4);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=1 HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=2 HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=1 HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=2 HTTP/1.1"
                ),
            ]
        );
    }

    #[test]
    fn commit_status_snapshot_rejects_total_count_drift_between_pages() {
        let sha = "b".repeat(40);
        let first_page = (0..COMMIT_STATUS_PAGE_SIZE)
            .map(|index| json!({"status":"success","context":format!("ci/{index}")}))
            .collect::<Vec<_>>();
        let first_body = serde_json::to_vec(&json!({
            "state":"success",
            "sha":sha,
            "total_count":COMMIT_STATUS_PAGE_SIZE,
            "statuses":first_page,
            "repository":{"full_name":"forge-admin/agent-forge-core"},
        }))
        .unwrap();
        let second_body = serde_json::to_vec(&json!({
            "state":"success",
            "sha":sha,
            "total_count":1,
            "statuses":[{"status":"success","context":"ci/new"}],
            "repository":{"full_name":"forge-admin/agent-forge-core"},
        }))
        .unwrap();
        let (origin, server, requests) =
            start_http_fixture_with_extra_headers_recording_requests(vec![
                (
                    200,
                    Duration::ZERO,
                    first_body,
                    vec!["X-Total-Count: 51".to_owned()],
                ),
                (
                    200,
                    Duration::ZERO,
                    second_body,
                    vec!["X-Total-Count: 52".to_owned()],
                ),
            ]);
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let error = api.read_commit_status_snapshot(&sha).unwrap_err();
        assert_eq!(error, ApiReadError::InconsistentPagination);
        assert_eq!(
            commit_status_snapshot_cli_error(error),
            "Forgejo status pages changed or counts are inconsistent; reread the complete snapshot before using it"
        );
        assert_eq!(server.join().unwrap(), 2);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=1 HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=2 HTTP/1.1"
                ),
            ]
        );
    }

    #[test]
    fn commit_status_snapshot_rejects_constant_total_status_drift_between_pages() {
        let sha = "c".repeat(40);
        let expected_total = COMMIT_STATUS_PAGE_SIZE * 2;
        let make_page = |state: &str, contexts: Vec<Value>, page_count: usize| {
            let body = serde_json::to_vec(&json!({
                "state": state,
                "sha": sha,
                "total_count": page_count,
                "statuses": contexts,
                "repository":{"full_name":"forge-admin/agent-forge-core"},
            }))
            .unwrap();
            (
                200,
                Duration::ZERO,
                body,
                vec![format!("X-Total-Count: {expected_total}")],
            )
        };
        let first_page = (0..COMMIT_STATUS_PAGE_SIZE)
            .map(|index| {
                json!({
                    "status": "success",
                    "context": format!("ci/{index}"),
                })
            })
            .collect::<Vec<_>>();
        let second_page = (0..COMMIT_STATUS_PAGE_SIZE)
            .map(|index| {
                json!({
                    "status": if index == 1 { "failure" } else { "success" },
                    "context": format!("ci/{}", COMMIT_STATUS_PAGE_SIZE + index),
                })
            })
            .collect::<Vec<_>>();
        let changed_second_page = (0..COMMIT_STATUS_PAGE_SIZE)
            .map(|index| {
                json!({
                    "status": match index {
                        0 => "pending",
                        1 => "failure",
                        _ => "success",
                    },
                    "context": format!("ci/{}", COMMIT_STATUS_PAGE_SIZE + index),
                })
            })
            .collect::<Vec<_>>();
        let (origin, server, requests) =
            start_http_fixture_with_extra_headers_recording_requests(vec![
                make_page(
                    "success",
                    first_page.clone(),
                    COMMIT_STATUS_PAGE_SIZE as usize,
                ),
                make_page("failure", second_page, COMMIT_STATUS_PAGE_SIZE as usize),
                make_page("success", first_page, COMMIT_STATUS_PAGE_SIZE as usize),
                make_page(
                    "failure",
                    changed_second_page,
                    COMMIT_STATUS_PAGE_SIZE as usize,
                ),
            ]);
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let error = api.read_commit_status_snapshot(&sha).unwrap_err();
        assert_eq!(error, ApiReadError::InconsistentPagination);
        assert_eq!(
            commit_status_snapshot_cli_error(error),
            "Forgejo status pages changed or counts are inconsistent; reread the complete snapshot before using it"
        );
        assert_eq!(server.join().unwrap(), 4);
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=1 HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=2 HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=1 HTTP/1.1"
                ),
                format!(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/commits/{sha}/status?limit=50&page=2 HTTP/1.1"
                ),
            ]
        );
    }

    #[test]
    fn commit_status_snapshot_stops_at_the_maximum_page_budget() {
        let sha = "c".repeat(40);
        let responses = (1..=MAX_COMMIT_STATUS_PAGES)
            .map(|page| {
                let statuses = (0..COMMIT_STATUS_PAGE_SIZE)
                    .map(|index| {
                        json!({
                            "status":"success",
                            "context":format!("ci/{page}/{index}"),
                        })
                    })
                    .collect::<Vec<_>>();
                let body = serde_json::to_vec(&json!({
                    "state":"success",
                    "sha":sha,
                    "total_count":COMMIT_STATUS_PAGE_SIZE,
                    "statuses":statuses,
                    "repository":{"full_name":"forge-admin/agent-forge-core"},
                }))
                .unwrap();
                (
                    200,
                    Duration::ZERO,
                    body,
                    vec!["X-Total-Count: 501".to_owned()],
                )
            })
            .collect();
        let (origin, server, requests) =
            start_http_fixture_with_extra_headers_recording_requests(responses);
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin,
            ));
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        let (snapshot, complete) = api.read_commit_status_snapshot(&sha).unwrap();
        assert!(!complete);
        assert_eq!(snapshot["observation"], "inconclusive");
        assert_eq!(snapshot["status_count"], MAX_COMMIT_STATUS_COUNT);
        assert_eq!(snapshot["total_count"], 501);
        assert_eq!(snapshot["pages_read"], MAX_COMMIT_STATUS_PAGES);
        assert_eq!(snapshot["pagination_complete"], false);
        assert_eq!(snapshot["snapshot_consistency"], "sequential_non_atomic");
        assert_eq!(snapshot["forgejo_reported_combined_state"], Value::Null);
        assert_eq!(
            snapshot["statuses"].as_array().unwrap().len() as u64,
            MAX_COMMIT_STATUS_COUNT
        );
        assert_eq!(server.join().unwrap(), MAX_COMMIT_STATUS_PAGES as usize);
        let requests = requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("page=10 HTTP/1.1"))
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.contains("page=11 HTTP/1.1"))
        );
    }

    #[test]
    fn commit_status_cli_rejects_invalid_sha_before_reading_credential_fd() {
        let arguments = [
            std::ffi::OsString::from("not-a-full-sha"),
            std::ffi::OsString::from(RECONCILE_HOST_CREDENTIAL_FD_FLAG),
            std::ffi::OsString::from("2147483647"),
        ];
        let error = run_commit_status_snapshot(arguments.into_iter()).unwrap_err();
        assert_eq!(error, "commit SHA must be a full hexadecimal object ID");
    }

    #[test]
    fn reconciliation_policy_rejects_branch_query_before_credential_or_network() {
        let (origin, server) = start_http_fixture(vec![(200, Duration::ZERO)]);
        let credential_reads = Arc::new(AtomicUsize::new(0));
        let config = LocalForgejoConfigV1 {
            origin,
            owner: LOCAL_REPOSITORY_OWNER.to_owned(),
            repository: LOCAL_REPOSITORY_NAME.to_owned(),
        };
        let mut api = LocalLoopbackForgejoApi::with_host_credentials(
            config,
            LocalApiRetryPolicy::default(),
            Box::new(CountingHostCredentialProvider {
                token: "synthetic-token-0123456789".to_owned(),
                calls: Arc::clone(&credential_reads),
            }),
        );
        api.request_policy = LocalApiRequestPolicy::ReconciliationReadOnly;

        for path in [
            "/repos/forge-admin/agent-forge-core/branches/main?format=raw",
            "/repos/forge-admin/agent-forge-core/branches/feature%2Frelease%2Fagent-proof?format=raw",
        ] {
            assert!(
                !api.request_path_stays_in_configured_repository(path),
                "branch query unexpectedly passed repository path validation: {path}"
            );
            let query = api.request_api_once(path, "GET", None, Duration::from_secs(1));
            assert!(
                matches!(query, Err(ApiReadError::Forbidden)),
                "branch query unexpectedly reached the request layer: {path}"
            );
        }
        let served = server.join().unwrap();

        assert_eq!(credential_reads.load(Ordering::Relaxed), 0);
        assert_eq!(
            served, 0,
            "a branch query must be rejected before network I/O"
        );
    }

    #[test]
    fn merge_policy_reader_allows_only_the_exact_branch_protection_get() {
        let (origin, server) = start_http_fixture(vec![(200, Duration::ZERO)]);
        let mut api = api_for_origin(origin);
        api.request_policy = LocalApiRequestPolicy::MergePolicyReadOnly;

        let write = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/branch_protections/main",
            "PATCH",
            Some(b"{}"),
            Duration::from_secs(1),
        );
        let delete = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/branch_protections/main",
            "DELETE",
            None,
            Duration::from_secs(1),
        );
        let other_read = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/pulls/23",
            "GET",
            None,
            Duration::from_secs(1),
        );
        let get_with_body = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/branch_protections",
            "GET",
            Some(b"{}"),
            Duration::from_secs(1),
        );
        let list_read = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/branch_protections",
            "GET",
            None,
            Duration::from_secs(1),
        );
        let served = server.join().unwrap();

        assert!(matches!(write, Err(ApiReadError::Forbidden)));
        assert!(matches!(delete, Err(ApiReadError::Forbidden)));
        assert!(matches!(other_read, Err(ApiReadError::Forbidden)));
        assert!(matches!(get_with_body, Err(ApiReadError::Forbidden)));
        assert_eq!(list_read.map(|response| response.status), Ok(200));
        assert_eq!(served, 1, "only the branch-protection read may connect");
    }

    #[test]
    fn merge_authority_policy_allows_only_gate_reads_and_one_merge_route() {
        let api = LocalLoopbackForgejoApi::default();
        let body = Some(
            br#"{"Do":"merge","head_commit_id":"0123456789abcdef0123456789abcdef01234567"}"#
                .as_slice(),
        );
        let allowed = [
            ("GET", "/user", None),
            ("GET", "/users/forge-reviewer", None),
            ("GET", "/repos/forge-admin/agent-forge-core/pulls/23", None),
            (
                "GET",
                "/repos/forge-admin/agent-forge-core/pulls/23/reviews?limit=50&page=1",
                None,
            ),
            (
                "GET",
                "/repos/forge-admin/agent-forge-core/branches/main",
                None,
            ),
            ("GET", "/repos/forge-admin/agent-forge-core/issues/23", None),
            (
                "GET",
                "/repos/forge-admin/agent-forge-core/git/commits/0123456789abcdef0123456789abcdef01234567",
                None,
            ),
            (
                "GET",
                "/repos/forge-admin/agent-forge-core/git/trees/0123456789abcdef0123456789abcdef01234567?page=1&per_page=1000&recursive=false",
                None,
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                body,
            ),
        ];
        for (method, path, body) in allowed {
            assert!(
                api.merge_authority_request_is_allowed(path, method, body),
                "expected gate request to be allowed: {method} {path}"
            );
        }

        let denied = [
            (
                "PATCH",
                "/repos/forge-admin/agent-forge-core/branches/main",
                body,
            ),
            (
                "DELETE",
                "/repos/forge-admin/agent-forge-core/branches/main",
                None,
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/issues/23/labels",
                body,
            ),
            (
                "PATCH",
                "/repos/forge-admin/agent-forge-core/pulls/23",
                body,
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge?force=true",
                body,
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                Some(
                    br#"{"Do":"rebase","head_commit_id":"0123456789abcdef0123456789abcdef01234567"}"#
                        .as_slice(),
                ),
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                Some(br#"{"Do":"merge"}"#.as_slice()),
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                Some(
                    br#"{"Do":"merge","head_commit_id":"not-a-full-object-id"}"#
                        .as_slice(),
                ),
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                Some(
                    br#"{"Do":"merge","head_commit_id":"0123456789abcdef0123456789abcdef01234567","unexpected":true}"#
                        .as_slice(),
                ),
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                Some(
                    br#"{"Do":"merge","Do":"rebase","head_commit_id":"0123456789abcdef0123456789abcdef01234567"}"#
                        .as_slice(),
                ),
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/23/merge",
                None,
            ),
            (
                "POST",
                "/repos/forge-admin/agent-forge-core/pulls/023/merge",
                body,
            ),
            (
                "GET",
                "/repos/forge-admin/agent-forge-core/pulls/23/reviews?limit=50&page=01",
                None,
            ),
            (
                "GET",
                "/repos/forge-admin/agent-forge-core/branch_protections",
                None,
            ),
            (
                "GET",
                "/repos/another-owner/agent-forge-core/pulls/23",
                None,
            ),
            ("GET", "/repos/forge-admin/agent-forge-core/pulls/23", body),
        ];
        for (method, path, body) in denied {
            assert!(
                !api.merge_authority_request_is_allowed(path, method, body),
                "unexpectedly allowed gate request: {method} {path}"
            );
        }
    }

    #[test]
    fn merge_authority_policy_rejects_other_writes_before_network_or_credential_read() {
        let (origin, server) = start_http_fixture(vec![(200, Duration::ZERO)]);
        let credential_reads = Arc::new(AtomicUsize::new(0));
        let mut api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin,
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(CountingHostCredentialProvider {
                token: "synthetic-token-0123456789".to_owned(),
                calls: Arc::clone(&credential_reads),
            }),
        );
        api.request_policy = LocalApiRequestPolicy::MergeAuthority;

        let forbidden = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/branches/main",
            "PATCH",
            Some(b"{}"),
            Duration::from_secs(1),
        );
        let forbidden_protection_patch = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/branch_protections/main",
            "PATCH",
            Some(b"{}"),
            Duration::from_secs(1),
        );
        let forbidden_protection_delete = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/branch_protections/main",
            "DELETE",
            None,
            Duration::from_secs(1),
        );
        let forbidden_merge_body = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/pulls/23/merge",
            "POST",
            Some(
                br#"{"Do":"rebase","head_commit_id":"0123456789abcdef0123456789abcdef01234567"}"#
                    .as_slice(),
            ),
            Duration::from_secs(1),
        );
        let allowed = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/pulls/23",
            "GET",
            None,
            Duration::from_secs(1),
        );
        let served = server.join().unwrap();

        assert!(matches!(forbidden, Err(ApiReadError::Forbidden)));
        assert!(matches!(
            forbidden_protection_patch,
            Err(ApiReadError::Forbidden)
        ));
        assert!(matches!(
            forbidden_protection_delete,
            Err(ApiReadError::Forbidden)
        ));
        assert!(matches!(forbidden_merge_body, Err(ApiReadError::Forbidden)));
        assert_eq!(allowed.map(|response| response.status), Ok(200));
        assert_eq!(credential_reads.load(Ordering::Relaxed), 1);
        assert_eq!(
            served, 1,
            "the forbidden mutation must be refused before network"
        );
    }

    #[test]
    fn local_adapter_does_not_follow_redirects_with_host_credential() {
        let (origin, server) = start_redirect_http_fixture();
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin: origin.clone(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(
                "synthetic-token-0123456789",
            )),
        );
        let url = format!(
            "{origin}/api/v1/repos/{}/{}/issues/11",
            LOCAL_REPOSITORY_OWNER, LOCAL_REPOSITORY_NAME
        );

        assert_eq!(
            api.read_current_once(&url, Duration::from_secs(1))
                .map_err(|failure| failure.error),
            Err(ApiReadError::HttpStatus(302))
        );
        let (request_count, request) = server.join().unwrap();
        assert_eq!(request_count, 1);
        assert!(request.contains("GET /api/v1/repos/forge-admin/agent-forge-core/issues/11 "));
        assert!(request.contains("Authorization: token synthetic-token-0123456789\r\n"));
    }

    struct WebhookReceiverProcess(Child);

    impl Drop for WebhookReceiverProcess {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    fn start_webhook_receiver_process(
        address: SocketAddr,
        secret_path: &std::path::Path,
        journal_path: &std::path::Path,
    ) -> WebhookReceiverProcess {
        let test_executable = env::current_exe().expect("read reconcile test executable path");
        let receiver_executable = test_executable
            .parent()
            .and_then(Path::parent)
            .expect("test executable is inside target/debug/deps")
            .join("forgejo-webhook-local");
        assert!(
            receiver_executable.is_file(),
            "build forgejo-webhook-local first, or use cargo test --all-targets"
        );
        WebhookReceiverProcess(
            Command::new(receiver_executable)
                .env_clear()
                .arg(address.to_string())
                .arg(secret_path)
                .arg(journal_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start the actual webhook receiver binary"),
        )
    }

    fn wait_for_webhook_receiver(
        receiver: &mut WebhookReceiverProcess,
        address: SocketAddr,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if receiver
                .0
                .try_wait()
                .expect("check receiver startup")
                .is_some()
            {
                return false;
            }
            if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                thread::sleep(Duration::from_millis(20));
                return receiver
                    .0
                    .try_wait()
                    .expect("check receiver after loopback connect")
                    .is_none();
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn start_ready_webhook_receiver_process(
        secret_path: &std::path::Path,
        journal_path: &std::path::Path,
    ) -> (WebhookReceiverProcess, SocketAddr) {
        for _ in 0..5 {
            let probe = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
            let address = probe.local_addr().expect("read reserved loopback address");
            drop(probe);

            let mut receiver = start_webhook_receiver_process(address, secret_path, journal_path);
            if wait_for_webhook_receiver(&mut receiver, address) {
                return (receiver, address);
            }
        }
        panic!("receiver could not acquire a loopback port after five bounded attempts");
    }

    fn send_signed_webhook(
        address: SocketAddr,
        secret: &[u8],
        delivery_id: &str,
        body: &[u8],
    ) -> (u16, Value) {
        let mut mac = FixtureHmacSha256::new_from_slice(secret).expect("valid fixture secret");
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());
        let headers = format!(
            "POST /forgejo/webhook HTTP/1.1\r\n\
             Host: 127.0.0.1\r\n\
             Content-Length: {}\r\n\
             Content-Type: application/json\r\n\
             X-Forgejo-Delivery: {delivery_id}\r\n\
             X-Forgejo-Event: issues\r\n\
             X-Forgejo-Signature: {signature}\r\n\r\n",
            body.len()
        );
        let mut request = headers.into_bytes();
        request.extend_from_slice(body);

        let mut stream = TcpStream::connect(address).expect("connect to receiver subprocess");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set bounded receiver response timeout");
        stream
            .write_all(&request)
            .expect("send signed webhook request");
        stream
            .shutdown(Shutdown::Write)
            .expect("half-close webhook request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("read receiver response");
        let response = String::from_utf8(response).expect("receiver response is UTF-8");
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .expect("receiver response has header terminator");
        let status = headers
            .split_whitespace()
            .nth(1)
            .expect("receiver response has a status")
            .parse()
            .expect("receiver status is numeric");
        (
            status,
            serde_json::from_str(body).expect("receiver response body is JSON"),
        )
    }

    fn stop_webhook_receiver(receiver: &mut WebhookReceiverProcess) -> ExitStatus {
        let result = unsafe { libc::kill(receiver.0.id() as libc::pid_t, libc::SIGTERM) };
        assert_eq!(result, 0, "send graceful shutdown to receiver");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = receiver.0.try_wait().expect("wait for receiver shutdown") {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = receiver.0.kill();
                let _ = receiver.0.wait();
                panic!("receiver did not shut down within five seconds");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn receiver_process_journal_reconciles_through_local_http_adapter() {
        let scratch = ScratchJournalDir::new();
        let secret = b"synthetic-receiver-adapter-secret";
        let secret_path = scratch.0.join("webhook.secret");
        let journal_path = scratch.0.join("inbox.jsonl");
        let mut secret_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&secret_path)
            .expect("create owner-only synthetic webhook secret");
        secret_file
            .write_all(secret)
            .expect("write synthetic webhook secret");
        secret_file
            .sync_all()
            .expect("flush synthetic webhook secret");

        let (mut receiver, address) =
            start_ready_webhook_receiver_process(&secret_path, &journal_path);
        let stale_body = serde_json::to_vec(&json!({
            "action": "closed",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "issue": {"number": 41, "state": "closed"}
        }))
        .expect("serialize synthetic stale webhook state");

        let (status, first) =
            send_signed_webhook(address, secret, "process-adapter-41-a", &stale_body);
        assert_eq!(status, 202);
        assert_eq!(first["accepted"], true);
        assert_eq!(first["duplicate"], false);
        assert_eq!(first["sequence"], 1);
        let (status, second) =
            send_signed_webhook(address, secret, "process-adapter-41-b", &stale_body);
        assert_eq!(status, 202);
        assert_eq!(second["accepted"], true);
        assert_eq!(second["duplicate"], false);
        assert_eq!(second["sequence"], 2);
        let (status, duplicate) =
            send_signed_webhook(address, secret, "process-adapter-41-a", &stale_body);
        assert_eq!(status, 202);
        assert_eq!(duplicate["duplicate"], true);
        let conflicting_body = serde_json::to_vec(&json!({
            "action": "edited",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "issue": {"number": 41, "state": "closed"}
        }))
        .expect("serialize conflicting synthetic webhook body");
        let (status, conflict) =
            send_signed_webhook(address, secret, "process-adapter-41-a", &conflicting_body);
        assert_eq!(status, 409);
        assert_eq!(conflict["error"]["code"], "delivery_conflict");
        assert_eq!(conflict["error"]["retryable"], false);
        assert!(stop_webhook_receiver(&mut receiver).success());

        let first_inbox = EventJournal::open(&journal_path)
            .replay()
            .expect("replay receiver process journal after shutdown");
        assert_eq!(
            first_inbox.len(),
            2,
            "same-delivery retry and conflicting body are not appended"
        );
        assert_eq!(
            first_inbox[0].payload["payload"]["issue"]["state"],
            "closed"
        );

        let (mut recovered_receiver, recovered_address) =
            start_ready_webhook_receiver_process(&secret_path, &journal_path);
        let (status, after_restart) = send_signed_webhook(
            recovered_address,
            secret,
            "process-adapter-41-a",
            &stale_body,
        );
        assert_eq!(status, 202);
        assert_eq!(after_restart["duplicate"], true);
        assert!(stop_webhook_receiver(&mut recovered_receiver).success());

        let inbox = EventJournal::open(&journal_path)
            .replay()
            .expect("replay receiver journal after restart");
        assert_eq!(inbox.len(), 2, "restart does not append a duplicate");

        let (origin, server) = start_current_issue_http_fixture_for_requests(4);
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1 {
                origin: origin.clone(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(
                "synthetic-token-0123456789",
            )),
        );

        let reconciled = reconcile_inbox_events(&inbox, &api).unwrap();
        assert_eq!(reconciled.len(), 2);
        for (result, delivery_id) in reconciled
            .into_iter()
            .zip(["process-adapter-41-a", "process-adapter-41-b"])
        {
            let state = result.expect("current-state HTTP fixture returns a valid issue");
            assert_eq!(state.delivery_id, delivery_id);
            assert_eq!(state.resource.id, ResourceId::Number(41));
            assert_eq!(
                state.state, "open",
                "current HTTP fixture state supersedes stale webhook"
            );
            assert_eq!(state.updated_at, "2026-10-22T16:00:00Z");
            assert_eq!(
                state.html_url,
                format!("{origin}/forge-admin/agent-forge-core/issues/41")
            );
        }

        let scope = BaseBranchScope::new(
            &origin,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            "main",
        )
        .unwrap();
        let binding = MissionAdmissionBindingV2 {
            schema_version: 2,
            issue_number: 41,
            requirement_id: "REQ-001".to_owned(),
            requirement_revision: 1,
            allocation_id: "receiver-process-admission-41".to_owned(),
        };
        let mission_journal_path = scratch.0.join("mission-execution.jsonl");
        let mission_journal = MissionExecutionJournalV1::open(&mission_journal_path);
        let inbox_journal = EventJournal::open(&journal_path);
        let first_event = read_journaled_webhook_event(&inbox_journal, "process-adapter-41-a")
            .expect("read the HMAC-verified event from the recovered receiver journal");
        let replay_event = read_journaled_webhook_event(&inbox_journal, "process-adapter-41-b")
            .expect("read the second HMAC-verified event from the journal");
        let admission_source =
            FixtureMissionAdmissionSource::new(origin.clone(), MissionIssueStateV1::Closed);
        let expected_requirement_sha256 = sha256(ADMISSION_REQUIREMENTS.as_bytes());

        let stale_admission = admit_reconciled_issue_v2(
            &first_event,
            &api,
            &scope,
            &binding,
            &expected_requirement_sha256,
            &admission_source,
            admission_context(),
            "workspace-webhook-41",
            admission_capabilities(),
            &mission_journal,
        );
        assert!(matches!(
            stale_admission,
            Err(WebhookMissionAdmissionErrorV1::Admission(error))
                if error.code == "mission_issue_closed"
        ));
        assert!(
            EventJournal::open(&mission_journal_path)
                .replay()
                .unwrap()
                .is_empty(),
            "a close after current-state reconciliation cannot allocate or claim"
        );

        admission_source.issue_state.set(MissionIssueStateV1::Open);
        let admitted = admit_reconciled_issue_v2(
            &first_event,
            &api,
            &scope,
            &binding,
            &expected_requirement_sha256,
            &admission_source,
            admission_context(),
            "workspace-webhook-41",
            admission_capabilities(),
            &mission_journal,
        )
        .expect("open issue and host-owned requirement binding admit one execution");
        assert!(matches!(
            admitted.admission.claim,
            ExecutionClaimResultV1::Claimed(_)
        ));
        assert_eq!(admitted.reconciled.state, "open");
        assert_eq!(admitted.admission.base_commit, ADMISSION_BASE_COMMIT);
        assert_eq!(
            admitted.admission.requirement_path,
            "requirements/REQ-001.json"
        );
        let mission_event_count = EventJournal::open(&mission_journal_path)
            .replay()
            .unwrap()
            .len();
        assert!(mission_event_count > 0);

        let replayed_admission = admit_reconciled_issue_v2(
            &replay_event,
            &api,
            &scope,
            &binding,
            &expected_requirement_sha256,
            &admission_source,
            admission_context(),
            "workspace-webhook-41",
            admission_capabilities(),
            &mission_journal,
        )
        .expect("new delivery ID reuses the same host allocation and execution claim");
        assert_eq!(
            admitted.admission.request.mission_id,
            replayed_admission.admission.request.mission_id
        );
        assert_eq!(
            admitted.admission.allocation.execution_generation,
            replayed_admission.admission.allocation.execution_generation
        );
        assert!(replayed_admission.admission.allocation.replayed);
        assert!(matches!(
            replayed_admission.admission.claim,
            ExecutionClaimResultV1::AlreadyClaimed(_)
        ));
        assert_eq!(
            EventJournal::open(&mission_journal_path)
                .replay()
                .unwrap()
                .len(),
            mission_event_count,
            "new delivery ID does not append a second mission claim"
        );
        assert_eq!(admission_source.issue_reads.get(), 5);
        assert_eq!(admission_source.requirement_reads.get(), 2);

        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 4);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 4);
        for request in requests {
            assert!(
                request.contains(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n"
                )
            );
            assert!(request.contains("Authorization: token synthetic-token-0123456789\r\n"));
        }
    }

    #[test]
    fn signed_journal_deliveries_reconcile_through_the_local_http_adapter() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-reconcile-webhook-secret";
        let stale_body = serde_json::to_vec(&json!({
            "action": "closed",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "issue": {"number": 41, "state": "closed"}
        }))
        .unwrap();
        let signing_key = SigningKey::from_bytes(&[0x31; 32]);
        for delivery_id in ["reconcile-http-41-a", "reconcile-http-41-b"] {
            let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
            mac.update(&stale_body);
            let signature = hex::encode(mac.finalize().into_bytes());
            assert!(matches!(
                ingest_forgejo_delivery_attested(
                    &journal,
                    secret,
                    &signing_key,
                    delivery_id,
                    "issues",
                    &signature,
                    &stale_body
                )
                .unwrap(),
                AppendResult::Appended(_)
            ));
        }
        let source_events = journal.replay().unwrap();
        let (origin, server) = start_current_issue_http_fixture();
        let credential = b"synthetic-token-0123456789";
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let mut writer = unsafe { File::from_raw_fd(descriptors[1]) };
        writer.write_all(credential).unwrap();
        drop(writer);
        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: origin.clone(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: vec![
                "reconcile-http-41-a".to_owned(),
                "reconcile-http-41-b".to_owned(),
            ],
            credential_fd: Some(descriptors[0]),
            trusted_attestation_keys: vec![signing_key.verifying_key()],
        })
        .unwrap();

        assert!(exit_success);
        assert_eq!(result["verdict"], "current_state_read");
        assert_eq!(result["upstream_read_attempts"], 1);
        let reconciled = result["results"].as_array().unwrap();
        assert_eq!(reconciled.len(), 2);
        for ((state, delivery_id), source_event) in reconciled
            .iter()
            .zip(["reconcile-http-41-a", "reconcile-http-41-b"])
            .zip(&source_events)
        {
            assert_eq!(state["verdict"], "current_state_read");
            assert_eq!(state["delivery_id"], delivery_id);
            assert_eq!(state["resource"]["number"], 41);
            assert_eq!(state["canonical_state"]["state"], "open");
            assert_eq!(
                state["canonical_state"]["updated_at"],
                "2026-10-22T16:00:00Z"
            );
            assert_eq!(
                state["canonical_state"]["html_url"],
                format!("{origin}/forge-admin/agent-forge-core/issues/41")
            );
            assert_eq!(state["journal_source"]["sequence"], source_event.sequence);
            assert_eq!(state["journal_source"]["event_sha256"], source_event.hash);
            assert_eq!(
                state["journal_source"]["receiver_attestation"]["status"],
                "verified"
            );
            assert_eq!(
                state["journal_source"]["receiver_attestation"]["algorithm"],
                "ed25519"
            );
            assert_eq!(
                state["journal_source"]["receiver_attestation"]["key_id_sha256"],
                source_event.receiver_attestation.as_ref().unwrap().key_id
            );
            assert!(
                state["journal_source"]["receiver_attestation"]
                    .get("signature_hex")
                    .is_none()
            );
        }
        let request = server.join().unwrap();
        assert!(
            request
                .contains("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
        );
        assert!(request.contains("Authorization: token synthetic-token-0123456789\r\n"));
    }

    #[test]
    fn batch_host_attestation_refusal_binds_all_selected_source_rows() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-attestation-refusal-source-secret";
        let signing_key = SigningKey::from_bytes(&[0x33; 32]);
        let delivery_ids = vec![
            "batch-policy-missing-attested".to_owned(),
            "batch-policy-missing-legacy".to_owned(),
        ];
        for (index, delivery_id) in delivery_ids.iter().enumerate() {
            let body = serde_json::to_vec(&json!({
                "action": "opened",
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "issue": {"number": 41 + index as u64, "state": "open"}
            }))
            .unwrap();
            let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
            mac.update(&body);
            let signature = hex::encode(mac.finalize().into_bytes());
            let appended = if index == 0 {
                ingest_forgejo_delivery_attested(
                    &journal,
                    secret,
                    &signing_key,
                    delivery_id,
                    "issues",
                    &signature,
                    &body,
                )
                .unwrap()
            } else {
                ingest_forgejo_delivery(&journal, secret, delivery_id, "issues", &signature, &body)
                    .unwrap()
            };
            assert!(matches!(appended, AppendResult::Appended(_)));
        }
        let source_events = journal.replay().unwrap();

        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: delivery_ids.clone(),
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: Vec::new(),
        })
        .unwrap();

        assert!(!exit_success);
        assert_eq!(result["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(
            result["error"]["code"],
            "host_journal_attestation_policy_missing"
        );
        assert_eq!(result["error"]["details"]["host_credential_read"], false);
        assert_eq!(
            result["error"]["details"]["upstream_calls_attempted"],
            false
        );
        assert!(result.get("results").is_none());

        let selected_sources = result["selected_event_sources"]
            .as_array()
            .expect("attestation-policy refusal retains all selected source identities");
        assert_eq!(selected_sources.len(), delivery_ids.len());
        for (((source, delivery_id), source_event), expected_status) in selected_sources
            .iter()
            .zip(&delivery_ids)
            .zip(&source_events)
            .zip(["present_unverified", "missing"])
        {
            assert_eq!(source["delivery_id"].as_str(), Some(delivery_id.as_str()));
            assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
            assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
            assert_eq!(
                source["journal_source"]["receiver_attestation"]["status"],
                expected_status
            );
            assert_eq!(
                source["journal_source"]["receiver_attestation"]["verification_scope"],
                "not_verified"
            );
            assert_eq!(
                source["journal_source"]["receiver_attestation"]["signature_included"],
                false
            );
        }
    }

    #[test]
    fn batch_credential_pipe_read_failure_is_structured_and_source_bound() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-credential-read-failure-secret";
        let signing_key = SigningKey::from_bytes(&[0x34; 32]);
        let delivery_ids = vec![
            "credential-read-failure-a".to_owned(),
            "credential-read-failure-b".to_owned(),
        ];
        for (index, delivery_id) in delivery_ids.iter().enumerate() {
            let body = serde_json::to_vec(&json!({
                "action": "opened",
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "issue": {"number": 41 + index as u64, "state": "open"}
            }))
            .unwrap();
            let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
            mac.update(&body);
            let signature = hex::encode(mac.finalize().into_bytes());
            assert!(matches!(
                ingest_forgejo_delivery_attested(
                    &journal,
                    secret,
                    &signing_key,
                    delivery_id,
                    "issues",
                    &signature,
                    &body,
                )
                .unwrap(),
                AppendResult::Appended(_)
            ));
        }
        let source_events = journal.replay().unwrap();

        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: delivery_ids.clone(),
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: vec![signing_key.verifying_key()],
        })
        .unwrap();

        assert!(!exit_success);
        assert_eq!(result["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 0);
        assert_eq!(result["error"]["code"], "host_credential_pipe_read_failed");
        assert_eq!(result["error"]["retryable"], false);
        assert_eq!(
            result["error"]["details"]["credential_pipe_read_attempted"],
            true
        );
        assert_eq!(
            result["error"]["details"]["credential_pipe_read_succeeded"],
            false
        );
        assert_eq!(
            result["error"]["details"]["upstream_calls_attempted"],
            false
        );
        assert_eq!(result["error"]["details"]["canonical_state_created"], false);
        assert_eq!(
            result["error"]["details"]["reader_error"],
            "cannot secure the host credential descriptor"
        );
        assert!(result.get("results").is_none());
        assert!(result.get("canonical_state").is_none());

        let selected_sources = result["selected_event_sources"]
            .as_array()
            .expect("credential-pipe refusal retains all selected source identities");
        assert_eq!(selected_sources.len(), delivery_ids.len());
        for ((source, delivery_id), source_event) in selected_sources
            .iter()
            .zip(&delivery_ids)
            .zip(&source_events)
        {
            assert_eq!(source["delivery_id"].as_str(), Some(delivery_id.as_str()));
            assert_eq!(source["journal_source"]["sequence"], source_event.sequence);
            assert_eq!(source["journal_source"]["event_sha256"], source_event.hash);
            assert_eq!(
                source["journal_source"]["receiver_attestation"]["status"],
                "verified"
            );
            assert_eq!(
                source["journal_source"]["receiver_attestation"]["signature_included"],
                false
            );
        }
    }

    #[test]
    fn batch_cache_budget_is_enforced_across_signed_journal_and_loopback_http() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-batch-cache-webhook-secret";
        let signing_key = SigningKey::from_bytes(&[0x32; 32]);
        let mut delivery_ids = (11..29)
            .map(|number| format!("batch-cache-issue-{number}"))
            .collect::<Vec<_>>();
        delivery_ids.push("batch-cache-issue-11-replay".to_owned());
        for (index, delivery_id) in delivery_ids.iter().enumerate() {
            let number = if index + 1 == delivery_ids.len() {
                11
            } else {
                11 + index as u64
            };
            let body = serde_json::to_vec(&json!({
                "action": "opened",
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "issue": {"number": number, "state": "open"}
            }))
            .unwrap();
            let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
            mac.update(&body);
            let signature = hex::encode(mac.finalize().into_bytes());
            let appended = if delivery_id == "batch-cache-issue-11" {
                ingest_forgejo_delivery_attested(
                    &journal,
                    secret,
                    &signing_key,
                    delivery_id,
                    "issues",
                    &signature,
                    &body,
                )
                .unwrap()
            } else {
                ingest_forgejo_delivery(&journal, secret, delivery_id, "issues", &signature, &body)
                    .unwrap()
            };
            assert!(matches!(appended, AppendResult::Appended(_)));
        }

        let (origin, server) = start_batch_cache_http_fixture(16);
        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: origin.clone(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids,
            credential_fd: None,
            trusted_attestation_keys: Vec::new(),
        })
        .unwrap();

        assert!(!exit_success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 16);
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 19);
        assert!(
            results[..15]
                .iter()
                .all(|item| item["verdict"] == "current_state_read")
        );
        assert_eq!(
            results[0]["journal_source"]["receiver_attestation"]["status"],
            "present_unverified"
        );
        assert_eq!(
            results[0]["journal_source"]["receiver_attestation"]["verification_scope"],
            "not_verified"
        );
        assert!(results[0]["journal_source"]["receiver_attestation"]["key_id_sha256"].is_null());
        assert_eq!(
            results[1]["journal_source"]["receiver_attestation"]["status"],
            "missing"
        );
        assert!(results[15..18].iter().all(|item| {
            item["verdict"] == "inconclusive"
                && item["error"]["code"] == "batch_cache_budget_exceeded"
                && item["error"]["retryable"] == false
        }));
        assert_eq!(results[18]["verdict"], "current_state_read");
        assert_eq!(results[18]["resource"]["number"], 11);

        let responses = server.join().unwrap();
        let response_lengths = responses
            .iter()
            .map(|(_, body_bytes)| *body_bytes)
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 16);
        assert!(
            response_lengths
                .iter()
                .all(|length| *length <= MAX_RESPONSE_BYTES)
        );
        assert!(
            response_lengths[..15].iter().sum::<usize>() <= MAX_RECONCILE_BATCH_CACHED_JSON_BYTES
        );
        assert!(response_lengths.iter().sum::<usize>() > MAX_RECONCILE_BATCH_CACHED_JSON_BYTES);
        for (request, number) in responses.iter().zip(11..=26) {
            assert_eq!(
                request.0,
                format!("GET /api/v1/repos/forge-admin/agent-forge-core/issues/{number} HTTP/1.1")
            );
        }
    }

    #[test]
    fn local_adapter_retries_non_utf8_transient_http_body() {
        let (origin, server) = start_http_fixture_with_bodies(vec![
            (503, Duration::ZERO, vec![0xff, 0xfe]),
            (200, Duration::ZERO, br#"{"number":11}"#.to_vec()),
        ]);
        let api = api_for_origin(origin);

        let current = api.read_current(&issue_resource());

        assert_eq!(current.unwrap(), json!({"number": 11}));
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 2);
        assert_eq!(server.join().unwrap(), 2);
    }

    #[test]
    fn local_adapter_retries_rate_limit_response_within_the_read_budget() {
        let (origin, server) =
            start_http_fixture(vec![(429, Duration::ZERO), (200, Duration::ZERO)]);
        let api = api_for_origin(origin);

        let current = api.read_current(&issue_resource());

        assert_eq!(current.unwrap(), json!({"number": 11}));
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 2);
        assert_eq!(server.join().unwrap(), 2);
    }

    #[test]
    fn batch_cache_keeps_issue_and_pull_request_with_same_number_separate_over_http() {
        let (origin, server) = start_issue_and_pull_same_number_fixture();
        let api = api_for_origin(origin.clone());
        let event = |event_name: &str, resource_name: &str, delivery_id: &str| {
            let payload = if event_name == "issues" {
                json!({
                    "action": "opened",
                    "repository": {"full_name": "forge-admin/agent-forge-core"},
                    (resource_name): {"number": 19, "state": "open"}
                })
            } else {
                json!({
                    "action": "opened",
                    "repository": {"full_name": "forge-admin/agent-forge-core"},
                    (resource_name): {"number": 19, "state": "open"}
                })
            };
            JournalEvent {
                sequence: 1,
                event_id: format!("event-{delivery_id}"),
                idempotency_key: format!("forgejo-delivery-v1:{delivery_id}"),
                kind: format!("forgejo.webhook.v1.{event_name}"),
                payload: json!({
                    "schema_version": 1,
                    "provider": "forgejo",
                    "delivery_id": delivery_id,
                    "event": event_name,
                    "payload": payload
                }),
                previous_hash: "0".repeat(64),
                receiver_attestation: None,
                hash: "1".repeat(64),
            }
        };

        let events = [
            event("issues", "issue", "same-number-issue"),
            event("pull_request", "pull_request", "same-number-pull"),
        ];
        let results = reconcile_inbox_events(&events, &api).unwrap();

        let issue = results[0].as_ref().unwrap();
        assert_eq!(issue.resource.kind, ResourceKind::Issue);
        assert_eq!(issue.state, "closed");
        assert_eq!(
            issue.html_url,
            format!("{origin}/forge-admin/agent-forge-core/issues/19")
        );
        assert_eq!(issue.head_sha, None);
        assert_eq!(issue.base_sha, None);
        assert_eq!(
            reconciled_state_value(issue)["canonical_state"]["work_item_content"],
            json!({
                "title": "current issue title",
                "body": "current issue body",
                "source": "current_forgejo_issue_api",
                "trust": "untrusted_user_content",
            })
        );

        let pull_request = results[1].as_ref().unwrap();
        assert_eq!(pull_request.resource.kind, ResourceKind::PullRequest);
        assert_eq!(pull_request.state, "open");
        assert_eq!(pull_request.merged, Some(false));
        assert_eq!(pull_request.draft, Some(false));
        assert_eq!(
            pull_request.html_url,
            format!("{origin}/forge-admin/agent-forge-core/pulls/19")
        );
        assert_eq!(
            pull_request.head_sha.as_deref(),
            Some("e7a0672a04f51c3f430a57780a2de5fb2fdf0958")
        );
        assert_eq!(
            pull_request.base_sha.as_deref(),
            Some("f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0")
        );
        let canonical_output = reconciled_state_value(pull_request);
        assert_eq!(
            canonical_output["canonical_state"]["work_item_content"],
            json!({
                "title": "current pull request title",
                "body": "current pull request body",
                "source": "current_forgejo_pull_request_api",
                "trust": "untrusted_user_content",
            })
        );
        assert_eq!(
            canonical_output["canonical_state"]["requested_reviewers"],
            json!([
                {"id": 3, "login": "reviewer-a"},
                {"id": 7, "login": "reviewer-z"}
            ]),
            "reviewer readback is canonical, stable and excludes private account fields"
        );
        assert_eq!(
            canonical_output["canonical_state"]["requested_reviewers_teams"],
            json!([
                {"id": 4, "name": "team-a"},
                {"id": 29, "name": "team-z"}
            ]),
            "team reviewer readback is canonical, stable and excludes organization details"
        );

        assert_eq!(
            server.join().unwrap(),
            vec![
                "GET /api/v1/repos/forge-admin/agent-forge-core/issues/19 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/19 HTTP/1.1",
            ]
        );
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn local_adapter_respects_retry_after_when_server_delay_exceeds_remaining_budget() {
        let (origin, server, _) = start_http_fixture_with_extra_headers_recording_requests(vec![
            (
                429,
                Duration::ZERO,
                br#"{"message":"rate limited"}"#.to_vec(),
                vec!["Retry-After: 30".to_owned()],
            ),
            (
                200,
                Duration::ZERO,
                br#"{"number":11}"#.to_vec(),
                Vec::new(),
            ),
        ]);
        let api = api_for_origin_with_policy(
            origin,
            LocalApiRetryPolicy {
                max_attempts: 3,
                request_timeout: Duration::from_secs(1),
                total_budget: Duration::from_millis(500),
                backoff: [Duration::ZERO, Duration::ZERO],
            },
        );

        let current = api.read_current(&issue_resource());

        assert_eq!(
            current,
            Err(ApiReadError::HttpStatusWithRetryAfter {
                status: 429,
                retry_after_ms: 30_000,
            })
        );
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 1);
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn retry_after_parser_accepts_delta_seconds_and_http_date_formats() {
        assert_eq!(
            parse_retry_after(b"30"),
            Some(RetryAfterValue::Delay(Duration::from_secs(30)))
        );
        assert_eq!(
            parse_http_date_epoch_seconds("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(
            parse_http_date_epoch_seconds("Sunday, 06-Nov-94 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(
            parse_http_date_epoch_seconds("Sun Nov  6 08:49:37 1994"),
            Some(784_111_777)
        );
        assert_eq!(
            parse_retry_after(b"Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(RetryAfterValue::Delay(Duration::ZERO))
        );
        assert_eq!(parse_retry_after(b""), None);
        assert_eq!(
            parse_retry_after(b"not-a-date"),
            Some(RetryAfterValue::Invalid)
        );
    }

    #[test]
    fn local_adapter_stops_after_three_transient_http_attempts() {
        let (origin, server) = start_http_fixture(vec![
            (503, Duration::ZERO),
            (503, Duration::ZERO),
            (503, Duration::ZERO),
            (200, Duration::ZERO),
        ]);
        let api = api_for_origin(origin);

        let current = api.read_current(&issue_resource());

        assert_eq!(current, Err(ApiReadError::HttpStatus(503)));
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 3);
        assert_eq!(server.join().unwrap(), 3);
    }

    #[test]
    fn local_adapter_does_not_retry_unauthorized_http_responses() {
        let (origin, server) =
            start_http_fixture(vec![(401, Duration::ZERO), (200, Duration::ZERO)]);
        let api = api_for_origin(origin);

        let current = api.read_current(&issue_resource());

        assert_eq!(current, Err(ApiReadError::Unauthorized));
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 1);
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn local_adapter_does_not_retry_oversized_http_response() {
        let (origin, server) = start_http_fixture_with_bodies(vec![
            (503, Duration::ZERO, vec![b'x'; MAX_RESPONSE_BYTES + 1]),
            (200, Duration::ZERO, br#"{"number":11}"#.to_vec()),
        ]);
        let api = api_for_origin(origin);

        let current = api.read_current(&issue_resource());

        assert_eq!(
            current,
            Err(ApiReadError::ResponseTooLarge {
                component: "body",
                maximum_bytes: MAX_RESPONSE_BYTES,
            })
        );
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 1);
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn local_adapter_rejects_success_response_with_duplicate_json_member() {
        let (origin, server) = start_http_fixture_with_bodies(vec![(
            200,
            Duration::ZERO,
            br#"{"number":11,"number":12}"#.to_vec(),
        )]);
        let api = api_for_origin(origin);

        let current = api.read_current(&issue_resource());

        assert_eq!(current, Err(ApiReadError::InvalidResponse));
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 1);
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn local_adapter_rejects_duplicate_total_count_headers() {
        let (origin, server, _) = start_http_fixture_with_extra_headers_recording_requests(vec![(
            200,
            Duration::ZERO,
            b"[]".to_vec(),
            vec!["X-Total-Count: 1".to_owned(), "X-Total-Count: 2".to_owned()],
        )]);
        let api = api_for_origin(origin);

        let response = api.request_api_once(
            "/repos/forge-admin/agent-forge-core/pulls/23/reviews?limit=50&page=1",
            "GET",
            None,
            Duration::from_secs(2),
        );

        assert_eq!(
            response.map(|response| response.total_count),
            Err(ApiReadError::InvalidResponse)
        );
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn local_adapter_shares_one_deadline_across_reads() {
        let (origin, server) = start_http_fixture(vec![(200, Duration::from_millis(200))]);
        let api = api_for_origin_with_policy(
            origin,
            LocalApiRetryPolicy {
                max_attempts: 3,
                request_timeout: Duration::from_secs(1),
                total_budget: Duration::from_millis(60),
                backoff: [Duration::ZERO, Duration::ZERO],
            },
        );

        let started = Instant::now();
        let first = api.read_current(&issue_resource());
        let second = api.read_current(&ResourceRef {
            id: ResourceId::Number(12),
            ..issue_resource()
        });
        let elapsed = started.elapsed();

        assert_eq!(first, Err(ApiReadError::Unavailable));
        assert_eq!(second, Err(ApiReadError::Unavailable));
        assert!(elapsed < Duration::from_millis(150), "elapsed: {elapsed:?}");
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 1);
        assert_eq!(server.join().unwrap(), 1);
    }

    #[test]
    fn response_reader_stops_after_one_byte_over_the_limit() {
        let mut reader = CountingReader {
            remaining: 1_000_000,
            bytes_read: 0,
        };

        assert_eq!(
            read_limited(&mut reader, 8),
            Err(ApiReadError::InvalidResponse)
        );
        assert_eq!(reader.bytes_read, 9);
    }

    #[test]
    fn response_reader_accepts_the_exact_limit() {
        let mut reader = CountingReader {
            remaining: 8,
            bytes_read: 0,
        };

        assert_eq!(read_limited(&mut reader, 8).unwrap(), vec![b'x'; 8]);
        assert_eq!(reader.bytes_read, 8);
    }

    #[test]
    fn command_reader_stops_an_unbounded_child_at_the_cap() {
        let result = run_command_bounded(
            Command::new("/usr/bin/yes"),
            8,
            Instant::now() + Duration::from_secs(1),
        );

        assert_eq!(
            result,
            Err(ApiReadError::ResponseTooLarge {
                component: "captured_output",
                maximum_bytes: 8,
            })
        );
    }

    #[test]
    fn curl_timeout_rounds_down_to_whole_milliseconds() {
        assert_eq!(
            curl_timeout_argument(Duration::from_micros(1_999)).as_deref(),
            Some("0.001")
        );
        assert_eq!(curl_timeout_argument(Duration::from_micros(999)), None);
    }

    #[test]
    fn bounded_command_is_cancelled_before_the_child_exits_naturally() {
        for _ in 0..3 {
            let started = Instant::now();
            let mut command = Command::new("/usr/bin/sleep");
            command.arg("2");
            let result = run_command_bounded(command, 8, started + Duration::from_millis(50));

            assert_eq!(result, Err(ApiReadError::Unavailable));
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "deadline cancellation should finish before the 2-second child exits"
            );
        }
    }

    #[test]
    fn bounded_command_cleans_up_child_when_reader_thread_cannot_start() {
        let started = Instant::now();
        let mut command = Command::new("/usr/bin/sleep");
        command.arg("2");
        let child_pid = std::sync::Mutex::new(None);
        let result = run_command_bounded_with_spawner(
            command,
            8,
            started + Duration::from_secs(1),
            |reader_task, pid| {
                drop(reader_task);
                *child_pid.lock().unwrap() = Some(pid);
                Err(io::Error::other("injected reader thread creation failure"))
            },
        );

        assert_eq!(result, Err(ApiReadError::Unavailable));
        assert!(started.elapsed() < Duration::from_millis(300));
        let child_pid = child_pid.into_inner().unwrap().unwrap() as libc::pid_t;
        let mut status = 0;
        let child_state = unsafe { libc::waitpid(child_pid, &mut status, libc::WNOHANG) };
        let wait_error = io::Error::last_os_error();
        if child_state == 0 {
            unsafe {
                libc::kill(child_pid, libc::SIGKILL);
                libc::waitpid(child_pid, &mut status, 0);
            }
        }
        assert_eq!(child_state, -1, "child remains alive or unreaped");
        assert_eq!(wait_error.raw_os_error(), Some(libc::ECHILD));
    }

    #[test]
    fn invalid_host_credential_is_actionable_nonretryable_and_value_free() {
        let value =
            reconciliation_error_value(&ReconcileError::Source(ApiReadError::InvalidCredential));
        let error_text = value.to_string();

        assert_eq!(value["code"], "credential_source_invalid");
        assert_eq!(value["retryable"], false);
        assert!(
            value["action"]
                .as_str()
                .is_some_and(|action| !action.is_empty())
        );
        assert!(!error_text.contains("synthetic-token-0123456789"));
    }

    #[test]
    fn invalid_event_payload_field_reports_persisted_journal_recovery_action() {
        let value = reconciliation_error_value(&ReconcileError::InvalidEventPayloadField("sha"));

        assert_eq!(value["code"], "invalid_envelope");
        assert_eq!(value["retryable"], false);
        assert_eq!(value["details"]["field_path"], "payload.sha");
        assert_eq!(value["details"]["persistence_state"], "already_persisted");
        assert_eq!(value["details"]["source_hmac_reverified"], false);
        let action = value["action"].as_str().unwrap();
        assert!(action.contains("persisted row"), "{action}");
        assert!(action.contains("trusted sender evidence"), "{action}");
        assert!(!action.contains("not journaled"), "{action}");
    }

    #[test]
    fn trusted_quarantine_command_appends_once_and_reconcile_refuses_before_reads_or_credential() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let delivery_id = "operator-quarantine-review-17";
        write_legacy_webhook_fixture(
            &journal_path,
            delivery_id,
            "pull_request_approved",
            json!({
                "repository":{"full_name":"forge-admin/agent-forge-core"},
                "pull_request":{"number":17},
                "review":{"type":"pull_request_review_approved"}
            }),
        );
        let source = EventJournal::open_webhook_inbox(&journal_path)
            .replay()
            .unwrap()
            .remove(0);

        let args = vec![
            journal_path.as_os_str().to_owned(),
            delivery_id.into(),
            "1".into(),
            source.hash.clone().into(),
        ]
        .into_iter();
        let (quarantined, operation_ok) = run_quarantine_invalid_delivery(args).unwrap();
        assert!(operation_ok);
        assert_eq!(quarantined["verdict"], "delivery_quarantined");
        assert_eq!(quarantined["canonical_state_created"], false);
        assert_eq!(quarantined["upstream_read_attempts"], 0);
        assert_eq!(quarantined["source_sequence"], 1);
        assert_eq!(quarantined["source_event_hash"], source.hash);
        assert_eq!(quarantined["disposition_sequence"], 2);
        assert_eq!(quarantined["reason_field"], "action");

        let mut pipe_fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
        let credential_probe = b"quarantine-must-not-consume-this";
        assert_eq!(
            unsafe {
                libc::write(
                    pipe_fds[1],
                    credential_probe.as_ptr().cast(),
                    credential_probe.len(),
                )
            },
            credential_probe.len() as isize
        );
        unsafe { libc::close(pipe_fds[1]) };

        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1::default(),
            journal_path: journal_path.as_os_str().to_owned(),
            delivery_ids: vec![delivery_id.to_owned()],
            credential_fd: Some(pipe_fds[0]),
            trusted_attestation_keys: Vec::new(),
        })
        .unwrap();
        assert!(!exit_success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "delivery_quarantined");
        assert_eq!(result["error"]["retryable"], false);
        assert_eq!(result["error"]["details"]["canonical_state_created"], false);
        assert_eq!(result["upstream_read_attempts"], 0);

        let mut credential = unsafe { File::from_raw_fd(pipe_fds[0]) };
        let mut remaining = Vec::new();
        credential.read_to_end(&mut remaining).unwrap();
        assert_eq!(
            remaining, credential_probe,
            "quarantine does not read host credentials"
        );
    }

    #[test]
    fn mixed_reconciliation_batch_keeps_quarantine_inconclusive_and_reads_only_active_delivery() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let quarantined_delivery_id = "mixed-quarantine-review-17";
        write_legacy_webhook_fixture(
            &journal_path,
            quarantined_delivery_id,
            "pull_request_approved",
            json!({
                "repository":{"full_name":"forge-admin/agent-forge-core"},
                "pull_request":{"number":17},
                "review":{"type":"pull_request_review_approved"}
            }),
        );
        let quarantined_source = EventJournal::open_webhook_inbox(&journal_path)
            .replay()
            .unwrap()
            .remove(0);
        quarantine_invalid_webhook_delivery(
            &EventJournal::open_webhook_inbox(&journal_path),
            quarantined_delivery_id,
            quarantined_source.sequence,
            &quarantined_source.hash,
        )
        .unwrap();

        let active_delivery_id = "mixed-active-issue-41";
        let secret = b"synthetic-mixed-batch-webhook-secret";
        let active_body = serde_json::to_vec(&json!({
            "action":"opened",
            "repository":{"full_name":"forge-admin/agent-forge-core"},
            "issue":{"number":41}
        }))
        .unwrap();
        let signing_key = SigningKey::from_bytes(&[0x32; 32]);
        let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
        mac.update(&active_body);
        let signature = hex::encode(mac.finalize().into_bytes());
        ingest_forgejo_delivery_attested(
            &EventJournal::open_webhook_inbox(&journal_path),
            secret,
            &signing_key,
            active_delivery_id,
            "issues",
            &signature,
            &active_body,
        )
        .unwrap();

        let (origin, server) = start_current_issue_http_fixture();
        let token = b"synthetic-active-only-credential";
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let mut writer = unsafe { File::from_raw_fd(descriptors[1]) };
        writer.write_all(token).unwrap();
        drop(writer);
        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1::for_profile_at_test_origin(
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                origin.clone(),
            ),
            journal_path: journal_path.into_os_string(),
            delivery_ids: vec![
                quarantined_delivery_id.to_owned(),
                active_delivery_id.to_owned(),
            ],
            credential_fd: Some(descriptors[0]),
            trusted_attestation_keys: vec![signing_key.verifying_key()],
        })
        .unwrap();

        assert!(!exit_success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 1);
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["delivery_id"], quarantined_delivery_id);
        assert_eq!(results[0]["verdict"], "inconclusive");
        assert_eq!(results[0]["error"]["code"], "delivery_quarantined");
        assert_eq!(results[0].get("canonical_state"), None);
        assert_eq!(results[1]["delivery_id"], active_delivery_id);
        assert_eq!(results[1]["verdict"], "current_state_read");
        assert_eq!(results[1]["canonical_state"]["state"], "open");
        assert_eq!(
            results[1]["canonical_state"]["html_url"],
            format!("{origin}/forge-admin/agent-forge-core/issues/41")
        );
        assert!(
            !result
                .to_string()
                .contains("synthetic-active-only-credential")
        );
        let request = server.join().unwrap();
        assert!(
            request
                .contains("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
        );
        assert!(request.contains("Authorization: token synthetic-active-only-credential\r\n"));
    }

    #[test]
    fn unsupported_event_error_points_to_cli_capability_contract() {
        let value = reconciliation_error_value(&ReconcileError::UnsupportedEvent);

        assert_eq!(value["code"], "unsupported_event");
        assert_eq!(value["retryable"], false);
        assert_eq!(
            value["details"]["supported_events"],
            json!(SUPPORTED_EVENTS)
        );
        assert_eq!(
            value["details"]["event_contracts_capability"],
            "operations[reconcile_webhook_deliveries].event_contracts"
        );
        assert!(
            value["action"]
                .as_str()
                .is_some_and(|action| action.contains("--capabilities"))
        );

        let mut largest_error = value;
        largest_error["details"]["journal_source_sequence"] = Value::from(u64::MAX);
        largest_error["details"]["journal_source_hash"] = Value::String("f".repeat(64));
        let largest_error_result = json!({
            "verdict": "inconclusive",
            "delivery_id": "d".repeat(128),
            "error": largest_error,
        });
        assert!(
            serde_json::to_vec(&largest_error_result).unwrap().len() <= 2048,
            "CLI error envelope must fit the batch output reserve"
        );
    }

    #[test]
    fn unsupported_event_payload_field_names_the_unsupported_ref_type() {
        let value =
            reconciliation_error_value(&ReconcileError::UnsupportedEventPayloadField("ref_type"));

        assert_eq!(value["code"], "unsupported_event");
        assert_eq!(value["retryable"], false);
        assert_eq!(value["details"]["field_path"], "payload.ref_type");
        assert_eq!(
            value["details"]["event_contracts_capability"],
            "operations[reconcile_webhook_deliveries].event_contracts"
        );
        let action = value["action"].as_str().unwrap();
        assert!(action.contains("branch refs only"));
        assert!(action.contains("--capabilities"));
    }

    #[test]
    fn private_resource_not_found_is_inconclusive_and_explains_visibility_ambiguity() {
        let (value, exit_success) = single_reconciliation_result(
            "webhook-private-pr",
            Err(ReconcileError::Source(ApiReadError::NotFound)),
            1,
        );

        assert!(!exit_success);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["error"]["code"], "resource_not_found");
        assert_eq!(value["error"]["retryable"], false);
        assert!(value.get("canonical_state").is_none());
        let action = value["error"]["action"].as_str().unwrap();
        assert!(action.contains("private resource may be masked as HTTP 404"));
        assert!(action.contains("host-scoped Forgejo credential"));
        assert!(action.contains("outside agent input"));
    }

    #[test]
    fn single_reconciliation_failure_is_structured_and_inconclusive() {
        let (value, exit_success) = single_reconciliation_result(
            "webhook-017-delivery",
            Err(ReconcileError::Source(ApiReadError::HttpStatus(503))),
            3,
        );

        assert!(!exit_success);
        assert_eq!(value["schema_version"], RECONCILE_SINGLE_SCHEMA_VERSION);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["delivery_id"], "webhook-017-delivery");
        assert_eq!(value["error"]["code"], "upstream_http_status");
        assert_eq!(value["error"]["retryable"], true);
        assert_eq!(value["error"]["details"]["http_status"], 503);
        assert!(value["error"]["details"].get("retry_after_ms").is_none());
        assert_eq!(value["upstream_read_attempts"], 3);
        assert!(!value["error"]["action"].as_str().unwrap().is_empty());
    }

    #[test]
    fn successful_reconciliation_states_that_webhook_hmac_is_not_reverified() {
        let reconciled = ReconciledState {
            delivery_id: "source-provenance-issue".to_owned(),
            resource: ResourceRef {
                kind: ResourceKind::Issue,
                owner: "forge-admin".to_owned(),
                repository: "agent-forge-core".to_owned(),
                id: ResourceId::Number(41),
            },
            state: "open".to_owned(),
            updated_at: "2026-10-27T12:00:00Z".to_owned(),
            html_url: "http://127.0.0.1:3130/forge-admin/agent-forge-core/issues/41".to_owned(),
            head_sha: None,
            parent_workflow_run: None,
            base_sha: None,
            merged: None,
            draft: None,
            merge_commit_sha: None,
            requested_reviewers: None,
            requested_reviewers_teams: None,
            labels: None,
            assignees: None,
            milestone: None,
            work_item_content: None,
            comment: None,
            review_notification: None,
            release: None,
        };

        let (single, exit_success) =
            single_reconciliation_result("source-provenance-issue", Ok(reconciled), 1);
        assert!(exit_success);
        assert_eq!(single["verdict"], "current_state_read");
        assert_eq!(
            single["source_event_provenance_contract"]["replay_hmac_verification"],
            "not_performed"
        );
        assert_eq!(
            single["source_event_provenance_contract"]["raw_signature_persisted"],
            false
        );
        assert_eq!(
            single["source_event_provenance_contract"]["journal_hash_chain_authenticates_local_writer"],
            false
        );
        assert_eq!(
            single["source_event_provenance_contract"]["current_state_read_means"],
            "canonical_api_read_only"
        );

        let (batch, _) = batch_reconciliation_failure(&ReconcileError::BatchTooLarge, 101, 0);
        assert_eq!(
            batch["source_event_provenance_contract"],
            single["source_event_provenance_contract"]
        );
    }

    #[test]
    fn retry_after_http_error_exposes_only_a_bounded_structured_delay() {
        let (value, exit_success) = single_reconciliation_result(
            "webhook-017-delivery",
            Err(ReconcileError::Source(
                ApiReadError::HttpStatusWithRetryAfter {
                    status: 429,
                    retry_after_ms: 30_000,
                },
            )),
            1,
        );

        assert!(!exit_success);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["error"]["code"], "upstream_http_status");
        assert_eq!(value["error"]["retryable"], true);
        assert_eq!(value["error"]["details"]["http_status"], 429);
        assert_eq!(value["error"]["details"]["retry_after_ms"], 30_000);
        assert!(
            value["error"]["action"]
                .as_str()
                .unwrap()
                .contains("retry_after_ms")
        );
        assert!(value.get("canonical_state").is_none());
    }

    #[test]
    fn retry_after_delay_conversion_saturates_without_overflow() {
        let failure = ForgejoReadError {
            error: ApiReadError::HttpStatus(429),
            retry_after: Some(RetryAfterValue::Delay(Duration::from_secs(u64::MAX))),
        };

        assert_eq!(
            failure.as_api_read_error(),
            ApiReadError::HttpStatusWithRetryAfter {
                status: 429,
                retry_after_ms: u64::MAX,
            }
        );
    }

    #[test]
    fn invalid_snapshot_error_is_not_retryable_success() {
        let (value, exit_success) = single_reconciliation_result(
            "webhook-017-delivery",
            Err(ReconcileError::InvalidCurrentState),
            1,
        );

        assert!(!exit_success);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["error"]["code"], "invalid_current_state");
        assert_eq!(value["error"]["retryable"], false);
        assert!(!value["error"]["action"].as_str().unwrap().is_empty());
    }

    #[test]
    fn batch_level_failure_is_structured_and_inconclusive() {
        let (value, exit_success) =
            batch_reconciliation_failure(&ReconcileError::BatchTooLarge, 101, 0);

        assert!(!exit_success);
        assert_eq!(value["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["requested_delivery_count"], 101);
        assert_eq!(value["upstream_read_attempts"], 0);
        assert_eq!(value["error"]["code"], "batch_too_large");
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(value["error"]["details"]["maximum_events"], 100);
        assert!(!value["error"]["action"].as_str().unwrap().is_empty());
    }

    #[test]
    fn malformed_selected_delivery_is_refused_before_host_credential_read() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        // Simulate a chain-valid inbox record from an older writer. Current
        // ingestion refuses malformed resource bodies before append.
        write_legacy_webhook_fixture(
            &journal_path,
            "malformed-delivery-41",
            "issues",
            json!({
                "action": "opened",
                "issue": {"number": 41, "state": "open"}
            }),
        );

        let outcome = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: vec!["malformed-delivery-41".to_owned()],
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: Vec::new(),
        });
        assert!(
            outcome.is_ok(),
            "invalid resource must be rejected before accessing the credential FD: {outcome:?}"
        );
        let (result, exit_success) = outcome.unwrap();

        assert!(!exit_success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 0);
        assert_eq!(result["error"]["code"], "invalid_repository");
        assert_eq!(result["error"]["retryable"], false);
    }

    #[test]
    fn mismatched_review_subtype_is_refused_before_host_credential_read() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        write_legacy_webhook_fixture(
            &journal_path,
            "legacy-review-subtype-mismatch",
            "pull_request_approved",
            json!({
                "action": "reviewed",
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "pull_request": {"number": 14},
                "review": {"type": "pull_request_review_rejected"}
            }),
        );

        let outcome = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: vec!["legacy-review-subtype-mismatch".to_owned()],
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: Vec::new(),
        });
        assert!(
            outcome.is_ok(),
            "mismatched review type must be rejected before reading the host credential: {outcome:?}"
        );
        let (result, exit_success) = outcome.unwrap();

        assert!(!exit_success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 0);
        assert_eq!(result["error"]["code"], "invalid_envelope");
        assert_eq!(result["error"]["retryable"], false);
    }

    #[test]
    fn out_of_scope_selected_delivery_is_refused_before_host_credential_read() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-out-of-scope-delivery-secret";
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": "other-owner/other-repository"},
            "issue": {"number": 41, "state": "open"}
        }))
        .unwrap();
        let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
        mac.update(&body);
        let signature = hex::encode(mac.finalize().into_bytes());
        assert!(matches!(
            ingest_forgejo_delivery(
                &journal,
                secret,
                "out-of-scope-delivery-41",
                "issues",
                &signature,
                &body
            )
            .unwrap(),
            AppendResult::Appended(_)
        ));

        let outcome = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: vec!["out-of-scope-delivery-41".to_owned()],
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: Vec::new(),
        });
        assert!(
            outcome.is_ok(),
            "out-of-scope resource must be refused before accessing the credential FD: {outcome:?}"
        );
        let (result, exit_success) = outcome.unwrap();

        assert!(!exit_success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 0);
        assert_eq!(result["error"]["code"], "resource_out_of_scope");
        assert_eq!(result["error"]["retryable"], false);
        assert!(
            result["error"]["action"]
                .as_str()
                .unwrap()
                .contains("matching configured repository")
        );
        assert_eq!(
            result["error"]["details"]["configured_repository"],
            "forge-admin/agent-forge-core"
        );
        assert_eq!(
            result["error"]["details"]["event_repository"],
            "other-owner/other-repository"
        );
    }

    #[test]
    fn same_repository_forbidden_error_keeps_upstream_permission_classification() {
        let event = JournalEvent {
            sequence: 1,
            event_id: "same-repository-forbidden".to_owned(),
            idempotency_key: "forgejo-delivery-v1:same-repository-forbidden".to_owned(),
            kind: "forgejo.webhook.v1.issues".to_owned(),
            payload: json!({
                "schema_version": 1,
                "provider": "forgejo",
                "delivery_id": "same-repository-forbidden",
                "event": "issues",
                "payload": {
                    "action": "opened",
                    "repository": {"full_name": "forge-admin/agent-forge-core"},
                    "issue": {"number": 41, "state": "open"}
                }
            }),
            previous_hash: "0".repeat(64),
            receiver_attestation: None,
            hash: "1".repeat(64),
        };
        let mut error =
            reconciliation_error_value(&ReconcileError::Source(ApiReadError::Forbidden));

        classify_out_of_scope_delivery_error(
            &mut error,
            &event,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
        );

        assert_eq!(error["code"], "upstream_forbidden");
        assert_eq!(error["retryable"], false);
        assert_eq!(error["details"]["upstream"], "forgejo");
    }

    #[test]
    fn mixed_batch_reads_host_credential_only_for_in_scope_rows() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-mixed-batch-credential-secret";
        let signing_key = SigningKey::from_bytes(&[0x33; 32]);
        let deliveries = [
            (
                "mixed-in-scope-issue-41",
                json!({
                    "action": "opened",
                    "repository": {"full_name": "forge-admin/agent-forge-core"},
                    "issue": {"number": 41, "state": "open"}
                }),
            ),
            (
                "mixed-out-of-scope-issue-41",
                json!({
                    "action": "opened",
                    "repository": {"full_name": "other-owner/other-repository"},
                    "issue": {"number": 41, "state": "open"}
                }),
            ),
        ];
        for (delivery_id, value) in deliveries {
            let body = serde_json::to_vec(&value).unwrap();
            let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
            mac.update(&body);
            let signature = hex::encode(mac.finalize().into_bytes());
            assert!(matches!(
                ingest_forgejo_delivery_attested(
                    &journal,
                    secret,
                    &signing_key,
                    delivery_id,
                    "issues",
                    &signature,
                    &body,
                )
                .unwrap(),
                AppendResult::Appended(_)
            ));
        }

        let (origin, server) = start_current_issue_http_fixture();
        let credential = b"synthetic-token-0123456789";
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let mut writer = unsafe { File::from_raw_fd(descriptors[1]) };
        writer.write_all(credential).unwrap();
        drop(writer);
        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: origin.clone(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: vec![
                "mixed-in-scope-issue-41".to_owned(),
                "mixed-out-of-scope-issue-41".to_owned(),
            ],
            credential_fd: Some(descriptors[0]),
            trusted_attestation_keys: vec![signing_key.verifying_key()],
        })
        .unwrap();

        assert!(!exit_success);
        assert_eq!(result["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 1);
        let rows = result["results"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["verdict"], "current_state_read");
        assert_eq!(rows[0]["delivery_id"], "mixed-in-scope-issue-41");
        assert_eq!(rows[1]["verdict"], "inconclusive");
        assert_eq!(rows[1]["delivery_id"], "mixed-out-of-scope-issue-41");
        assert_eq!(rows[1]["error"]["code"], "resource_out_of_scope");

        let request = server.join().unwrap();
        assert!(
            request
                .contains("GET /api/v1/repos/forge-admin/agent-forge-core/issues/41 HTTP/1.1\r\n")
        );
        assert!(!request.contains("other-owner/other-repository"));
        assert!(request.contains("Authorization: token synthetic-token-0123456789\r\n"));
    }

    #[test]
    fn denied_batch_is_refused_before_host_credential_read() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-denied-batch-credential-secret";
        let deliveries = [
            (
                "denied-malformed-issue-41",
                json!({
                    "action": "opened",
                    "issue": {"number": 41, "state": "open"}
                }),
            ),
            (
                "denied-out-of-scope-issue-42",
                json!({
                    "action": "opened",
                    "repository": {"full_name": "other-owner/other-repository"},
                    "issue": {"number": 42, "state": "open"}
                }),
            ),
        ];
        for (delivery_id, value) in deliveries {
            if value.get("repository").is_none() {
                write_legacy_webhook_fixture(&journal_path, delivery_id, "issues", value);
                continue;
            }
            let body = serde_json::to_vec(&value).unwrap();
            let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
            mac.update(&body);
            let signature = hex::encode(mac.finalize().into_bytes());
            assert!(matches!(
                ingest_forgejo_delivery(&journal, secret, delivery_id, "issues", &signature, &body)
                    .unwrap(),
                AppendResult::Appended(_)
            ));
        }

        let outcome = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids: vec![
                "denied-malformed-issue-41".to_owned(),
                "denied-out-of-scope-issue-42".to_owned(),
            ],
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: Vec::new(),
        });
        assert!(
            outcome.is_ok(),
            "a denied batch must not consume or validate the host credential FD: {outcome:?}"
        );
        let (result, exit_success) = outcome.unwrap();

        assert!(!exit_success);
        assert_eq!(result["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["upstream_read_attempts"], 0);
        let rows = result["results"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["delivery_id"], "denied-malformed-issue-41");
        assert_eq!(rows[0]["error"]["code"], "invalid_repository");
        assert_eq!(rows[1]["delivery_id"], "denied-out-of-scope-issue-42");
        assert_eq!(rows[1]["error"]["code"], "resource_out_of_scope");
    }

    #[test]
    fn duplicate_delivery_selection_is_refused_before_host_credential_read() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        write_legacy_webhook_fixture(
            &journal_path,
            "duplicate-selection-issue-41",
            "issues",
            json!({
                "action": "opened",
                "repository": {"full_name": "forge-admin/agent-forge-core"},
                "issue": {"number": 41, "state": "open"}
            }),
        );
        let journal_before = fs::read(&journal_path).unwrap();

        let outcome = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.clone().into_os_string(),
            delivery_ids: vec![
                "duplicate-selection-issue-41".to_owned(),
                "duplicate-selection-issue-41".to_owned(),
            ],
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: Vec::new(),
        });
        assert!(
            outcome.is_ok(),
            "duplicate selection must be refused before reading the credential: {outcome:?}"
        );
        let (result, exit_success) = outcome.unwrap();

        assert!(!exit_success);
        assert_eq!(result["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["requested_delivery_count"], 2);
        assert_eq!(result["upstream_read_attempts"], 0);
        assert_eq!(result["error"]["code"], "duplicate_delivery_selection");
        assert_eq!(result["error"]["retryable"], false);
        assert!(
            result["error"]["action"]
                .as_str()
                .unwrap()
                .contains("remove")
        );
        assert_eq!(
            result["error"]["details"]["selection"],
            "delivery_ids_must_be_unique"
        );
        assert_eq!(fs::read(&journal_path).unwrap(), journal_before);
    }

    #[test]
    fn oversized_batch_is_rejected_before_host_credential_read() {
        let scratch = ScratchJournalDir::new();
        let journal_path = scratch.0.join("inbox.jsonl");
        let journal = EventJournal::open(&journal_path);
        let secret = b"synthetic-oversized-batch-secret";
        let body = serde_json::to_vec(&json!({
            "action": "opened",
            "repository": {"full_name": "forge-admin/agent-forge-core"},
            "issue": {"number": 41, "state": "open"}
        }))
        .unwrap();
        let mut mac = FixtureHmacSha256::new_from_slice(secret).unwrap();
        mac.update(&body);
        let signature = hex::encode(mac.finalize().into_bytes());
        assert!(matches!(
            ingest_forgejo_delivery(
                &journal,
                secret,
                "oversized-batch-delivery",
                "issues",
                &signature,
                &body
            )
            .unwrap(),
            AppendResult::Appended(_)
        ));

        let delivery_ids =
            vec!["oversized-batch-delivery".to_owned(); MAX_RECONCILE_BATCH_EVENTS + 1];
        let (result, exit_success) = run_reconciliation(ReconcileInvocationV1 {
            config: LocalForgejoConfigV1 {
                origin: LOCAL_FORGEJO_ORIGIN.to_owned(),
                owner: LOCAL_REPOSITORY_OWNER.to_owned(),
                repository: LOCAL_REPOSITORY_NAME.to_owned(),
            },
            journal_path: journal_path.into_os_string(),
            delivery_ids,
            credential_fd: Some(i32::MAX),
            trusted_attestation_keys: Vec::new(),
        })
        .expect(
            "oversized input must return a structured refusal without opening the credential FD",
        );

        assert!(!exit_success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(
            result["requested_delivery_count"],
            MAX_RECONCILE_BATCH_EVENTS + 1
        );
        assert_eq!(result["upstream_read_attempts"], 0);
        assert_eq!(result["error"]["code"], "batch_too_large");
        assert_eq!(
            result["error"]["details"]["maximum_events"],
            MAX_RECONCILE_BATCH_EVENTS
        );
    }

    #[test]
    fn batch_cache_budget_failure_is_structured_and_nonretryable() {
        let error = ReconcileError::Source(ApiReadError::BatchCacheBudgetExceeded {
            maximum_cached_json_bytes: MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
        });
        let (value, exit_success) = batch_reconciliation_failure(&error, 4, 2);

        assert!(!exit_success);
        assert_eq!(value["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["requested_delivery_count"], 4);
        assert_eq!(value["upstream_read_attempts"], 2);
        assert_eq!(value["error"]["code"], "batch_cache_budget_exceeded");
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(
            value["error"]["details"]["maximum_cached_json_bytes"],
            MAX_RECONCILE_BATCH_CACHED_JSON_BYTES
        );
        assert_eq!(
            value["error"]["details"]["measurement"],
            "combined_uncompressed_serialized_json"
        );
        assert!(value["error"]["action"].as_str().unwrap().contains("split"));
        assert!(!transient_api_read_error(
            &ApiReadError::BatchCacheBudgetExceeded {
                maximum_cached_json_bytes: MAX_RECONCILE_BATCH_CACHED_JSON_BYTES,
            }
        ));
    }

    #[test]
    fn batch_output_budget_failure_is_structured_and_nonretryable() {
        let error = ReconcileError::BatchOutputBudgetExceeded {
            maximum_output_bytes: MAX_RECONCILE_BATCH_OUTPUT_BYTES,
        };
        let (value, exit_success) = batch_reconciliation_failure(&error, 100, 2);

        assert!(!exit_success);
        assert_eq!(value["schema_version"], RECONCILE_BATCH_SCHEMA_VERSION);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["requested_delivery_count"], 100);
        assert_eq!(value["upstream_read_attempts"], 2);
        assert_eq!(value["error"]["code"], "batch_result_budget_exceeded");
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(
            value["error"]["details"]["maximum_projected_output_bytes"],
            MAX_RECONCILE_BATCH_OUTPUT_BYTES
        );
        assert_eq!(
            value["error"]["details"]["measurement"],
            "serialized_canonical_state_json_plus_2048_bytes_per_delivery_for_cli_envelope"
        );
        assert!(value["error"]["action"].as_str().unwrap().contains("split"));
        assert!(
            value["error"]["action"]
                .as_str()
                .unwrap()
                .contains("do not retry")
        );
    }

    #[test]
    fn merge_preflight_success_is_read_only_and_not_merge_authority() {
        let (value, exit_success) = merge_preflight_result(23, 1, Ok(()));

        assert!(exit_success);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["operation"], "read_only_merge_preflight");
        assert_eq!(value["verdict"], "proof_current");
        assert_eq!(value["merge_authorized"], false);
        assert_eq!(value["merge_attempted"], false);
        assert_eq!(value["upstream_read_attempts"], 1);
    }

    #[test]
    fn stale_merge_preflight_is_actionable_and_inconclusive() {
        let (value, exit_success) = merge_preflight_result(
            23,
            1,
            Err(agent_forge_core::forgejo::MergePreflightError::StaleBinding("base")),
        );

        assert!(!exit_success);
        assert_eq!(value["operation"], "read_only_merge_preflight");
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["merge_authorized"], false);
        assert_eq!(value["merge_attempted"], false);
        assert_eq!(value["error"]["code"], "stale_base");
        assert_eq!(value["error"]["retryable"], false);
        assert!(!value["error"]["action"].as_str().unwrap().is_empty());
    }

    #[test]
    fn merge_preflight_unauthorized_error_explains_anonymous_read_contract() {
        let (value, exit_success) = merge_preflight_api_failure(23, 1, ApiReadError::Unauthorized);

        assert!(!exit_success);
        assert_eq!(value["verdict"], "inconclusive");
        assert_eq!(value["error"]["code"], "upstream_unauthorized");
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(value["error"]["details"]["http_status"], 401);
        let action = value["error"]["action"].as_str().unwrap();
        assert!(action.contains("anonymously readable"));
        assert!(action.contains("sends no credential"));
        assert!(action.contains("host-authenticated merge authority"));
    }

    #[test]
    fn merge_preflight_keeps_failure_distinct_from_inconclusive() {
        let (failed, failed_exit) = merge_preflight_result(
            23,
            1,
            Err(agent_forge_core::forgejo::MergePreflightError::Proof(
                agent_forge_core::GateError::OutcomeFailed,
            )),
        );
        let (inconclusive, inconclusive_exit) = merge_preflight_result(
            23,
            1,
            Err(agent_forge_core::forgejo::MergePreflightError::Proof(
                agent_forge_core::GateError::OutcomeInconclusive,
            )),
        );

        assert!(!failed_exit && !inconclusive_exit);
        assert_eq!(failed["verdict"], "rejected");
        assert_eq!(failed["error"]["code"], "proof_failed");
        assert_eq!(inconclusive["verdict"], "inconclusive");
        assert_eq!(inconclusive["error"]["code"], "proof_inconclusive");
        assert_eq!(failed["merge_authorized"], false);
        assert_eq!(inconclusive["merge_authorized"], false);
    }

    #[test]
    fn merge_authority_api_refusal_reports_a_bounded_stage() {
        let (result, success) = merge_authority_api_error_result(
            23,
            "branch-protection-read",
            &ApiReadError::Forbidden,
        );

        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "upstream_forbidden");
        assert_eq!(result["error"]["stage"], "branch-protection-read");
        assert_eq!(result["pull_request_number"], 23);
    }

    // MERGE-015: compile-time target profiles for the local merge gate.

    const PROFILE_BASE_SHA: &str = "1111111111111111111111111111111111111111";
    const PROFILE_HEAD_SHA: &str = "2222222222222222222222222222222222222222";
    const PROFILE_MERGE_SHA: &str = "3333333333333333333333333333333333333333";
    const PROFILE_FIXTURE_TOKEN: &str = "synthetic-token-0123456789";
    const PROFILE_VERIFIER_KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn os_args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn target_profile_selector_accepts_only_exact_literals() {
        assert_eq!(
            LocalForgejoTargetProfile::parse("merge-014-agent-forge-core"),
            Ok(LocalForgejoTargetProfile::Merge014AgentForgeCore)
        );
        assert_eq!(
            LocalForgejoTargetProfile::parse("mission-012-clamp"),
            Ok(LocalForgejoTargetProfile::Mission012Clamp)
        );
        for rejected in [
            "",
            "default",
            "mission-012",
            "MISSION-012-CLAMP",
            "Mission-012-Clamp",
            " mission-012-clamp",
            "mission-012-clamp ",
            "mission-012-clamp\n",
            "mission-012-clamp\0",
            "mission-012-clamp,merge-014-agent-forge-core",
            "forge-admin/mission-012-clamp",
            "http://127.0.0.1:3132",
            "http://127.0.0.1:3132/forge-admin/mission-012-clamp",
            "http://localhost:3132",
        ] {
            assert!(
                LocalForgejoTargetProfile::parse(rejected).is_err(),
                "selector {rejected:?} must be refused"
            );
        }
    }

    #[test]
    fn target_profiles_bind_origin_owner_and_repository_as_one_literal_tuple() {
        let merge_014 = LocalForgejoTargetProfile::Merge014AgentForgeCore.config();
        assert_eq!(
            (
                merge_014.origin.as_str(),
                merge_014.owner.as_str(),
                merge_014.repository.as_str()
            ),
            ("http://127.0.0.1:3130", "forge-admin", "agent-forge-core")
        );
        let mission_012 = LocalForgejoTargetProfile::Mission012Clamp.config();
        assert_eq!(
            (
                mission_012.origin.as_str(),
                mission_012.owner.as_str(),
                mission_012.repository.as_str()
            ),
            ("http://127.0.0.1:3132", "forge-admin", "mission-012-clamp")
        );
        let default = LocalForgejoConfigV1::default();
        assert_eq!(
            (default.origin, default.owner, default.repository),
            (merge_014.origin, merge_014.owner, merge_014.repository)
        );
        for profile in LocalForgejoTargetProfile::ALL {
            assert_eq!(
                LocalForgejoTargetProfile::parse(profile.selector()),
                Ok(profile)
            );
            assert!(profile.origin().starts_with("http://127.0.0.1:"));
        }
    }

    #[test]
    fn merge_cli_without_profile_selection_keeps_the_default_target() {
        let positional = os_args(&["23", "p", "e", "k", "policy", "binding", "agent", "9"]);
        let (profile, rest) = split_merge_target_profile(&positional).unwrap();
        assert_eq!(profile, LocalForgejoTargetProfile::Merge014AgentForgeCore);
        assert_eq!(rest, positional.as_slice());

        let mut explicit = os_args(&["--target-profile", "merge-014-agent-forge-core"]);
        explicit.extend(positional.iter().cloned());
        let (profile, rest) = split_merge_target_profile(&explicit).unwrap();
        assert_eq!(profile, LocalForgejoTargetProfile::Merge014AgentForgeCore);
        assert_eq!(rest, positional.as_slice());

        let mut selected = os_args(&["--target-profile", "mission-012-clamp"]);
        selected.extend(positional.iter().cloned());
        let (profile, rest) = split_merge_target_profile(&selected).unwrap();
        assert_eq!(profile, LocalForgejoTargetProfile::Mission012Clamp);
        assert_eq!(rest, positional.as_slice());
    }

    #[test]
    fn mission_merge_cli_requires_admission_and_journal_before_reading_credentials() {
        let mut mission = os_args(&["--target-profile", "mission-012-clamp"]);
        mission.extend(os_args(&[
            "23", "proof", "expected", "key", "policy", "binding", "agent", "not-a-fd",
        ]));
        let error = run_local_test_merge_authority(mission.into_iter())
            .expect_err("MISSION-012 must not merge without durable admission inputs");
        assert!(error.contains(MISSION_ADMISSION_FILE_FLAG), "{error}");
        assert!(error.contains(MISSION_EXECUTION_JOURNAL_FLAG), "{error}");

        let mut default = os_args(&[
            MISSION_ADMISSION_FILE_FLAG,
            "/private/admission.json",
            MISSION_EXECUTION_JOURNAL_FLAG,
            "/private/missions.jsonl",
        ]);
        default.extend(os_args(&[
            "23", "proof", "expected", "key", "policy", "binding", "agent", "not-a-fd",
        ]));
        let error = run_local_test_merge_authority(default.into_iter())
            .expect_err("the default MERGE-014 profile must reject mission-specific inputs");
        assert!(
            error.contains("valid only for the mission-012-clamp target"),
            "{error}"
        );
    }

    #[test]
    fn unknown_or_malformed_profile_is_refused_before_credential_or_inputs_are_read() {
        use std::os::unix::ffi::OsStringExt;
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let (read_fd, write_fd) = (descriptors[0], descriptors[1]);
        let token = PROFILE_FIXTURE_TOKEN.as_bytes();
        assert_eq!(
            unsafe { libc::write(write_fd, token.as_ptr().cast(), token.len()) },
            token.len() as isize
        );
        unsafe { libc::close(write_fd) };
        let fd = read_fd.to_string();
        // Input paths that do not exist: if any were opened first, the error
        // would name the input instead of the target profile.
        let tail = [
            "23",
            "/nonexistent/merge-015/proof.json",
            "/nonexistent/merge-015/expected.json",
            "/nonexistent/merge-015/key",
            "/nonexistent/merge-015/policy.json",
            "/nonexistent/merge-015/binding.json",
            "fixture-agent",
            fd.as_str(),
        ];
        let with_prefix = |prefix: &[&str]| {
            let mut values = prefix.to_vec();
            values.extend_from_slice(&tail);
            os_args(&values)
        };
        let cases = vec![
            with_prefix(&["--target-profile", "mission-012"]),
            with_prefix(&["--target-profile", "MISSION-012-CLAMP"]),
            with_prefix(&["--target-profile", "http://127.0.0.1:3132"]),
            with_prefix(&["--target-profile", ""]),
            with_prefix(&["--target-profile=mission-012-clamp"]),
            with_prefix(&["--profile", "mission-012-clamp"]),
            with_prefix(&["--target-profile"]),
            with_prefix(&[
                "--target-profile",
                "mission-012-clamp",
                "--target-profile",
                "mission-012-clamp",
            ]),
            {
                let mut values = os_args(&["--target-profile"]);
                values.push(std::ffi::OsString::from_vec(
                    b"mission-012-\xffclamp".to_vec(),
                ));
                values.extend(os_args(&tail));
                values
            },
        ];
        for arguments in cases {
            let rendered = format!("{arguments:?}");
            let error = run_local_test_merge_authority(arguments.into_iter())
                .expect_err("malformed selection must be refused");
            assert!(
                error.contains("target profile") || error.contains("usage:"),
                "{rendered}: unexpected refusal {error:?}"
            );
            assert!(
                !error.contains("input") && !error.contains("credential"),
                "{rendered}: refusal happened after reading inputs: {error:?}"
            );
        }

        // The descriptor was never touched: no close-on-exec mark and the
        // synthetic token bytes are still unread in the pipe.
        let descriptor_flags = unsafe { libc::fcntl(read_fd, libc::F_GETFD) };
        assert!(descriptor_flags >= 0, "credential descriptor was closed");
        assert_eq!(descriptor_flags & libc::FD_CLOEXEC, 0);
        let mut remaining = [0_u8; 64];
        let count = unsafe { libc::read(read_fd, remaining.as_mut_ptr().cast(), remaining.len()) };
        unsafe { libc::close(read_fd) };
        assert_eq!(&remaining[..count as usize], token);
    }

    #[test]
    fn valid_single_writer_selector_refuses_before_reaching_the_credential_reader() {
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let (read_fd, write_fd) = (descriptors[0], descriptors[1]);
        // No capability descriptor was supplied. The gate must refuse before
        // consuming this inherited credential or opening mission inputs.
        let token = b"not a valid token";
        assert_eq!(
            unsafe { libc::write(write_fd, token.as_ptr().cast(), token.len()) },
            token.len() as isize
        );
        unsafe { libc::close(write_fd) };
        let fd = read_fd.to_string();
        let arguments = os_args(&[
            MERGE_WRITER_ADMISSION_PROFILE_FLAG,
            "local_single_writer_sandbox",
            "--target-profile",
            "mission-012-clamp",
            MISSION_ADMISSION_FILE_FLAG,
            "/nonexistent/host-admission.json",
            MISSION_EXECUTION_JOURNAL_FLAG,
            "/nonexistent/mission-execution.jsonl",
            "23",
            "/nonexistent/merge-015/proof.json",
            "/nonexistent/merge-015/expected.json",
            "/nonexistent/merge-015/key",
            "/nonexistent/merge-015/policy.json",
            "/nonexistent/merge-015/binding.json",
            "fixture-agent",
            fd.as_str(),
        ]);
        let (result, success) = run_local_test_merge_authority(arguments.into_iter())
            .expect("missing single-writer attestation is a structured refusal");
        assert!(!success);
        assert!(
            result["error"]["code"] == "supervisor_capability_missing",
            "{result}"
        );
        assert_eq!(result["merge_attempted"], false);
        assert_eq!(result["merge_authorized"], false);

        let mut remaining = [0_u8; 64];
        let count = unsafe { libc::read(read_fd, remaining.as_mut_ptr().cast(), remaining.len()) };
        unsafe { libc::close(read_fd) };
        assert_eq!(&remaining[..count as usize], token);
    }

    #[test]
    fn duplicate_or_trailing_options_after_a_profile_are_refused_with_distinct_messages() {
        let tail = os_args(&["23", "p", "e", "k", "policy", "binding", "agent", "9"]);
        let with = |prefix: &[&str]| {
            let mut values = os_args(prefix);
            values.extend(tail.iter().cloned());
            values
        };
        let duplicate = with(&[
            "--target-profile",
            "mission-012-clamp",
            "--target-profile",
            "mission-012-clamp",
        ]);
        let error = split_merge_target_profile(&duplicate).unwrap_err();
        assert!(error.contains("only once"), "{error:?}");
        let unknown = with(&["--target-profile", "mission-012-clamp", "--force"]);
        let error = split_merge_target_profile(&unknown).unwrap_err();
        assert!(
            error.contains("unknown --merge-local-test option"),
            "{error:?}"
        );
    }

    #[test]
    fn reconciliation_only_profile_is_refused_by_merge_before_credential_read() {
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let (read_fd, write_fd) = (descriptors[0], descriptors[1]);
        let token = PROFILE_FIXTURE_TOKEN.as_bytes();
        assert_eq!(
            unsafe { libc::write(write_fd, token.as_ptr().cast(), token.len()) },
            token.len() as isize
        );
        unsafe { libc::close(write_fd) };

        let mut arguments = os_args(&[
            "--reconcile-target-profile",
            "org-team-review-e2e",
            "23",
            "proof",
            "expected",
            "key",
            "policy",
            "binding",
            "agent",
        ]);
        arguments.push(read_fd.to_string().into());
        let error = run_local_test_merge_authority(arguments.into_iter())
            .expect_err("a reconciliation-only profile must not enter merge authority");
        assert!(
            error.contains("unknown --merge-local-test option"),
            "{error}"
        );

        let mut remaining = [0_u8; 64];
        let count = unsafe { libc::read(read_fd, remaining.as_mut_ptr().cast(), remaining.len()) };
        unsafe { libc::close(read_fd) };
        assert_eq!(&remaining[..count as usize], token);
    }

    #[test]
    fn reconciliation_only_profile_value_is_refused_before_credential_read() {
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let (read_fd, write_fd) = (descriptors[0], descriptors[1]);
        let token = PROFILE_FIXTURE_TOKEN.as_bytes();
        assert_eq!(
            unsafe { libc::write(write_fd, token.as_ptr().cast(), token.len()) },
            token.len() as isize
        );
        unsafe { libc::close(write_fd) };

        let mut arguments = os_args(&[
            "--target-profile",
            "org-team-review-e2e",
            "23",
            "proof",
            "expected",
            "key",
            "policy",
            "binding",
            "agent",
        ]);
        arguments.push(read_fd.to_string().into());
        let error = run_local_test_merge_authority(arguments.into_iter())
            .expect_err("a reconciliation-only profile value must not select merge authority");
        assert!(error.contains("unknown Forgejo target profile"), "{error}");

        let mut remaining = [0_u8; 64];
        let count = unsafe { libc::read(read_fd, remaining.as_mut_ptr().cast(), remaining.len()) };
        unsafe { libc::close(read_fd) };
        assert_eq!(&remaining[..count as usize], token);
    }

    #[test]
    fn cli_profile_wiring_binds_api_scope_and_credential_to_the_same_profile() {
        let token = ForgejoApiToken::parse(PROFILE_FIXTURE_TOKEN.as_bytes().to_vec()).unwrap();
        for profile in LocalForgejoTargetProfile::ALL {
            let api = merge_api_for_profile(profile, &token);
            assert_eq!(
                (
                    api.config.origin.as_str(),
                    api.config.owner.as_str(),
                    api.config.repository.as_str()
                ),
                (profile.origin(), profile.owner(), profile.repository()),
                "{profile:?}"
            );
            let provider = api.credential_provider.as_ref().expect("bound credential");
            assert_eq!(provider.origin, profile.origin(), "{profile:?}");
            assert!(matches!(provider.token_for(profile.origin()), Ok(Some(_))));
            for other in LocalForgejoTargetProfile::ALL
                .into_iter()
                .filter(|other| *other != profile)
            {
                assert!(
                    matches!(provider.token_for(other.origin()), Ok(None)),
                    "{profile:?} credential must be withheld from {other:?}"
                );
            }
        }
    }

    fn write_private_fixture(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = directory.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    fn read_fixture_http_request(stream: &mut TcpStream) -> (String, String) {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        let header_end = loop {
            let count = stream.read(&mut buffer).unwrap();
            assert_ne!(count, 0, "HTTP request ended before its headers");
            request.extend_from_slice(&buffer[..count]);
            if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            assert!(
                request.len() < 16 * 1024,
                "HTTP request headers are oversized"
            );
        };
        let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|length| length.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        while request.len() - header_end < content_length {
            let count = stream.read(&mut buffer).unwrap();
            assert_ne!(count, 0, "HTTP request body is truncated");
            request.extend_from_slice(&buffer[..count]);
        }
        (headers.lines().next().unwrap().to_owned(), headers)
    }

    /// Fixture origin, stop flag, and recorded (request line, carried bound
    /// credential) pairs.
    type ProfileGateFixture = (
        String,
        Arc<std::sync::atomic::AtomicBool>,
        thread::JoinHandle<Vec<(String, String)>>,
    );

    /// Stateful loopback Forgejo stand-in for one selected repository. Every
    /// other repository answers 404 so a mixed-scope request cannot succeed.
    fn start_profile_gate_fixture(
        owner: &'static str,
        repository: &'static str,
    ) -> ProfileGateFixture {
        start_profile_gate_fixture_with_protection(
            owner,
            repository,
            merge_authority_branch_protection_fixture(),
        )
    }

    fn start_profile_gate_fixture_with_protection(
        owner: &'static str,
        repository: &'static str,
        branch_protection: Value,
    ) -> ProfileGateFixture {
        start_profile_gate_fixture_with_protection_changes(
            owner,
            repository,
            branch_protection,
            None,
        )
    }

    fn start_profile_gate_fixture_with_protection_changes(
        owner: &'static str,
        repository: &'static str,
        branch_protection: Value,
        final_branch_protection: Option<Value>,
    ) -> ProfileGateFixture {
        start_profile_gate_fixture_with_role_credentials(
            owner,
            repository,
            branch_protection,
            final_branch_protection,
            "",
            "",
            "forge-admin",
            vec![1],
        )
    }

    fn start_profile_gate_fixture_with_role_credentials(
        owner: &'static str,
        repository: &'static str,
        branch_protection: Value,
        final_branch_protection: Option<Value>,
        merge_token: &'static str,
        policy_token: &'static str,
        merge_actor_login: &'static str,
        review_page_totals: Vec<u64>,
    ) -> ProfileGateFixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let fixture_origin = origin.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = Arc::clone(&done);
        let server = thread::spawn(move || {
            let repo = format!("/api/v1/repos/{owner}/{repository}");
            let mut merged = false;
            let mut branch_protection_reads = 0;
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(20);
            while !stop.load(Ordering::SeqCst) {
                assert!(Instant::now() < deadline, "profile gate fixture timed out");
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("profile gate fixture accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                let (request_line, headers) = read_fixture_http_request(&mut stream);
                let authorization = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Authorization: token "))
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_owned();
                requests.push((request_line.clone(), authorization.clone()));
                let target = request_line
                    .split(' ')
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let method = request_line.split(' ').next().unwrap_or_default();
                let pull_url = format!("{fixture_origin}/{owner}/{repository}/pulls/23");
                let full_name = format!("{owner}/{repository}");
                let expected_token = if target == format!("{repo}/branch_protections") {
                    policy_token
                } else {
                    merge_token
                };
                let (status, total_count, body) = if !expected_token.is_empty()
                    && authorization != expected_token
                {
                    (
                        403,
                        None,
                        br#"{"message":"wrong fixture credential"}"#.to_vec(),
                    )
                } else if method == "POST" && target == format!("{repo}/pulls/23/merge") {
                    merged = true;
                    (200, None, Vec::new())
                } else if method != "GET" {
                    (405, None, b"{}".to_vec())
                } else if target == format!("{repo}/pulls/23") && !merged {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({
                            "number": 23,
                            "html_url": pull_url,
                            "state": "open",
                            "merged": false,
                            "draft": false,
                            "mergeable": true,
                            "base": {"ref": "main", "sha": PROFILE_BASE_SHA, "repo": {"full_name": full_name}},
                            "head": {"sha": PROFILE_HEAD_SHA, "repo": {"full_name": full_name}},
                        }))
                        .unwrap(),
                    )
                } else if target == format!("{repo}/pulls/23") {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({
                            "number": 23,
                            "html_url": pull_url,
                            "state": "closed",
                            "merged": true,
                            "merge_commit_sha": PROFILE_MERGE_SHA,
                            "base": {"ref": "main", "sha": PROFILE_MERGE_SHA, "repo": {"full_name": full_name}},
                            "head": {"sha": PROFILE_HEAD_SHA, "repo": {"full_name": full_name}},
                        }))
                        .unwrap(),
                    )
                } else if target == "/api/v1/user" {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({"id":9,"login":merge_actor_login})).unwrap(),
                    )
                } else if let Some(page) = target
                    .strip_prefix(&format!("{repo}/pulls/23/reviews?limit=50&page="))
                    .and_then(|page| page.parse::<usize>().ok())
                    .filter(|page| *page > 0)
                {
                    match review_page_totals.get(page - 1).copied() {
                        Some(total_count) if total_count > 0 => {
                            let page_start = (page - 1) as u64 * 50;
                            let page_len = total_count.saturating_sub(page_start).min(50) as usize;
                            let reviews = (0..page_len)
                                .map(|offset| {
                                    json!({
                                        "id": if total_count == 1 && page == 1 {
                                            17
                                        } else {
                                            page_start + offset as u64 + 1
                                        },
                                        "user": {"id": 42},
                                        "state": "APPROVED",
                                        "commit_id": PROFILE_HEAD_SHA,
                                        "pull_request_url": pull_url,
                                        "stale": false,
                                        "official": true,
                                        "dismissed": false,
                                    })
                                })
                                .collect::<Vec<_>>();
                            (
                                200,
                                Some(total_count),
                                serde_json::to_vec(&reviews).unwrap(),
                            )
                        }
                        _ => (404, None, br#"{"message":"not found"}"#.to_vec()),
                    }
                } else if target == format!("{repo}/branch_protections") {
                    let protection = if branch_protection_reads == 0 {
                        &branch_protection
                    } else {
                        final_branch_protection
                            .as_ref()
                            .unwrap_or(&branch_protection)
                    };
                    branch_protection_reads += 1;
                    (200, None, serde_json::to_vec(protection).unwrap())
                } else if target == "/api/v1/users/verifier-bot" {
                    (200, None, br#"{"id":42,"login":"verifier-bot"}"#.to_vec())
                } else if target == format!("{repo}/branches/main") {
                    let main = if merged {
                        PROFILE_MERGE_SHA
                    } else {
                        PROFILE_BASE_SHA
                    };
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({"commit": {"id": main}})).unwrap(),
                    )
                } else if target == format!("{repo}/git/commits/{PROFILE_MERGE_SHA}") && merged {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({
                            "sha": PROFILE_MERGE_SHA,
                            "parents": [{"sha": PROFILE_BASE_SHA}, {"sha": PROFILE_HEAD_SHA}],
                        }))
                        .unwrap(),
                    )
                } else if target == format!("{repo}/git/commits/{PROFILE_HEAD_SHA}") && merged {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({"sha": PROFILE_HEAD_SHA})).unwrap(),
                    )
                } else if target == format!("{repo}/issues/23") {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({
                            "number": 23,
                            "state": if merged { "closed" } else { "open" },
                        }))
                        .unwrap(),
                    )
                } else if target
                    == format!(
                        "{repo}/git/trees/{PROFILE_MERGE_SHA}?page=1&per_page=1000&recursive=false"
                    )
                    && merged
                {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({
                            "sha": PROFILE_MERGE_SHA,
                            "tree": [{"path": "src", "mode": "100644", "type": "blob", "size": 1, "sha": "5555555555555555555555555555555555555555"}],
                            "truncated": false,
                            "page": 1,
                            "total_count": 1,
                        }))
                        .unwrap(),
                    )
                } else if target
                    == format!(
                        "{repo}/git/trees/{PROFILE_HEAD_SHA}?page=1&per_page=1000&recursive=false"
                    )
                    && merged
                {
                    (
                        200,
                        None,
                        serde_json::to_vec(&json!({
                            "sha": PROFILE_HEAD_SHA,
                            "tree": [{"path": "src", "mode": "100644", "type": "blob", "size": 1, "sha": "5555555555555555555555555555555555555555"}],
                            "truncated": false,
                            "page": 1,
                            "total_count": 1,
                        }))
                        .unwrap(),
                    )
                } else {
                    (404, None, br#"{"message":"not found"}"#.to_vec())
                };
                let total_count = total_count
                    .map(|count: u64| format!("X-Total-Count: {count}\r\n"))
                    .unwrap_or_default();
                write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\n{total_count}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (origin, done, server)
    }

    fn profile_gate_inputs_v1(directory: &Path) -> MergeAuthorityInputsV1 {
        let policy_bytes = serde_json::to_vec(&Policy {
            schema_version: 1,
            revision: 1,
            verifier_id: "independent-verifier".into(),
            environment: "merge-015 loopback fixture".into(),
            acceptance_command: vec!["cargo".into(), "test".into()],
            commands: vec![vec!["cargo".into(), "test".into()]],
            protected_criteria: vec!["requirements/**".into()],
            timeout_seconds: 60,
            merge_approval_policy: Some(agent_forge_core::forgejo::MergeApprovalPolicyV1 {
                schema_version: 1,
                required_approvals: 1,
                authorized_reviewer_ids: vec![42],
            }),
        })
        .unwrap();
        let expected = ExpectedBinding {
            requirement_sha256: "a".repeat(64),
            policy_sha256: sha256(&policy_bytes),
            candidate_commit: PROFILE_HEAD_SHA.into(),
            base_commit: PROFILE_BASE_SHA.into(),
            merge_commit: None,
            verifier_id: "independent-verifier".into(),
        };
        let proof = agent_forge_core::sign_proof(
            agent_forge_core::ProofEvidence {
                schema_version: 1,
                requirement_sha256: expected.requirement_sha256.clone(),
                policy_sha256: expected.policy_sha256.clone(),
                candidate_commit: expected.candidate_commit.clone(),
                base_commit: expected.base_commit.clone(),
                merge_commit: None,
                verifier_id: expected.verifier_id.clone(),
                environment: "merge-015 loopback fixture".into(),
                commands: vec![vec!["cargo".into(), "test".into()]],
                artifact_sha256: vec!["c".repeat(64)],
                outcome: agent_forge_core::Outcome::Pass,
                detail: "synthetic fixture proof".into(),
            },
            PROFILE_VERIFIER_KEY,
        )
        .unwrap();
        let binding = json!({
            "schema_version": 1,
            "review_id": 17,
            "reviewer_id": 42,
            "reviewer_login": "verifier-bot",
            "pull_request_number": 23,
            "base_branch": "main",
            "base_commit": PROFILE_BASE_SHA,
            "candidate_commit": PROFILE_HEAD_SHA,
            "source": "merge-015 loopback fixture",
        });
        MergeAuthorityInputsV1 {
            pull_request_number: 23,
            proof_path: write_private_fixture(
                directory,
                "proof.json",
                &serde_json::to_vec(&proof).unwrap(),
            ),
            expected_path: write_private_fixture(
                directory,
                "expected.json",
                &serde_json::to_vec(&expected).unwrap(),
            ),
            key_path: write_private_fixture(directory, "verifier.key", PROFILE_VERIFIER_KEY),
            policy_path: write_private_fixture(directory, "policy.json", &policy_bytes),
            review_binding_path: write_private_fixture(
                directory,
                "review-binding.json",
                &serde_json::to_vec(&binding).unwrap(),
            ),
            mission_admission_path: None,
            mission_journal_path: None,
            execution_identity: "fixture-agent".into(),
        }
    }

    fn write_mission_admission_fixture(
        directory: &Path,
        target: &ProofTargetV2,
    ) -> (PathBuf, PathBuf, String) {
        let requirement_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mission-012/requirements/MISSION-012-CLAMP-001.json");
        let requirements_json = fs::read_to_string(requirement_path)
            .expect("read the versioned MISSION-012 fixture requirements");
        let requirement_sha256 = sha256(requirements_json.as_bytes());
        let scope = BaseBranchScope::new(
            &target.origin,
            &target.owner,
            &target.repository,
            MERGE_PREFLIGHT_BASE_BRANCH,
        )
        .expect("build fixture mission scope");
        let journal_path = directory.join("mission-execution.jsonl");
        let journal = MissionExecutionJournalV1::open(&journal_path);
        let allocation = journal
            .allocate_generation(&scope, 1, &requirement_sha256, "profile-gate-admission")
            .expect("durably allocate the fixture mission generation");
        let identity = derive_mission_identity_v1(
            &scope,
            1,
            &requirement_sha256,
            allocation.execution_generation,
        )
        .expect("derive the fixture claim identity");
        let mut context = admission_context();
        context.manifest.source_commit = PROFILE_BASE_SHA.to_owned();
        context.manifest.requirement_sha256 = requirement_sha256.clone();
        context.manifest.sources[0].provenance = format!("git:{PROFILE_BASE_SHA}:src/lib.rs#L1");
        let request = AgentExecutionRequestV1 {
            schema_version: 1,
            mission_id: identity.mission_id,
            idempotency_key: identity.idempotency_key,
            requirement_sha256: requirement_sha256.clone(),
            requirements_json,
            untrusted_work_item: None,
            requirement_arbitration: None,
            context,
            workspace: WorkspaceRefV1 {
                id: "profile-gate-workspace".to_owned(),
                base_commit: PROFILE_BASE_SHA.to_owned(),
            },
            capabilities: admission_capabilities(),
        };
        journal
            .claim_allocated(
                &scope,
                1,
                allocation.execution_generation,
                &allocation.allocation_id,
                &request,
            )
            .expect("durably claim the fixture request");
        let admission = MissionHostAdmissionV1 {
            schema_version: 1,
            scope: MissionHostScopeV1 {
                origin: target.origin.clone(),
                owner: target.owner.clone(),
                repository: target.repository.clone(),
                branch: MERGE_PREFLIGHT_BASE_BRANCH.to_owned(),
            },
            issue_number: 1,
            base_commit: PROFILE_BASE_SHA.to_owned(),
            requirement_path: "requirements/MISSION-012-CLAMP-001.json".to_owned(),
            requirement_sha256,
            allocation: MissionHostAllocationV1 {
                schema_version: allocation.schema_version,
                mission_id: allocation.mission_id,
                allocation_id: allocation.allocation_id,
                requirement_sha256: allocation.requirement_sha256,
                execution_generation: allocation.execution_generation,
            },
            request,
        };
        admission
            .validate()
            .expect("fixture admission satisfies its versioned host contract");
        let bytes = serde_json::to_vec(&admission).unwrap();
        let digest = sha256(&bytes);
        let admission_path = write_private_fixture(directory, "host-admission.json", &bytes);
        (admission_path, journal_path, digest)
    }

    fn profile_gate_inputs_v2(directory: &Path, target: ProofTargetV2) -> MergeAuthorityInputsV1 {
        let mut inputs = profile_gate_inputs_v1(directory);
        let mission_admission = (target.profile_selector
            == LocalForgejoTargetProfile::Mission012Clamp.selector())
        .then(|| write_mission_admission_fixture(directory, &target));
        let mut legacy: SignedProof =
            serde_json::from_slice(&fs::read(&inputs.proof_path).unwrap())
                .expect("V1 fixture helper produces a V1 proof");
        if let Some((admission_path, _, _)) = mission_admission.as_ref() {
            let admission: MissionHostAdmissionV1 =
                serde_json::from_slice(&fs::read(admission_path).unwrap()).unwrap();
            let mut expected: ExpectedBinding =
                serde_json::from_slice(&fs::read(&inputs.expected_path).unwrap()).unwrap();
            expected.requirement_sha256 = admission.requirement_sha256.clone();
            legacy.evidence.requirement_sha256 = admission.requirement_sha256;
            fs::write(
                &inputs.expected_path,
                serde_json::to_vec(&expected).unwrap(),
            )
            .unwrap();
        }
        let proof = agent_forge_core::sign_target_bound_proof_v2(
            legacy.evidence,
            target,
            PROFILE_VERIFIER_KEY,
        )
        .unwrap();
        fs::write(&inputs.proof_path, serde_json::to_vec(&proof).unwrap()).unwrap();
        if let Some((admission_path, journal_path, admission_sha256)) = mission_admission {
            inputs.mission_admission_path = Some(admission_path);
            inputs.mission_journal_path = Some(journal_path);
            let mut binding: Value =
                serde_json::from_slice(&fs::read(&inputs.review_binding_path).unwrap()).unwrap();
            binding["mission_admission_sha256"] = json!(admission_sha256);
            binding["mission_source_issue_number"] = json!(1);
            fs::write(
                &inputs.review_binding_path,
                serde_json::to_vec(&binding).unwrap(),
            )
            .unwrap();
        }
        inputs
    }

    fn mission_profile_target() -> ProofTargetV2 {
        let profile = LocalForgejoTargetProfile::Mission012Clamp;
        ProofTargetV2::new(
            profile.selector(),
            profile.origin(),
            profile.owner(),
            profile.repository(),
            MERGE_PREFLIGHT_BASE_BRANCH,
        )
        .unwrap()
    }

    fn verify_fixture_mission_admission(inputs: &MergeAuthorityInputsV1) -> Result<Value, Value> {
        let expected: ExpectedBinding =
            serde_json::from_slice(&fs::read(&inputs.expected_path).unwrap()).unwrap();
        let binding: HostReviewBaseBindingFileV1 =
            serde_json::from_slice(&fs::read(&inputs.review_binding_path).unwrap()).unwrap();
        verify_mission_admission_binding(
            LocalForgejoTargetProfile::Mission012Clamp,
            MISSION_012_FORGEJO_ORIGIN,
            MISSION_012_REPOSITORY_OWNER,
            MISSION_012_REPOSITORY_NAME,
            &expected,
            &binding,
            inputs,
        )
    }

    #[test]
    fn mission_gate_verifies_the_admission_digest_and_exact_durable_claim() {
        let scratch = ScratchJournalDir::new();
        let target = mission_profile_target();
        let inputs = profile_gate_inputs_v2(&scratch.0, target);

        let verified = verify_fixture_mission_admission(&inputs)
            .expect("host admission and exact durable claim should verify");
        let admission: MissionHostAdmissionV1 = serde_json::from_slice(
            &fs::read(inputs.mission_admission_path.as_ref().unwrap()).unwrap(),
        )
        .unwrap();

        assert_eq!(verified["issue_number"], 1);
        assert_eq!(verified["base_commit"], PROFILE_BASE_SHA);
        assert_eq!(verified["requirement_sha256"], admission.requirement_sha256);
        assert_eq!(verified["execution_generation"], 1);
        assert_eq!(
            verified["admission_sha256"],
            sha256(&fs::read(inputs.mission_admission_path.as_ref().unwrap()).unwrap())
        );
    }

    #[test]
    fn mission_gate_rejects_an_admission_digest_not_bound_to_the_host_review() {
        let scratch = ScratchJournalDir::new();
        let target = mission_profile_target();
        let inputs = profile_gate_inputs_v2(&scratch.0, target);
        let mut binding: Value =
            serde_json::from_slice(&fs::read(&inputs.review_binding_path).unwrap()).unwrap();
        binding["mission_admission_sha256"] = json!("0".repeat(64));
        fs::write(
            &inputs.review_binding_path,
            serde_json::to_vec(&binding).unwrap(),
        )
        .unwrap();

        let error = verify_fixture_mission_admission(&inputs)
            .expect_err("the review binding must refer to the exact admission bytes");

        assert_eq!(error["code"], "mission_admission_binding_mismatch");
    }

    #[test]
    fn mission_gate_rejects_a_base_that_differs_from_the_admitted_snapshot() {
        let scratch = ScratchJournalDir::new();
        let target = mission_profile_target();
        let inputs = profile_gate_inputs_v2(&scratch.0, target);
        let binding: HostReviewBaseBindingFileV1 =
            serde_json::from_slice(&fs::read(&inputs.review_binding_path).unwrap()).unwrap();
        let mut expected: ExpectedBinding =
            serde_json::from_slice(&fs::read(&inputs.expected_path).unwrap()).unwrap();
        expected.base_commit = "d".repeat(40);

        let error = verify_mission_admission_binding(
            LocalForgejoTargetProfile::Mission012Clamp,
            MISSION_012_FORGEJO_ORIGIN,
            MISSION_012_REPOSITORY_OWNER,
            MISSION_012_REPOSITORY_NAME,
            &expected,
            &binding,
            &inputs,
        )
        .expect_err("the proof base and admitted workspace base must be identical");

        assert_eq!(error["code"], "mission_admission_binding_mismatch");
    }

    fn merge_authority_branch_protection_fixture() -> Value {
        json!([{
            "rule_name": "main",
            "branch_name": "main",
            "required_approvals": 1,
            "enable_approvals_whitelist": true,
            "approvals_whitelist_username": ["verifier-bot"],
            "apply_to_admins": true,
            "dismiss_stale_approvals": true,
            "block_on_outdated_branch": true,
            "enable_merge_whitelist": true,
            "merge_whitelist_usernames": ["forge-admin"],
            "merge_whitelist_teams": [],
            "enable_push": false,
            "unprotected_file_patterns": "",
        }])
    }

    fn merge_authority_branch_protection_policy() -> Policy {
        Policy {
            schema_version: 1,
            revision: 1,
            verifier_id: "independent-verifier".into(),
            environment: "test fixture".into(),
            acceptance_command: vec!["cargo".into(), "test".into()],
            commands: vec![vec!["cargo".into(), "test".into()]],
            protected_criteria: vec!["requirements/**".into()],
            timeout_seconds: 60,
            merge_approval_policy: Some(agent_forge_core::forgejo::MergeApprovalPolicyV1 {
                schema_version: 1,
                required_approvals: 1,
                authorized_reviewer_ids: vec![42],
            }),
        }
    }

    #[test]
    fn local_merge_gate_refuses_additional_merge_whitelist_users() {
        let policy = merge_authority_branch_protection_policy();
        let reviewer_logins = vec!["verifier-bot".to_owned()];
        let mut protection = merge_authority_branch_protection_fixture();
        protection[0]["merge_whitelist_usernames"] = json!(["forge-admin", "second-merger"]);

        let result = validate_local_test_branch_protection(
            &protection,
            &policy,
            &reviewer_logins,
            "forge-admin",
        );

        assert_eq!(
            result,
            Err("branch_protection_does_not_match_local_gate_policy")
        );
    }

    #[test]
    fn local_merge_gate_refuses_merge_whitelist_teams() {
        let policy = merge_authority_branch_protection_policy();
        let reviewer_logins = vec!["verifier-bot".to_owned()];
        let mut protection = merge_authority_branch_protection_fixture();
        protection[0]["merge_whitelist_teams"] = json!(["release-managers"]);

        let result = validate_local_test_branch_protection(
            &protection,
            &policy,
            &reviewer_logins,
            "forge-admin",
        );

        assert_eq!(
            result,
            Err("branch_protection_does_not_match_local_gate_policy")
        );
    }

    #[test]
    fn local_merge_gate_refuses_branch_protection_without_team_whitelist_readback() {
        let policy = merge_authority_branch_protection_policy();
        let reviewer_logins = vec!["verifier-bot".to_owned()];
        let mut protection = merge_authority_branch_protection_fixture();
        protection[0]
            .as_object_mut()
            .unwrap()
            .remove("merge_whitelist_teams");

        let result = validate_local_test_branch_protection(
            &protection,
            &policy,
            &reviewer_logins,
            "forge-admin",
        );

        assert_eq!(result, Err("invalid_merge_whitelist_teams"));
    }

    #[test]
    fn local_merge_gate_refuses_unprotected_file_patterns() {
        let policy = merge_authority_branch_protection_policy();
        let reviewer_logins = vec!["verifier-bot".to_owned()];
        let mut protection = merge_authority_branch_protection_fixture();
        protection[0]["unprotected_file_patterns"] = json!("requirements/**");

        let result = validate_local_test_branch_protection(
            &protection,
            &policy,
            &reviewer_logins,
            "forge-admin",
        );

        assert_eq!(
            result,
            Err("branch_protection_does_not_match_local_gate_policy")
        );
    }

    #[test]
    fn local_merge_gate_requires_unprotected_file_patterns_readback() {
        let policy = merge_authority_branch_protection_policy();
        let reviewer_logins = vec!["verifier-bot".to_owned()];
        let mut protection = merge_authority_branch_protection_fixture();
        protection[0]
            .as_object_mut()
            .unwrap()
            .remove("unprotected_file_patterns");

        let result = validate_local_test_branch_protection(
            &protection,
            &policy,
            &reviewer_logins,
            "forge-admin",
        );

        assert_eq!(
            result,
            Err("branch_protection_does_not_match_local_gate_policy")
        );
    }

    #[test]
    fn local_merge_gate_stops_before_merge_post_when_another_user_is_whitelisted() {
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let mut protection = merge_authority_branch_protection_fixture();
        protection[0]["merge_whitelist_usernames"] = json!(["forge-admin", "second-merger"]);

        let ((result, merge_authorized), requests, _) =
            run_profile_gate_against_fixture_with_protection(profile, protection);
        let request_lines = requests
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>();

        assert!(!merge_authorized);
        assert_eq!(result["verdict"], "rejected");
        assert_eq!(
            result["error"]["code"],
            "branch_protection_does_not_match_local_gate_policy"
        );
        assert!(!result["merge_attempted"].as_bool().unwrap());
        assert!(request_lines.iter().any(|line| {
            line.starts_with("GET /api/v1/repos/forge-admin/agent-forge-core/branch_protections ")
        }));
        assert!(
            !request_lines.iter().any(|line| line
                .starts_with("POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge ")),
            "a non-exclusive merge whitelist must stop before the merge POST"
        );
    }

    #[test]
    fn local_merge_gate_rechecks_branch_protection_immediately_before_merge_post() {
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let initial_protection = merge_authority_branch_protection_fixture();
        let mut changed_protection = merge_authority_branch_protection_fixture();
        changed_protection[0]["merge_whitelist_usernames"] =
            json!(["forge-admin", "second-merger"]);

        let ((result, merge_authorized), requests, _) =
            run_profile_gate_against_fixture_with_protection_change(
                profile,
                initial_protection,
                changed_protection,
            );
        let request_lines = requests
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>();

        assert!(!merge_authorized);
        assert_eq!(result["verdict"], "rejected");
        assert_eq!(
            result["error"]["code"],
            "branch_protection_changed_during_gate"
        );
        assert!(!result["merge_attempted"].as_bool().unwrap());
        assert_eq!(
            request_lines
                .iter()
                .filter(|line| line.starts_with(
                    "GET /api/v1/repos/forge-admin/agent-forge-core/branch_protections "
                ))
                .count(),
            2,
            "the gate must read branch protection again in the final window"
        );
        assert!(
            !request_lines.iter().any(|line| line
                .starts_with("POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge ")),
            "a changed branch protection must stop before the merge POST"
        );
    }

    #[test]
    fn local_merge_gate_reports_invalid_final_branch_protection_as_inconclusive() {
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let initial_protection = merge_authority_branch_protection_fixture();

        let ((result, merge_authorized), requests, _) =
            run_profile_gate_against_fixture_with_protection_change(
                profile,
                initial_protection,
                json!({"unexpected": "object instead of protection array"}),
            );
        let request_lines = requests
            .iter()
            .map(|(line, _)| line.as_str())
            .collect::<Vec<_>>();

        assert!(!merge_authorized);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["error"]["code"], "upstream_invalid_response");
        assert!(!result["merge_attempted"].as_bool().unwrap());
        assert!(
            !request_lines.iter().any(|line| line
                .starts_with("POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge ")),
            "an invalid policy response must stop before the merge POST"
        );
    }

    #[test]
    fn merge_gate_refuses_a_mission_review_binding_for_another_pull_request_before_post() {
        const MERGE_TOKEN: &str = "synthetic-merge-actor-token";
        const POLICY_TOKEN: &str = "synthetic-policy-reader-token";
        let profile = LocalForgejoTargetProfile::Mission012Clamp;
        let (origin, done, server) = start_profile_gate_fixture_with_role_credentials(
            profile.owner(),
            profile.repository(),
            merge_authority_branch_protection_fixture(),
            None,
            MERGE_TOKEN,
            POLICY_TOKEN,
            "forge-admin",
            vec![1],
        );
        let scratch = ScratchJournalDir::new();
        let target = ProofTargetV2::new(
            profile.selector(),
            origin.clone(),
            profile.owner(),
            profile.repository(),
            MERGE_PREFLIGHT_BASE_BRANCH,
        )
        .unwrap();
        let inputs = profile_gate_inputs_v2(&scratch.0, target);
        let mut review_binding: Value =
            serde_json::from_slice(&fs::read(&inputs.review_binding_path).unwrap()).unwrap();
        review_binding["pull_request_number"] = json!(24);
        fs::write(
            &inputs.review_binding_path,
            serde_json::to_vec(&review_binding).unwrap(),
        )
        .unwrap();

        let mut api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(MERGE_TOKEN)),
        );
        api.request_policy = LocalApiRequestPolicy::MergeAuthority;
        let mut policy_api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, api.config.origin.clone()),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(POLICY_TOKEN)),
        );
        policy_api.request_policy = LocalApiRequestPolicy::MergePolicyReadOnly;

        let (result, success) = run_merge_authority_for_apis(
            profile,
            &api,
            &policy_api,
            &inputs,
            &scratch.0.join("merge-locks"),
        )
        .unwrap();
        done.store(true, Ordering::SeqCst);
        let requests = server.join().unwrap();
        assert!(
            !success,
            "a review binding for PR 24 cannot authorize the admitted mission's PR 23"
        );
        assert_eq!(result["pull_request_number"], 23);
        assert_eq!(result["verdict"], "rejected");
        assert_eq!(result["error"]["code"], "merge_attempt_binding_mismatch");
        assert_eq!(
            result["error"]["action"],
            "restore the exact host inputs bound to the durable merge attempt or admit a new generation"
        );
        assert_eq!(result["error"]["details"]["pull_request_number"], 23);
        assert_eq!(result["merge_attempted"], false);
        assert!(
            requests.is_empty(),
            "the mismatched mission-to-PR binding must be refused before any Forgejo read or merge POST"
        );
    }

    #[test]
    fn mission_gate_uses_separate_credentials_and_posts_only_for_the_admitted_pull_request() {
        const MERGE_TOKEN: &str = "synthetic-mission-merge-token";
        const POLICY_TOKEN: &str = "synthetic-mission-policy-reader-token";
        let profile = LocalForgejoTargetProfile::Mission012Clamp;
        let protection = merge_authority_branch_protection_fixture();
        let ((result, success), requests, origin) =
            run_profile_gate_against_fixture_with_role_credentials(
                profile,
                protection.clone(),
                protection,
                MERGE_TOKEN,
                POLICY_TOKEN,
                "forge-admin",
            );
        let request_lines = requests
            .iter()
            .map(|(line, _)| line.clone())
            .collect::<Vec<_>>();
        let merge_path = format!(
            "POST /api/v1/repos/{}/{}/pulls/23/merge ",
            profile.owner(),
            profile.repository()
        );

        assert!(
            success,
            "valid admitted mission should reach its merge: {result}"
        );
        assert_eq!(result["verdict"], "merged");
        assert_eq!(result["pull_request_number"], 23);
        assert_eq!(result["merge_commit_sha"], PROFILE_MERGE_SHA);
        assert_eq!(
            result["target"],
            json!({
                "selector": profile.selector(),
                "origin": origin,
                "owner": profile.owner(),
                "repository": profile.repository(),
                "base_branch": MERGE_PREFLIGHT_BASE_BRANCH,
            })
        );
        assert_eq!(request_lines, expected_profile_gate_requests(profile));
        assert_eq!(
            requests
                .iter()
                .filter(|(line, _)| line.starts_with(&merge_path))
                .count(),
            1,
            "the gate may issue exactly one merge POST for admitted PR 23"
        );
        for (line, token) in &requests {
            let path = line.split(' ').nth(1).unwrap();
            let expected_token = if path.ends_with("/branch_protections") {
                POLICY_TOKEN
            } else {
                MERGE_TOKEN
            };
            assert_eq!(token, expected_token, "wrong credential on {line}");
        }
        let serialized_result = serde_json::to_string(&result).unwrap();
        assert!(!serialized_result.contains(MERGE_TOKEN));
        assert!(!serialized_result.contains(POLICY_TOKEN));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_supervisor_capability_reaches_fixture_merge_and_is_bound_in_audit_output() {
        let profile = LocalForgejoTargetProfile::Mission012Clamp;
        let ((result, success), requests, origin) =
            run_profile_gate_against_fixture_with_supervisor_capability(profile);
        let merge_path = format!(
            "POST /api/v1/repos/{}/{}/pulls/23/merge ",
            profile.owner(),
            profile.repository()
        );

        assert!(
            success,
            "live fixture capability should admit this scoped test: {result}"
        );
        assert_eq!(result["verdict"], "merged");
        assert_eq!(result["pull_request_number"], 23);
        assert_eq!(result["writer_admission"]["target"]["origin"], origin);
        assert_eq!(
            result["writer_admission"]["target"]["profile_selector"],
            profile.selector()
        );
        assert_eq!(result["writer_admission"]["pre_post_revalidated"], true);
        assert_eq!(
            result["writer_admission"]["supervisor_policy_independently_verified"],
            false
        );
        assert_eq!(
            result["external_writer_exclusion"],
            "trusted_supervisor_policy_not_independently_verified"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|(line, _)| line.starts_with(&merge_path))
                .count(),
            1,
            "fixture gate must issue exactly one POST after the pre-POST capability check"
        );
    }

    #[test]
    fn v17_multiwriter_profile_is_refused_before_lock_inputs_or_forgejo_reads() {
        let scratch = ScratchJournalDir::new();
        let profile = LocalForgejoTargetProfile::Mission012Clamp;
        let origin = "http://127.0.0.1:1".to_owned();
        let config = LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin.clone());
        let api = LocalLoopbackForgejoApi::new(config);
        let policy_api = LocalLoopbackForgejoApi::new(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin),
        );
        let inputs = MergeAuthorityInputsV1 {
            pull_request_number: 23,
            proof_path: scratch.0.join("must-not-read-proof.json"),
            expected_path: scratch.0.join("must-not-read-expected.json"),
            key_path: scratch.0.join("must-not-read-key"),
            policy_path: scratch.0.join("must-not-read-policy.json"),
            review_binding_path: scratch.0.join("must-not-read-review-binding.json"),
            mission_admission_path: None,
            mission_journal_path: None,
            execution_identity: "fixture-gate".into(),
        };
        let lock_root = scratch.0.join("must-not-create-merge-locks");

        let (result, success) = run_merge_authority_for_apis_with_writer_profile(
            profile,
            &api,
            &policy_api,
            &inputs,
            &lock_root,
            MergeWriterAdmissionProfileV1::ForgejoV17MultiWriter,
            false,
            None,
        )
        .expect("profile admission returns a structured refusal");

        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["merge_attempted"], false);
        assert_eq!(
            result["error"]["code"],
            "server_expected_base_precondition_unavailable"
        );
        assert_eq!(
            result["error"]["details"]["server_expected_base_precondition"],
            "not_mandatory_or_unavailable"
        );
        assert_eq!(result["target"]["selector"], profile.selector());
        assert!(
            !lock_root.exists(),
            "the multiwriter refusal must precede lock acquisition and host input reads"
        );
    }

    #[test]
    fn merge_cli_defaults_to_refusing_multiwriter_before_reading_merge_credential() {
        let arguments = [
            "--target-profile",
            "merge-014-agent-forge-core",
            "23",
            "missing-proof.json",
            "missing-expected.json",
            "missing-key",
            "missing-policy.json",
            "missing-review-binding.json",
            "fixture-gate",
            "999999",
        ]
        .into_iter()
        .map(std::ffi::OsString::from);

        let (result, success) = run_local_test_merge_authority(arguments)
            .expect("missing writer admission returns a structured safe refusal");

        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(
            result["error"]["code"],
            "server_expected_base_precondition_unavailable"
        );
        assert_eq!(result["merge_attempted"], false);
    }

    #[test]
    fn merge_cli_refuses_single_writer_profile_without_supervisor_capability_before_credential() {
        let arguments = [
            "--writer-admission-profile",
            "local_single_writer_sandbox",
            "--target-profile",
            "merge-014-agent-forge-core",
            "23",
            "missing-proof.json",
            "missing-expected.json",
            "missing-key",
            "missing-policy.json",
            "missing-review-binding.json",
            "fixture-gate",
            "999999",
        ]
        .into_iter()
        .map(std::ffi::OsString::from);

        let (result, success) = run_local_test_merge_authority(arguments)
            .expect("missing host attestation returns a structured safe refusal");

        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["merge_attempted"], false);
        assert_eq!(result["error"]["code"], "supervisor_capability_missing");
        assert_eq!(result["error"]["details"]["stage"], "writer_admission");
    }

    #[test]
    fn merge_cli_refuses_invalid_supervisor_descriptor_before_reading_credential() {
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let (read_fd, write_fd) = (descriptors[0], descriptors[1]);
        let token = b"synthetic-credential-must-remain-unread";
        assert_eq!(
            unsafe { libc::write(write_fd, token.as_ptr().cast(), token.len()) },
            token.len() as isize
        );
        unsafe { libc::close(write_fd) };
        let credential_fd = read_fd.to_string();
        let arguments = [
            MERGE_WRITER_ADMISSION_PROFILE_FLAG,
            "local_single_writer_sandbox",
            SUPERVISOR_CAPABILITY_FD_FLAG,
            "999999",
            "--target-profile",
            "merge-014-agent-forge-core",
            "23",
            "missing-proof.json",
            "missing-expected.json",
            "missing-key",
            "missing-policy.json",
            "missing-review-binding.json",
            "fixture-gate",
            credential_fd.as_str(),
        ]
        .into_iter()
        .map(std::ffi::OsString::from);

        let (result, success) = run_local_test_merge_authority(arguments)
            .expect("invalid capability descriptor is a structured safe refusal");
        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(
            result["error"]["code"],
            "supervisor_capability_invalid_descriptor"
        );

        let mut remaining = [0_u8; 64];
        let count = unsafe { libc::read(read_fd, remaining.as_mut_ptr().cast(), remaining.len()) };
        unsafe { libc::close(read_fd) };
        assert_eq!(&remaining[..count as usize], token);
    }

    fn run_profile_gate_against_fixture(
        profile: LocalForgejoTargetProfile,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        run_profile_gate_against_fixture_with_protection(
            profile,
            merge_authority_branch_protection_fixture(),
        )
    }

    fn run_profile_gate_against_fixture_with_protection(
        profile: LocalForgejoTargetProfile,
        branch_protection: Value,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        run_profile_gate_against_fixture_with_protection_change(
            profile,
            branch_protection,
            merge_authority_branch_protection_fixture(),
        )
    }

    fn run_profile_gate_against_fixture_with_protection_change(
        profile: LocalForgejoTargetProfile,
        branch_protection: Value,
        final_branch_protection: Value,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        run_profile_gate_against_fixture_with_role_credentials(
            profile,
            branch_protection,
            final_branch_protection,
            PROFILE_FIXTURE_TOKEN,
            PROFILE_FIXTURE_TOKEN,
            "forge-admin",
        )
    }

    fn run_profile_gate_against_fixture_with_role_credentials(
        profile: LocalForgejoTargetProfile,
        branch_protection: Value,
        final_branch_protection: Value,
        merge_token: &'static str,
        policy_token: &'static str,
        merge_actor_login: &'static str,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        run_profile_gate_against_fixture_with_role_credentials_and_review_page_totals(
            profile,
            branch_protection,
            final_branch_protection,
            merge_token,
            policy_token,
            merge_actor_login,
            vec![1],
        )
    }

    fn run_profile_gate_against_fixture_with_review_page_totals(
        profile: LocalForgejoTargetProfile,
        review_page_totals: Vec<u64>,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        run_profile_gate_against_fixture_with_role_credentials_and_review_page_totals(
            profile,
            merge_authority_branch_protection_fixture(),
            merge_authority_branch_protection_fixture(),
            PROFILE_FIXTURE_TOKEN,
            PROFILE_FIXTURE_TOKEN,
            "forge-admin",
            review_page_totals,
        )
    }

    fn run_profile_gate_against_fixture_with_role_credentials_and_review_page_totals(
        profile: LocalForgejoTargetProfile,
        branch_protection: Value,
        final_branch_protection: Value,
        merge_token: &'static str,
        policy_token: &'static str,
        merge_actor_login: &'static str,
        review_page_totals: Vec<u64>,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        run_profile_gate_fixture_internal(
            profile,
            branch_protection,
            final_branch_protection,
            merge_token,
            policy_token,
            merge_actor_login,
            review_page_totals,
            false,
        )
    }

    fn run_profile_gate_against_fixture_with_supervisor_capability(
        profile: LocalForgejoTargetProfile,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        let protection = merge_authority_branch_protection_fixture();
        run_profile_gate_fixture_internal(
            profile,
            protection.clone(),
            protection,
            PROFILE_FIXTURE_TOKEN,
            PROFILE_FIXTURE_TOKEN,
            "forge-admin",
            vec![1],
            true,
        )
    }

    #[cfg(target_os = "linux")]
    fn test_supervisor_capability(
        profile: LocalForgejoTargetProfile,
        origin: &str,
    ) -> (VerifiedSupervisorMergeCapabilityV1, std::os::fd::OwnedFd) {
        use std::os::fd::OwnedFd;

        let mut descriptors = [-1; 2];
        assert_eq!(
            unsafe {
                libc::socketpair(
                    libc::AF_UNIX,
                    libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                    0,
                    descriptors.as_mut_ptr(),
                )
            },
            0
        );
        let gate_socket = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let supervisor_endpoint = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
        let process = agent_forge_core::supervisor_capability::ProcessBindingV1::current()
            .expect("test process identity");
        let nonce_input = format!(
            "{}:{}:{}",
            process.pid,
            process.start_time_ticks,
            agent_forge_core::supervisor_capability::monotonic_time_nanoseconds().unwrap()
        );
        let envelope =
            agent_forge_core::supervisor_capability::SupervisorMergeCapabilityEnvelopeV1 {
                schema_version: 1,
                target: MergeCapabilityTargetV1 {
                    profile_selector: profile.selector().to_owned(),
                    origin: origin.to_owned(),
                    owner: profile.owner().to_owned(),
                    repository: profile.repository().to_owned(),
                    pull_request_number: 23,
                    base_branch: MERGE_PREFLIGHT_BASE_BRANCH.to_owned(),
                },
                supervisor: process.clone(),
                supervisor_uid: unsafe { libc::geteuid() },
                gate: process.clone(),
                forgejo: process,
                expires_at_monotonic_ns:
                    agent_forge_core::supervisor_capability::monotonic_time_nanoseconds().unwrap()
                        + 60_000_000_000,
                nonce: agent_forge_core::sha256(nonce_input.as_bytes()),
            };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let sent = unsafe {
            libc::send(
                supervisor_endpoint.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        assert_eq!(sent, bytes.len() as isize);
        let capability = VerifiedSupervisorMergeCapabilityV1::receive(gate_socket, envelope.target)
            .expect("test supervisor capability should bind to the live gate fixture");
        (capability, supervisor_endpoint)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_profile_gate_fixture_internal(
        profile: LocalForgejoTargetProfile,
        branch_protection: Value,
        final_branch_protection: Value,
        merge_token: &'static str,
        policy_token: &'static str,
        merge_actor_login: &'static str,
        review_page_totals: Vec<u64>,
        attach_supervisor_capability: bool,
    ) -> ((Value, bool), Vec<(String, String)>, String) {
        let (origin, done, server) = start_profile_gate_fixture_with_role_credentials(
            profile.owner(),
            profile.repository(),
            branch_protection,
            Some(final_branch_protection),
            merge_token,
            policy_token,
            merge_actor_login,
            review_page_totals,
        );
        let scratch = ScratchJournalDir::new();
        let target = ProofTargetV2::new(
            profile.selector(),
            origin.clone(),
            profile.owner(),
            profile.repository(),
            MERGE_PREFLIGHT_BASE_BRANCH,
        )
        .unwrap();
        let inputs = profile_gate_inputs_v2(&scratch.0, target);
        let mut api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin.clone()),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(merge_token)),
        );
        api.request_policy = LocalApiRequestPolicy::MergeAuthority;
        let mut policy_api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin.clone()),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(policy_token)),
        );
        policy_api.request_policy = LocalApiRequestPolicy::MergePolicyReadOnly;
        let (supervisor_capability, supervisor_endpoint) = if attach_supervisor_capability {
            let (capability, endpoint) = test_supervisor_capability(profile, &origin);
            (Some(capability), Some(endpoint))
        } else {
            (None, None)
        };
        // Private lock root: never the shared /tmp production lock directory.
        let result = if let Some(capability) = supervisor_capability.as_ref() {
            run_merge_authority_for_apis_with_writer_profile(
                profile,
                &api,
                &policy_api,
                &inputs,
                &scratch.0.join("merge-locks"),
                MergeWriterAdmissionProfileV1::LocalSingleWriterSandbox,
                false,
                Some(capability),
            )
        } else {
            run_merge_authority_for_apis(
                profile,
                &api,
                &policy_api,
                &inputs,
                &scratch.0.join("merge-locks"),
            )
        };
        drop(supervisor_endpoint);
        done.store(true, Ordering::SeqCst);
        let requests = server.join().unwrap();
        (result.unwrap(), requests, origin)
    }

    fn expected_profile_gate_requests(profile: LocalForgejoTargetProfile) -> Vec<String> {
        let owner = profile.owner();
        let repository = profile.repository();
        let repo = format!("/api/v1/repos/{owner}/{repository}");
        let mut lines = vec![
            format!("GET {repo}/pulls/23"),
            "GET /api/v1/user".to_owned(),
            format!("GET {repo}/pulls/23/reviews?limit=50&page=1"),
            format!("GET {repo}/branch_protections"),
            "GET /api/v1/users/verifier-bot".to_owned(),
            format!("GET {repo}/branches/main"),
            format!("GET {repo}/pulls/23"),
            format!("GET {repo}/branches/main"),
            format!("GET {repo}/branch_protections"),
            format!("POST {repo}/pulls/23/merge"),
            format!("GET {repo}/pulls/23"),
            format!("GET {repo}/git/commits/{PROFILE_MERGE_SHA}"),
            format!("GET {repo}/git/commits/{PROFILE_HEAD_SHA}"),
            format!("GET {repo}/branches/main"),
            format!("GET {repo}/issues/23"),
            format!(
                "GET {repo}/git/trees/{PROFILE_MERGE_SHA}?page=1&per_page=1000&recursive=false"
            ),
            format!("GET {repo}/git/trees/{PROFILE_HEAD_SHA}?page=1&per_page=1000&recursive=false"),
        ];
        if profile == LocalForgejoTargetProfile::Mission012Clamp {
            // The durable mission intent is written only after an additional
            // current-state validation. The legacy nonmission gate keeps its
            // original HTTP sequence.
            let post = lines
                .iter()
                .position(|line| line.ends_with("/pulls/23/merge"))
                .expect("expected one merge POST");
            lines.splice(
                post..post,
                [
                    format!("GET {repo}/pulls/23"),
                    format!("GET {repo}/branches/main"),
                    format!("GET {repo}/branch_protections"),
                ],
            );
        }
        lines
            .into_iter()
            .map(|line| format!("{line} HTTP/1.1"))
            .collect()
    }

    fn assert_profile_gate_scope(profile: LocalForgejoTargetProfile) {
        let ((result, success), requests, origin) = run_profile_gate_against_fixture(profile);
        let lines = requests
            .iter()
            .map(|(line, _)| line.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            expected_profile_gate_requests(profile),
            "{profile:?}: result {result}"
        );
        let selected = format!(
            "/api/v1/repos/{}/{}/",
            profile.owner(),
            profile.repository()
        );
        for (line, authorized) in &requests {
            let target = line.split(' ').nth(1).unwrap();
            assert!(
                !target.starts_with("/api/v1/repos/") || target.starts_with(&selected),
                "{profile:?}: request left the selected repository: {line}"
            );
            for other in LocalForgejoTargetProfile::ALL
                .into_iter()
                .filter(|other| *other != profile)
            {
                assert!(
                    !target.contains(&format!("/{}/", other.repository())),
                    "{profile:?}: request mixed in {other:?}: {line}"
                );
            }
            assert_eq!(
                authorized, PROFILE_FIXTURE_TOKEN,
                "{profile:?}: origin-bound credential missing: {line}"
            );
        }
        assert!(success, "{profile:?}: {result}");
        assert_eq!(result["verdict"], "merged");
        assert_eq!(result["merge_commit_sha"], PROFILE_MERGE_SHA);
        assert_eq!(result["validated_base_sha"], PROFILE_BASE_SHA);
        assert_eq!(result["submitted_head_sha"], PROFILE_HEAD_SHA);
        assert_eq!(
            result["target"],
            json!({
                "selector": profile.selector(),
                "origin": origin,
                "owner": profile.owner(),
                "repository": profile.repository(),
                "base_branch": "main",
            }),
            "{profile:?}: merge result must name the target it merged"
        );
    }

    #[test]
    fn merge_authority_refuses_changed_review_total_before_any_merge_post() {
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let ((result, success), requests, _) =
            run_profile_gate_against_fixture_with_review_page_totals(profile, vec![51, 52]);
        let request_lines = requests
            .iter()
            .map(|(line, _)| line.clone())
            .collect::<Vec<_>>();

        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_attempted"], false);
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["error"]["stage"], "review-list-read");
        assert_eq!(result["error"]["code"], "review_pagination_inconsistent");
        assert_eq!(result["error"]["retryable"], true);
        assert_eq!(
            result["error"]["action"],
            "discard all pages and reread the complete review list before retrying the merge preflight"
        );
        assert_eq!(
            request_lines,
            vec![
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23 HTTP/1.1",
                "GET /api/v1/user HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23/reviews?limit=50&page=1 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23/reviews?limit=50&page=2 HTTP/1.1",
            ]
        );
        assert!(
            request_lines
                .iter()
                .all(|line| !line.contains("/pulls/23/merge")),
            "inconsistent review pagination must stop before merge POST"
        );
    }

    #[test]
    fn forgejo_review_page_adapter_reports_changed_total_as_retryable_inconclusive() {
        let review_json = |id: u64| {
            json!({
                "id": id,
                "user": {"id": 42},
                "state": "APPROVED",
                "commit_id": PROFILE_HEAD_SHA,
                "pull_request_url": "http://fixture/forge-admin/agent-forge-core/pulls/23",
                "stale": false,
                "official": true,
                "dismissed": false,
            })
        };
        let first_page = serde_json::to_vec(&(1..=50).map(review_json).collect::<Vec<_>>())
            .expect("serialize the first page of reviews");
        let second_page = serde_json::to_vec(&(51..=52).map(review_json).collect::<Vec<_>>())
            .expect("serialize the changed second page of reviews");
        let (origin, server, requests) =
            start_http_fixture_with_extra_headers_recording_requests(vec![
                (
                    200,
                    Duration::ZERO,
                    first_page,
                    vec!["X-Total-Count: 51".to_owned()],
                ),
                (
                    200,
                    Duration::ZERO,
                    second_page,
                    vec!["X-Total-Count: 52".to_owned()],
                ),
            ]);
        let api = api_for_origin(origin.clone());
        let scope = PullRequestReviewScopeV1::new(
            &origin,
            LOCAL_REPOSITORY_OWNER,
            LOCAL_REPOSITORY_NAME,
            23,
        )
        .expect("fixture scope is valid");

        let error = forgejo_review_pages(&api, &scope)
            .expect_err("a changing X-Total-Count must not yield a review snapshot");
        let (result, success) = merge_authority_api_error_result(23, "review-list-read", &error);
        assert!(!success);
        assert_eq!(result["verdict"], "inconclusive");
        assert_eq!(result["merge_attempted"], false);
        assert_eq!(result["merge_authorized"], false);
        assert_eq!(result["error"]["code"], "review_pagination_inconsistent");
        assert_eq!(result["error"]["retryable"], true);
        assert_eq!(
            result["error"]["action"],
            "discard all pages and reread the complete review list before retrying the merge preflight"
        );
        assert_eq!(
            server.join().expect("review pagination fixture completed"),
            2
        );
        assert_eq!(
            *requests.lock().unwrap(),
            vec![
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23/reviews?limit=50&page=1 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/agent-forge-core/pulls/23/reviews?limit=50&page=2 HTTP/1.1",
            ]
        );
    }

    #[test]
    fn mission_012_profile_scopes_every_gate_request_to_one_repository() {
        assert_profile_gate_scope(LocalForgejoTargetProfile::Mission012Clamp);
    }

    #[test]
    fn merge_gate_uses_separate_policy_reader_and_non_admin_merge_credentials() {
        const MERGE_TOKEN: &str = "synthetic-merge-actor-token";
        const POLICY_TOKEN: &str = "synthetic-policy-reader-token";
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let mut protection = merge_authority_branch_protection_fixture();
        protection[0]["merge_whitelist_usernames"] = json!(["forge-merger"]);

        let ((result, success), requests, _) =
            run_profile_gate_against_fixture_with_role_credentials(
                profile,
                protection.clone(),
                protection,
                MERGE_TOKEN,
                POLICY_TOKEN,
                "forge-merger",
            );

        assert!(success, "{result}");
        assert_eq!(result["verdict"], "merged");
        assert!(!requests.is_empty());
        for (line, token) in &requests {
            let expected = if line.contains("/branch_protections ") {
                POLICY_TOKEN
            } else {
                MERGE_TOKEN
            };
            assert_eq!(token, expected, "credential role mismatch for {line}");
        }
        assert!(requests.iter().any(|(line, token)| {
            line.starts_with("POST /api/v1/repos/forge-admin/agent-forge-core/pulls/23/merge ")
                && token == MERGE_TOKEN
        }));
        assert!(
            requests
                .iter()
                .filter(|(line, _)| line.contains("/branch_protections "))
                .count()
                >= 2
        );
    }

    #[test]
    fn legacy_v1_proof_is_rejected_before_any_forgejo_request_or_merge() {
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let (origin, done, server) =
            start_profile_gate_fixture(profile.owner(), profile.repository());
        let scratch = ScratchJournalDir::new();
        // The existing helper deliberately supplies the pre-MERGE-016 V1
        // proof. The authority must refuse it before even reading the PR.
        let inputs = profile_gate_inputs_v1(&scratch.0);
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(PROFILE_FIXTURE_TOKEN)),
        );

        let (result, merge_called) =
            run_merge_authority_for_api(profile, &api, &inputs, &scratch.0.join("merge-locks"))
                .unwrap();
        done.store(true, Ordering::SeqCst);
        let requests = server.join().unwrap();

        assert!(!merge_called);
        assert_eq!(result["verdict"], "rejected");
        assert_eq!(result["error"]["code"], "target_bound_proof_required");
        assert!(
            requests.is_empty(),
            "a V1 proof must be refused before GET or POST"
        );
    }

    #[test]
    fn malformed_target_bound_schema_is_rejected_before_any_forgejo_request() {
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let (origin, done, server) =
            start_profile_gate_fixture(profile.owner(), profile.repository());
        let scratch = ScratchJournalDir::new();
        let target = ProofTargetV2::new(
            profile.selector(),
            origin.clone(),
            profile.owner(),
            profile.repository(),
            MERGE_PREFLIGHT_BASE_BRANCH,
        )
        .unwrap();
        let inputs = profile_gate_inputs_v2(&scratch.0, target);
        let mut proof: Value =
            serde_json::from_slice(&fs::read(&inputs.proof_path).unwrap()).unwrap();
        proof["schema_version"] = json!("2");
        fs::write(&inputs.proof_path, serde_json::to_vec(&proof).unwrap()).unwrap();
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(PROFILE_FIXTURE_TOKEN)),
        );

        let (result, merge_called) =
            run_merge_authority_for_api(profile, &api, &inputs, &scratch.0.join("merge-locks"))
                .unwrap();
        done.store(true, Ordering::SeqCst);
        let requests = server.join().unwrap();

        assert!(!merge_called);
        assert_eq!(result["verdict"], "rejected");
        assert_eq!(result["error"]["code"], "invalid_target_bound_proof");
        assert!(
            requests.is_empty(),
            "malformed proof schema must not read Forgejo"
        );
    }

    #[test]
    fn short_verifier_key_is_refused_before_any_forgejo_request() {
        let profile = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let (origin, done, server) =
            start_profile_gate_fixture(profile.owner(), profile.repository());
        let scratch = ScratchJournalDir::new();
        let target = ProofTargetV2::new(
            profile.selector(),
            origin.clone(),
            profile.owner(),
            profile.repository(),
            MERGE_PREFLIGHT_BASE_BRANCH,
        )
        .unwrap();
        let inputs = profile_gate_inputs_v2(&scratch.0, target);
        fs::write(&inputs.key_path, [0_u8; 31]).unwrap();
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(profile, origin),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(PROFILE_FIXTURE_TOKEN)),
        );

        let (result, merge_called) =
            run_merge_authority_for_api(profile, &api, &inputs, &scratch.0.join("merge-locks"))
                .unwrap();
        done.store(true, Ordering::SeqCst);
        let requests = server.join().unwrap();

        assert!(!merge_called);
        assert_eq!(result["verdict"], "rejected");
        assert_eq!(result["error"]["code"], "invalid_verifier_key");
        assert_eq!(
            result["error"]["action"],
            "replace the host-managed verifier key with at least 32 bytes and issue a new proof"
        );
        assert_eq!(result["error"]["details"]["minimum_bytes"], 32);
        assert!(
            requests.is_empty(),
            "short verifier key must not read Forgejo"
        );
    }

    #[test]
    fn proof_signed_for_other_profile_is_rejected_before_any_forgejo_request() {
        let selected = LocalForgejoTargetProfile::Merge014AgentForgeCore;
        let signed_for = LocalForgejoTargetProfile::Mission012Clamp;
        let (origin, done, server) =
            start_profile_gate_fixture(selected.owner(), selected.repository());
        let scratch = ScratchJournalDir::new();
        let wrong_target = ProofTargetV2::new(
            signed_for.selector(),
            origin.clone(),
            selected.owner(),
            selected.repository(),
            MERGE_PREFLIGHT_BASE_BRANCH,
        )
        .unwrap();
        let inputs = profile_gate_inputs_v2(&scratch.0, wrong_target);
        let api = LocalLoopbackForgejoApi::with_host_credentials(
            LocalForgejoConfigV1::for_profile_at_test_origin(selected, origin),
            LocalApiRetryPolicy::default(),
            Box::new(FixtureHostCredentialProvider::new(PROFILE_FIXTURE_TOKEN)),
        );

        let (result, merge_called) =
            run_merge_authority_for_api(selected, &api, &inputs, &scratch.0.join("merge-locks"))
                .unwrap();
        done.store(true, Ordering::SeqCst);
        let requests = server.join().unwrap();

        assert!(!merge_called);
        assert_eq!(result["verdict"], "rejected");
        assert_eq!(result["error"]["code"], "target_proof_target_mismatch");
        assert!(
            requests.is_empty(),
            "a cross-profile V2 proof must not read Forgejo"
        );
    }

    #[test]
    fn default_profile_gate_requests_are_unchanged() {
        assert_profile_gate_scope(LocalForgejoTargetProfile::Merge014AgentForgeCore);
    }

    #[test]
    fn gate_wrapper_refuses_an_api_scoped_to_another_profile_before_any_input() {
        let scratch = ScratchJournalDir::new();
        let inputs = profile_gate_inputs_v1(&scratch.0);
        let lock_root = scratch.0.join("merge-locks");
        for (profile, other) in [
            (
                LocalForgejoTargetProfile::Mission012Clamp,
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
            ),
            (
                LocalForgejoTargetProfile::Merge014AgentForgeCore,
                LocalForgejoTargetProfile::Mission012Clamp,
            ),
        ] {
            // Port 9 on loopback: nothing may be contacted before the refusal.
            let api =
                LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                    other,
                    "http://127.0.0.1:9".to_owned(),
                ));
            let error = run_merge_authority_for_api(profile, &api, &inputs, &lock_root)
                .expect_err("mismatched profile and API scope must be refused");
            assert!(error.contains("does not match the selected target profile"));
            assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 0);
        }
        assert!(
            !lock_root.exists(),
            "refusal must precede the merge lock and input reads"
        );
    }

    #[test]
    fn gate_wrapper_refuses_a_credential_bound_to_another_origin_before_any_input() {
        let scratch = ScratchJournalDir::new();
        let inputs = profile_gate_inputs_v1(&scratch.0);
        let lock_root = scratch.0.join("merge-locks");
        let profile = LocalForgejoTargetProfile::Mission012Clamp;
        let mut api =
            LocalLoopbackForgejoApi::new(LocalForgejoConfigV1::for_profile_at_test_origin(
                profile,
                "http://127.0.0.1:9".to_owned(),
            ));
        api.credential_provider = Some(OriginBoundHostCredentialProvider {
            origin: LocalForgejoTargetProfile::Merge014AgentForgeCore
                .origin()
                .to_owned(),
            provider: Box::new(FixtureHostCredentialProvider::new(PROFILE_FIXTURE_TOKEN)),
        });
        let error = run_merge_authority_for_api(profile, &api, &inputs, &lock_root)
            .expect_err("credential bound to another origin must be refused");
        assert!(error.contains("credential origin"), "{error:?}");
        assert_eq!(api.upstream_read_attempts.load(Ordering::Relaxed), 0);
        assert!(!lock_root.exists());
    }

    #[test]
    fn each_profile_has_its_own_merge_lock() {
        let scratch = ScratchJournalDir::new();
        let lock_root = scratch.0.join("merge-locks");
        let scope = |profile: LocalForgejoTargetProfile| {
            let config = profile.config();
            BaseBranchScope::new(
                &config.origin,
                &config.owner,
                &config.repository,
                MERGE_PREFLIGHT_BASE_BRANCH,
            )
            .unwrap()
        };
        let default_scope = scope(LocalForgejoTargetProfile::Merge014AgentForgeCore);
        let mission_scope = scope(LocalForgejoTargetProfile::Mission012Clamp);
        let _held = BaseBranchMergeLock::acquire(&lock_root, &default_scope, MERGE_LOCK_WAIT)
            .expect("default profile lock");
        let other =
            BaseBranchMergeLock::acquire(&lock_root, &mission_scope, Duration::from_millis(200));
        assert!(
            other.is_ok(),
            "the MISSION-012 profile must not wait on the default lock"
        );
        drop(other);
        let same =
            BaseBranchMergeLock::acquire(&lock_root, &default_scope, Duration::from_millis(200));
        assert!(
            matches!(same, Err(MergeLockError::TimedOut)),
            "the same profile scope must serialize"
        );
    }

    #[test]
    fn configured_repository_scopes_final_state_and_post_merge_readback_paths() {
        let (origin, done, server) = start_profile_gate_fixture("forge-admin", "mission-012-clamp");
        let api = LocalLoopbackForgejoApi::new(LocalForgejoConfigV1 {
            origin,
            owner: "forge-admin".to_owned(),
            repository: "mission-012-clamp".to_owned(),
        });
        let main_matches = main_commit_matches_validated_base(&api, PROFILE_BASE_SHA);
        let readback = merged_readback(&api, 23);
        done.store(true, Ordering::SeqCst);
        let requests = server
            .join()
            .unwrap()
            .into_iter()
            .map(|(line, _)| line)
            .collect::<Vec<_>>();
        assert_eq!(
            requests,
            [
                "GET /api/v1/repos/forge-admin/mission-012-clamp/branches/main HTTP/1.1",
                "GET /api/v1/repos/forge-admin/mission-012-clamp/pulls/23 HTTP/1.1",
                "GET /api/v1/repos/forge-admin/mission-012-clamp/branches/main HTTP/1.1",
                "GET /api/v1/repos/forge-admin/mission-012-clamp/issues/23 HTTP/1.1",
            ]
        );
        assert_eq!(main_matches, Ok(true));
        let (pull_request, merge_commit, candidate_commit, _, issue, merge_tree, candidate_tree) =
            readback.unwrap();
        assert_eq!(pull_request["merged"], false);
        assert_eq!(merge_commit, Value::Null);
        assert_eq!(candidate_commit, Value::Null);
        assert!(merge_tree.is_none());
        assert!(candidate_tree.is_none());
        assert_eq!(issue["number"], 23);
    }

    #[test]
    fn gate_request_paths_cannot_mix_repositories_from_two_profiles() {
        let (origin, server) = start_http_fixture(vec![(200, Duration::ZERO)]);
        let api = LocalLoopbackForgejoApi::new(LocalForgejoConfigV1 {
            origin,
            owner: "forge-admin".to_owned(),
            repository: "mission-012-clamp".to_owned(),
        });
        for foreign in [
            "/repos/forge-admin/agent-forge-core/pulls/23",
            "/repos/forge-admin/agent-forge-core/pulls/23/merge",
            "/repos/forge-admin/mission-012-clamp/../agent-forge-core/pulls/23",
            "/repos/forge-admin/mission-012-clamp/./pulls/23",
            "/repos/forge-admin/mission-012-clamp/%2e%2e/agent-forge-core/pulls/23",
            "/repos/forge-admin/mission-012-clamp-other/pulls/23",
            "/repos/forge-admin/mission-012-clamp",
            "/repos/forge-admin//mission-012-clamp/pulls/23",
            // Repository-by-ID and other routes outside the allowlisted shapes.
            "/repositories/1",
            "/repositories/1/pulls/23/merge",
            "/orgs/forge-admin/repos",
            "/users/forge-admin/repos",
            "/user/repos",
            "/admin/users",
        ] {
            assert_eq!(
                api.request_api_once(foreign, "GET", None, Duration::from_secs(1))
                    .map(|response| response.status),
                Err(ApiReadError::Forbidden),
                "{foreign} must be refused before any connection"
            );
        }
        let allowed = api
            .request_api_once(
                "/repos/forge-admin/mission-012-clamp/issues/11",
                "GET",
                None,
                Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(allowed.status, 200);
        assert_eq!(
            server.join().unwrap(),
            1,
            "only the in-scope request connected"
        );

        let default_api = LocalLoopbackForgejoApi::default();
        assert_eq!(
            default_api
                .request_api_once(
                    "/repos/forge-admin/mission-012-clamp/pulls/23",
                    "GET",
                    None,
                    Duration::from_secs(1),
                )
                .map(|response| response.status),
            Err(ApiReadError::Forbidden)
        );
    }

    #[test]
    fn mission_012_policy_v4_is_not_a_merge_gate_policy_document() {
        let v4 =
            include_bytes!("../../tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v4.json");
        assert_eq!(
            sha256(v4),
            "e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb"
        );
        assert!(
            serde_json::from_slice::<Policy>(v4).is_err(),
            "MISSION-012 policy v4 must not be silently accepted by the gate schema"
        );
        let document: Value = serde_json::from_slice(v4).unwrap();
        assert!(document.get("merge_approval_policy").is_none());
        assert!(document.get("verifier_id").is_none());
        assert_eq!(document["verifier_identity"], "verifier-bot");
    }

    #[test]
    fn capabilities_list_exact_merge_target_profiles() {
        let document = capabilities_document();
        assert_eq!(
            document["upstream"]["max_response_header_json_bytes"],
            MAX_CURL_HEADER_JSON_BYTES
        );
        assert_eq!(
            document["upstream"]["oversized_response"]["error_code"],
            "upstream_response_too_large"
        );
        assert_eq!(
            document["upstream"]["oversized_response"]["retryable"],
            false
        );
        assert_eq!(
            document["upstream"]["oversized_response"]["components"],
            json!(["body", "header_json", "captured_output"])
        );
        assert_eq!(
            document["upstream"]["duplicate_x_total_count"],
            "reject_response"
        );
        assert_eq!(
            document["upstream"]["duplicate_retry_after"],
            "disable_retry"
        );
        let reconcile = document["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["id"] == "reconcile_webhook_deliveries")
            .unwrap()
            .clone();
        let status_snapshot = document["operations"]
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
        assert_eq!(
            status_snapshot["exit_code_zero_means"],
            "snapshot_read_complete_not_verification_passed"
        );
        assert_eq!(
            status_snapshot["pull_request_head_binding"],
            "caller_supplied_sha_only_not_revalidated"
        );
        assert_eq!(
            status_snapshot["pagination"]["oversized_result"],
            "inconclusive_after_bounded_pages"
        );
        assert_eq!(status_snapshot["pagination"]["first_page_only"], false);
        assert_eq!(
            status_snapshot["pagination"]["maximum_pages"],
            MAX_COMMIT_STATUS_PAGES
        );
        assert_eq!(
            status_snapshot["pagination"]["maximum_statuses"],
            MAX_COMMIT_STATUS_COUNT
        );
        assert_eq!(
            status_snapshot["pagination"]["multi_page_consistency"],
            "sequential_non_atomic"
        );
        assert_eq!(
            status_snapshot["pagination"]["complete_multi_page_recheck"],
            "bounded_second_pass_rejects_changed_status_pages_or_combined_state"
        );
        assert_eq!(
            status_snapshot["pagination"]["maximum_page_reads_without_retries"],
            MAX_COMMIT_STATUS_PAGES * 2
        );
        assert_eq!(
            status_snapshot["pagination"]["changed_total_count"],
            "inconsistent_pagination_actionable_reread"
        );
        assert_eq!(reconcile["target_profile_selector"], "--target-profile");
        assert_eq!(
            reconcile["maximum_projected_output_bytes"],
            MAX_RECONCILE_BATCH_OUTPUT_BYTES
        );
        assert_eq!(
            reconcile["output_budget_measurement"],
            "serialized_canonical_state_json_plus_2048_bytes_per_delivery_for_cli_envelope"
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
        assert_eq!(
            document["credential_boundary"],
            json!({
                "secret_bytes_in_argv_environment_journal_or_response": false,
                "secret_material_resides_in": [
                    "adapter_process_memory",
                    "curl_child_stdin_and_memory",
                ],
                "credential_fd_validation": {
                    "descriptor_shape": "anonymous_pipe_checked",
                    "content": ["bounded_read_to_eof", "token_syntax_validated"],
                    "maximum_input_bytes": MAX_FORGEJO_API_TOKEN_BYTES,
                    "read_timeout_ms": HOST_CREDENTIAL_PIPE_READ_TIMEOUT.as_millis() as u64,
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
        assert_eq!(document["schema_version"], 28);
        assert_eq!(
            document["api_version"],
            "forgejo-reconcile-capabilities-v28"
        );
        assert_eq!(
            reconcile["work_item_assignees_projection"],
            json!({
                "resources": ["issue", "pull_request", "issue_comment_parent", "pull_request_comment_parent"],
                "source": "current_forgejo_resource_api",
                "projected_fields": ["id", "login"],
                "ordering": "ascending_user_id",
                "maximum_items": MAX_RECONCILED_ASSIGNEES,
                "maximum_login_bytes": MAX_RECONCILED_ASSIGNEE_LOGIN_BYTES,
                "missing_field": "null_unknown",
                "null_by_resource": {
                    "issue": "known_empty",
                    "issue_comment_parent": "known_empty",
                    "pull_request": "null_unknown",
                    "pull_request_comment_parent": "null_unknown"
                },
                "empty_array": "known_empty",
                "malformed_duplicate_or_over_limit": "inconclusive_invalid_current_state",
                "freshness": "current_api_read",
                "trust": "untrusted_forgejo_identity",
                "webhook_assignees": "never_used_as_canonical_context",
            })
        );
        assert_eq!(
            reconcile["target_profiles"],
            json!([
                {
                    "selector": "merge-014-agent-forge-core",
                    "origin": "http://127.0.0.1:3130",
                    "owner": "forge-admin",
                    "repository": "agent-forge-core",
                },
                {
                    "selector": "mission-012-clamp",
                    "origin": "http://127.0.0.1:3132",
                    "owner": "forge-admin",
                    "repository": "mission-012-clamp",
                },
            ])
        );
        assert_eq!(
            reconcile["reconcile_only_profile_selector"],
            RECONCILE_ONLY_TARGET_PROFILE_FLAG
        );
        assert_eq!(
            reconcile["reconcile_only_profiles"],
            json!([{
                "selector": "org-team-review-e2e",
                "origin": LOCAL_FORGEJO_ORIGIN,
                "owner": "forge-team-org",
                "repository": "team-review-readback",
                "merge_target": false,
                "forgejo_writes": false,
            }])
        );
        let merge = document["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["id"] == "local_test_merge_authority")
            .unwrap()
            .clone();
        assert_eq!(merge["profile"], "fixed_loopback_test_only");
        assert_eq!(
            merge["writer_admission"],
            json!({
                "selector_flag": "--writer-admission-profile",
                "default_profile": "forgejo_v17_multiwriter",
                "supervisor_capability_flag": "--supervisor-capability-fd",
                "supervisor_capability_protocol": "single bounded inherited AF_UNIX SOCK_SEQPACKET consumed before credential read and rechecked immediately before merge POST",
                "local_single_writer_sandbox": {
                    "explicit_host_selection_required": true,
                    "sandbox_requirement": "the trusted host must exclude every other Forgejo writer for the whole admission, verification, merge, and readback interval",
                    "sandbox_exclusion_verified_by_this_adapter": false,
                    "supervisor_policy_independently_verified": false,
                    "trusted_supervisor_launcher_implemented_here": false,
                },
                "forgejo_v17_multiwriter": {
                    "admitted": false,
                    "verdict": "inconclusive",
                    "error_code": "server_expected_base_precondition_unavailable",
                    "server_expected_base_precondition": "not_mandatory_or_unavailable",
                    "refuse_before": ["merge_lock", "host_input_read", "forgejo_read", "merge_post"],
                },
            })
        );
        assert_eq!(
            merge["mission_admission_validation"]["generation_race_protection"],
            json!({
                "lock_scope": "cooperative_per_mission_lock_shared_with_generation_allocation",
                "claim_revalidated_after_lock_acquisition": true,
                "lock_held_during": [
                    "final_pull_request_base_and_branch_policy_reads",
                    "merge_post",
                    "postmerge_readback",
                ],
                "concurrent_allocation_result": "retryable_execution_mission_busy",
                "excludes_noncooperating_journal_or_forgejo_writers": false,
            })
        );
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
        assert_eq!(
            merge["postmerge_pull_request_identity"],
            json!({
                "field": "base.repo.full_name",
                "required": true,
                "must_equal": "selected_target_profile_owner_and_repository",
                "mismatch": "inconclusive",
            })
        );
        let profile_entry = |selector: &str, default: bool, origin: &str, repository: &str| {
            let repo = format!("/api/v1/repos/forge-admin/{repository}");
            let mission_admission_inputs = if selector == "mission-012-clamp" {
                json!({
                    "required": true,
                    "flags": ["--mission-admission-file", "--mission-execution-journal"],
                    "source_of_truth": "private mission-execution.jsonl durable allocation and claim journal",
                })
            } else {
                json!({"required": false, "flags": []})
            };
            json!({
                "selector": selector,
                "default": default,
                "origin": origin,
                "owner": "forge-admin",
                "repository": repository,
                "base_branch": "main",
                "writer_admission_profile": "defaults_to_forgejo_v17_multiwriter; explicit_local_profile_requires_supervisor_capability",
                "writer_admission_source": "kernel_peer_credentials_and_live_process_bindings_from_inherited_supervisor_socket; supervisor_policy_remains_trusted_host_input",
                "supervisor_capability": {
                    "flag": "--supervisor-capability-fd",
                    "transport": "inherited_AF_UNIX_SOCK_SEQPACKET",
                    "packet_max_bytes": 8192,
                    "maximum_lifetime_seconds": 120,
                    "consumed_before_merge_credential": true,
                    "revalidated_immediately_before_merge_post": true,
                    "bindings": ["target_profile", "origin", "repository", "pull_request", "base_branch", "supervisor_peer_pid_uid_start_time_namespaces", "gate_pid_start_time_namespaces", "forgejo_pid_start_time_namespaces", "expiry", "nonce"],
                    "independent_supervisor_policy_verification": false,
                    "trusted_supervisor_launcher_implemented_here": false,
                },
                "mission_admission_inputs": mission_admission_inputs,
                "forgejo_reads": [
                    format!("{repo}/pulls/{{number}}"),
                    format!("{repo}/pulls/{{number}}/reviews"),
                    format!("{repo}/branch_protections"),
                    format!("{repo}/branches/main"),
                    "/api/v1/user",
                    "/api/v1/users/{reviewer-login}",
                ],
                "forgejo_writes": [format!("{repo}/pulls/{{number}}/merge")],
                "readback": [
                    format!("{repo}/pulls/{{number}}"),
                    format!("{repo}/git/commits/{{merge-commit-sha}}"),
                    format!("{repo}/git/commits/{{candidate-head-sha}}"),
                    format!("{repo}/branches/main"),
                    format!("{repo}/issues/{{number}}"),
                    format!(
                        "{repo}/git/trees/{{merge-commit-sha}}?page={{page}}&per_page=1000&recursive=false"
                    ),
                    format!(
                        "{repo}/git/trees/{{candidate-head-sha}}?page={{page}}&per_page=1000&recursive=false"
                    ),
                ],
            })
        };
        assert_eq!(
            merge["target_profiles"],
            json!([
                profile_entry(
                    "merge-014-agent-forge-core",
                    true,
                    "http://127.0.0.1:3130",
                    "agent-forge-core"
                ),
                profile_entry(
                    "mission-012-clamp",
                    false,
                    "http://127.0.0.1:3132",
                    "mission-012-clamp"
                ),
            ])
        );
        assert_eq!(merge["target_profile_selector"], "--target-profile");
        assert_eq!(merge["uses_configuration_allowed_origins"], false);
        assert_eq!(
            merge["credential_origin_binding"],
            "selected_target_profile_origin"
        );
        assert_eq!(
            merge["request_paths_shown_for_profile"],
            "merge-014-agent-forge-core"
        );
        assert_eq!(
            merge["result_target_fields"],
            json!(["selector", "origin", "owner", "repository", "base_branch"])
        );
        assert_eq!(merge["proof_input_schema"], "target_bound_proof_v2_only");
        assert_eq!(
            merge["proof_binding_scope"],
            json!([
                "profile_selector",
                "canonical_origin",
                "owner",
                "repository",
                "base_branch",
                "signed_evidence",
            ])
        );
        assert_eq!(
            merge["legacy_v1_merge_authority"],
            "rejected_before_forgejo_read"
        );
        assert_eq!(
            merge["credential_roles"]["merge_actor"],
            "required_inherited_anonymous_pipe_fd"
        );
        assert_eq!(
            merge["credential_roles"]["branch_protection_reader"]["flag"],
            "--policy-credential-fd"
        );
        assert_eq!(
            merge["credential_roles"]["branch_protection_reader"]["optional"],
            true
        );

        // Config-file reconciliation remains limited to 3130. The separate
        // compiled profile selector may read the 3132 mission fixture, while
        // read-only preflight stays pinned to the default profile.
        let configuration = &document["configuration"];
        assert_eq!(
            configuration["allowed_origins"],
            json!(["http://127.0.0.1:3130"])
        );
        assert_eq!(
            configuration["allowed_origins_scope"],
            json!(["reconcile_webhook_deliveries"])
        );
        let preflight = document["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["id"] == "read_only_merge_preflight")
            .unwrap();
        assert_eq!(preflight["target"], "fixed_default_profile");
        assert_eq!(preflight["reads_configuration_file"], false);
        assert_eq!(configuration["selects_merge_target_profile"], false);
        let rendered_configuration = configuration.to_string();
        assert!(!rendered_configuration.contains("3132"));
        assert!(!rendered_configuration.contains("mission-012-clamp"));
        assert!(
            !preflight.to_string().contains("3132")
                && !preflight.to_string().contains("mission-012-clamp"),
            "read-only preflight must stay pinned to the default profile"
        );
    }
}
