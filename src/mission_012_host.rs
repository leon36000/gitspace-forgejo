//! Narrow local tool host for the MISSION-012 adapter exercise.
//!
//! Model-facing writes are limited to `src/lib.rs`; acceptance tests are read-only.
//! `cargo.test` has a fixed offline command in a bubblewrap sandbox. Candidate-ready
//! finalization is refused until the unchanged base produced the expected red test
//! and the changed source produced a fresh green result.

use agent_forge_core::{
    agent_runtime::{
        AgentRuntimeErrorV1, AgentToolPortV1, RuntimeCapabilitiesV1, ToolCallReceiptV1,
    },
    sha256,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CString, OsString},
    fs::{self, File},
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

const MAX_TEXT_BYTES: usize = 65_536;
const MAX_CAPTURE_BYTES: usize = 65_536;
pub const TEST_TIMEOUT: Duration = Duration::from_secs(120);
const VERIFIER_MEMORY_MAX_BYTES: u64 = 1_073_741_824;
const VERIFIER_TASKS_MAX: u32 = 64;
const VERIFIER_CPU_QUOTA_PERCENT: u32 = 200;
pub const MISSION_VERIFICATION_CPU_BUDGET_MS: u64 = 1_200_000;
const VERIFIER_CPU_ACCOUNTING_MARGIN_MS: u64 = 1_000;
const VERIFIER_TARGET_TMPFS_BYTES: u64 = 268_435_456;
const VERIFIER_TMP_TMPFS_BYTES: u64 = 134_217_728;
const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";
const EXPECTED_FAILURE: &str = "values_outside_the_range_clamp_to_the_nearest_boundary";
const EXPECTED_GREEN_SUMMARY: &str = "2 passed; 0 failed";
// This first live task is intentionally a narrow, task-specific acceptance
// profile. Its trusted source contract prevents candidate code from forging
// libtest output (for example by printing a green summary and calling exit(0)).
const EXPECTED_BASE_SOURCE_SHA256: &str =
    "de0175e0fc5341a0ff3080f647ee16f4f4c1c45a205fc01ee8ec73899b3f9aa2";
const EXPECTED_FIXED_SOURCE_SHA256: &str =
    "173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15";
const EXPECTED_REQUIREMENT_SHA256: &str =
    "3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431";
const EXPECTED_POLICY_SHA256: &str =
    "e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb";
// These build-time pins are the verifier's trust roots. Callers may name the
// commits and snapshots to inspect, but cannot nominate new requirement or
// policy digests at verification time.
const TRUSTED_VERIFIER_ID: &str = "verifier-bot";
const VERIFICATION_COMMAND: [&str; 4] = ["/toolchain/bin/cargo", "test", "--offline", "--locked"];
const MAX_SNAPSHOT_FILES: usize = 20_000;
const MAX_SNAPSHOT_FILE_BYTES: u64 = 1 << 20;
const MAX_SNAPSHOT_TOTAL_BYTES: u64 = 16 << 20;
const GIT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(10);
static WRITE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
pub enum ToolRequest {
    MissionContext,
    ReadText { path: String },
    WriteText { path: String, content: String },
    CargoTest,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ToolRefusal {
    UnknownTool,
    InvalidInput,
    PathNotAllowed,
}

pub fn parse_model_tool(tool_id: &str, input: &Value) -> Result<ToolRequest, ToolRefusal> {
    fn exact_object_keys(input: &Value, allowed: &[&str]) -> bool {
        let Some(object) = input.as_object() else {
            return false;
        };
        object.len() == allowed.len() && allowed.iter().all(|key| object.contains_key(*key))
    }

    match tool_id {
        "mission.context" if exact_object_keys(input, &[]) => Ok(ToolRequest::MissionContext),
        "mission.context" => Err(ToolRefusal::InvalidInput),
        "workspace.read_text" => {
            if !exact_object_keys(input, &["path"]) {
                return Err(ToolRefusal::InvalidInput);
            }
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or(ToolRefusal::InvalidInput)?;
            if ![
                "Cargo.toml",
                "Cargo.lock",
                "src/lib.rs",
                "tests/acceptance.rs",
            ]
            .contains(&path)
            {
                return Err(ToolRefusal::PathNotAllowed);
            }
            Ok(ToolRequest::ReadText {
                path: path.to_owned(),
            })
        }
        "workspace.write_text" => {
            if !exact_object_keys(input, &["path", "content"]) {
                return Err(ToolRefusal::InvalidInput);
            }
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or(ToolRefusal::InvalidInput)?;
            if path != "src/lib.rs" {
                return Err(ToolRefusal::PathNotAllowed);
            }
            let content = input
                .get("content")
                .and_then(Value::as_str)
                .ok_or(ToolRefusal::InvalidInput)?;
            if content.len() > MAX_TEXT_BYTES {
                return Err(ToolRefusal::InvalidInput);
            }
            Ok(ToolRequest::WriteText {
                path: path.to_owned(),
                content: content.to_owned(),
            })
        }
        "cargo.test" if exact_object_keys(input, &[]) => Ok(ToolRequest::CargoTest),
        "cargo.test" => Err(ToolRefusal::InvalidInput),
        _ => Err(ToolRefusal::UnknownTool),
    }
}

#[derive(Clone, Debug)]
pub struct SandboxConfig {
    pub base_dir: PathBuf,
    pub candidate_dir: PathBuf,
    pub target_root: PathBuf,
    pub rust_sysroot: PathBuf,
}

impl SandboxConfig {
    pub fn new(
        base_dir: impl Into<PathBuf>,
        candidate_dir: impl Into<PathBuf>,
        target_root: impl Into<PathBuf>,
        rust_sysroot: impl Into<PathBuf>,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            candidate_dir: candidate_dir.into(),
            target_root: target_root.into(),
            rust_sysroot: rust_sysroot.into(),
        }
    }
}

#[derive(Clone, Debug)]
struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

#[derive(Clone, Debug)]
pub struct SandboxTestResult {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub output_truncated: bool,
    pub elapsed_ms: u64,
}

fn tool_error(code: &str, action: &str, details: Value) -> AgentRuntimeErrorV1 {
    AgentRuntimeErrorV1::new(code, false, action, details)
}

fn valid_sha256_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_regular_file(root: &Path, relative: &str) -> Result<Vec<u8>, AgentRuntimeErrorV1> {
    let path = match relative {
        "Cargo.toml" | "Cargo.lock" => root.join(relative),
        "src/lib.rs" => root.join("src/lib.rs"),
        "tests/acceptance.rs" => root.join("tests/acceptance.rs"),
        _ => {
            return Err(tool_error(
                "tool_path_denied",
                "read one of the advertised mission files",
                json!({"path":relative}),
            ));
        }
    };
    let metadata = fs::symlink_metadata(&path).map_err(|_| {
        tool_error(
            "tool_file_unavailable",
            "ask the host to restore the exact candidate snapshot",
            json!({"path":relative}),
        )
    })?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_TEXT_BYTES as u64 {
        return Err(tool_error(
            "tool_file_not_regular_or_too_large",
            "use only regular files up to the host text limit",
            json!({"path":relative,"max_bytes":MAX_TEXT_BYTES}),
        ));
    }
    fs::read(path).map_err(|_| {
        tool_error(
            "tool_file_unavailable",
            "ask the host to restore the exact candidate snapshot",
            json!({"path":relative}),
        )
    })
}

fn read_verification_binding_file(
    root: &Path,
    relative: &str,
) -> Result<Vec<u8>, AgentRuntimeErrorV1> {
    if ![
        "requirements/MISSION-012-CLAMP-001.json",
        "policy/active.json",
        "policy/MISSION-012-CLAMP-001-v4.json",
    ]
    .contains(&relative)
    {
        return Err(tool_error(
            "verification_binding_path_invalid",
            "use only the pinned MISSION-012 requirement and active policy paths",
            json!({}),
        ));
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|_| {
        tool_error(
            "verification_binding_missing",
            "restore the protected requirement and active policy files from the pinned base commit",
            json!({"path":relative}),
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_TEXT_BYTES as u64
    {
        return Err(tool_error(
            "verification_binding_invalid",
            "use regular protected binding files under the 64 KiB host limit",
            json!({"path":relative}),
        ));
    }
    fs::read(path).map_err(|_| {
        tool_error(
            "verification_binding_unreadable",
            "restore the protected requirement and active policy files",
            json!({"path":relative}),
        )
    })
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn verifier_scope_arguments(timeout: Duration) -> Vec<OsString> {
    let runtime_seconds = timeout
        .as_secs()
        .saturating_add(u64::from(timeout.subsec_nanos() != 0))
        .max(1);
    [
        "--user".to_owned(),
        "--scope".to_owned(),
        "--quiet".to_owned(),
        "--collect".to_owned(),
        "--property".to_owned(),
        format!("MemoryMax={VERIFIER_MEMORY_MAX_BYTES}"),
        "--property".to_owned(),
        format!("TasksMax={VERIFIER_TASKS_MAX}"),
        "--property".to_owned(),
        format!("CPUQuota={VERIFIER_CPU_QUOTA_PERCENT}%"),
        "--property".to_owned(),
        format!("RuntimeMaxSec={runtime_seconds}s"),
        "--property".to_owned(),
        "TimeoutStopSec=1s".to_owned(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// Hash the local verifier limits as one stable policy identity. A journal
/// opened under a different CPU budget or per-test cgroup ceiling must not silently
/// reuse an older mission's accounting history.
pub fn verifier_cpu_policy_sha256() -> String {
    let policy = format!(
        "mission-012-verifier-cpu-v1|mission_cpu_budget_ms={MISSION_VERIFICATION_CPU_BUDGET_MS}|cpu_quota_percent={VERIFIER_CPU_QUOTA_PERCENT}|test_timeout_ms={}|memory_max_bytes={VERIFIER_MEMORY_MAX_BYTES}|tasks_max={VERIFIER_TASKS_MAX}|target_tmpfs_bytes={VERIFIER_TARGET_TMPFS_BYTES}|tmp_tmpfs_bytes={VERIFIER_TMP_TMPFS_BYTES}",
        TEST_TIMEOUT.as_millis()
    );
    sha256(policy.as_bytes())
}

/// Maximum CPU time a single verifier scope could consume under its CPU quota
/// and runtime deadline. RuntimeMaxSec is rounded up to a whole second by
/// `verifier_scope_arguments`, so the reservation uses that same ceiling.
pub fn verifier_cpu_reservation_ms(timeout: Duration) -> u64 {
    let runtime_seconds = timeout
        .as_secs()
        .saturating_add(u64::from(timeout.subsec_nanos() != 0))
        .max(1);
    runtime_seconds
        .saturating_mul(1_000)
        .saturating_mul(u64::from(VERIFIER_CPU_QUOTA_PERCENT))
        .saturating_add(99)
        / 100
}

/// Convert observed wall time to a conservative CPU upper bound using the
/// enforced cgroup quota. The margin covers scheduling and scope teardown;
/// the result never exceeds the durable worst-case reservation.
pub fn verifier_cpu_usage_upper_bound_ms(elapsed_ms: u64, reserved_cpu_ms: u64) -> u64 {
    elapsed_ms
        .saturating_add(VERIFIER_CPU_ACCOUNTING_MARGIN_MS)
        .saturating_mul(u64::from(VERIFIER_CPU_QUOTA_PERCENT))
        .saturating_add(99)
        .checked_div(100)
        .unwrap_or(u64::MAX)
        .max(1)
        .min(reserved_cpu_ms)
}

fn systemd_runtime_directory() -> Option<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let directory = PathBuf::from(format!("/run/user/{uid}"));
    let metadata = fs::symlink_metadata(&directory).ok()?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
    {
        return None;
    }
    let bus_metadata = fs::symlink_metadata(directory.join("bus")).ok()?;
    if !bus_metadata.file_type().is_socket() || bus_metadata.uid() != uid {
        return None;
    }
    Some(directory)
}

fn configure_systemd_connection_environment(command: &mut Command) {
    command.env_clear();
    if let Some(directory) = systemd_runtime_directory() {
        command.env("XDG_RUNTIME_DIR", &directory).env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/bus", directory.display()),
        );
    }
}

fn manifest_sha256(manifest: &BTreeMap<String, String>) -> String {
    let mut canonical = Vec::new();
    for (path, hash) in manifest {
        canonical.extend_from_slice(path.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(hash.as_bytes());
        canonical.push(b'\n');
    }
    sha256(&canonical)
}

fn approved_mission_candidate(base_source_sha256: &str, candidate_source: &[u8]) -> bool {
    base_source_sha256 == EXPECTED_BASE_SOURCE_SHA256
        && sha256(candidate_source) == EXPECTED_FIXED_SOURCE_SHA256
}

fn git_output(repository: &Path, args: &[&str]) -> Result<Vec<u8>, AgentRuntimeErrorV1> {
    let mut command = Command::new("/usr/bin/git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("-C")
        .arg(repository)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = command.spawn().map_err(|_| {
        tool_error(
            "git_verification_unavailable",
            "provide a local trusted Git repository and Git executable to bind both snapshots to their commit IDs",
            json!({}),
        )
    })?;
    let result = spawn_capture(child, GIT_INSPECTION_TIMEOUT)?;
    if result.exit_code != Some(0) || result.timed_out || result.output_truncated {
        return Err(tool_error(
            "git_verification_failed",
            "inspect the local Git repository and retry against the exact immutable commits",
            json!({
                "exit_code":result.exit_code,
                "timed_out":result.timed_out,
                "output_truncated":result.output_truncated,
            }),
        ));
    }
    Ok(result.stdout.into_bytes())
}

fn git_blob_oid(repository: &Path, bytes: &[u8]) -> Result<String, AgentRuntimeErrorV1> {
    let mut command = Command::new("/usr/bin/git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("-C")
        .arg(repository)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|_| {
        tool_error(
            "git_verification_unavailable",
            "provide a local trusted Git repository and Git executable to bind both snapshots to their commit IDs",
            json!({}),
        )
    })?;
    let mut input = child.stdin.take().ok_or_else(|| {
        tool_error(
            "git_verification_pipe_missing",
            "repair the local Git verifier",
            json!({}),
        )
    })?;
    input.write_all(bytes).map_err(|_| {
        tool_error(
            "git_verification_input_failed",
            "retry Git blob verification with a fresh source snapshot",
            json!({}),
        )
    })?;
    drop(input);
    let result = spawn_capture(child, GIT_INSPECTION_TIMEOUT)?;
    if result.exit_code != Some(0) || result.timed_out || result.output_truncated {
        return Err(tool_error(
            "git_verification_failed",
            "inspect the local Git repository and retry against the exact immutable commits",
            json!({
                "exit_code":result.exit_code,
                "timed_out":result.timed_out,
                "output_truncated":result.output_truncated,
            }),
        ));
    }
    let output = result.stdout.trim();
    if output.len() != 40 || !output.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(tool_error(
            "git_blob_id_invalid",
            "retry with a SHA-1 Git repository and an intact local Git executable",
            json!({}),
        ));
    }
    Ok(output.to_ascii_lowercase())
}

fn git_commit_matches_snapshot(
    repository: &Path,
    commit: &str,
    root: &Path,
    snapshot: &BTreeMap<String, String>,
) -> Result<bool, AgentRuntimeErrorV1> {
    let normalized_commit = commit.to_ascii_lowercase();
    let commit_expression = format!("{normalized_commit}^{{commit}}");
    let resolved = git_output(repository, &["rev-parse", "--verify", &commit_expression])?;
    if std::str::from_utf8(&resolved).ok().map(str::trim) != Some(normalized_commit.as_str()) {
        return Ok(false);
    }
    let output = git_output(
        repository,
        &["ls-tree", "-r", "-z", "--full-tree", &normalized_commit],
    )?;
    let listing = std::str::from_utf8(&output).map_err(|_| {
        tool_error(
            "git_tree_path_not_utf8",
            "use a Git snapshot whose paths can be represented in the local verifier manifest",
            json!({}),
        )
    })?;
    let mut seen = BTreeMap::<String, (String, String)>::new();
    for record in listing.split_terminator('\0') {
        let Some((metadata, path)) = record.split_once('\t') else {
            return Ok(false);
        };
        let mut fields = metadata.split_ascii_whitespace();
        let (Some(mode), Some(kind), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Ok(false);
        };
        if !matches!(mode, "100644" | "100755")
            || kind != "blob"
            || oid.len() != 40
            || !oid.bytes().all(|byte| byte.is_ascii_hexdigit())
            || path.is_empty()
            || Path::new(path).is_absolute()
            || Path::new(path).components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::RootDir
                )
            })
        {
            return Ok(false);
        }
        if seen.contains_key(path) {
            return Ok(false);
        }
        let Some(snapshot_value) = snapshot.get(path) else {
            return Ok(false);
        };
        let Some((snapshot_mode, _)) = snapshot_value.split_once(':') else {
            return Ok(false);
        };
        if snapshot_mode != mode {
            return Ok(false);
        }
        let file_path = root.join(path);
        let metadata = fs::symlink_metadata(&file_path).map_err(|_| {
            tool_error(
                "git_snapshot_file_unavailable",
                "restore the exact source snapshot from its pinned Git commit",
                json!({"path":path}),
            )
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
            return Ok(false);
        }
        let bytes = fs::read(&file_path).map_err(|_| {
            tool_error(
                "git_snapshot_file_unavailable",
                "restore the exact source snapshot from its pinned Git commit",
                json!({"path":path}),
            )
        })?;
        let actual_oid = git_blob_oid(repository, &bytes)?;
        if !actual_oid.eq_ignore_ascii_case(oid) {
            return Ok(false);
        }
        seen.insert(path.to_owned(), (mode.to_owned(), actual_oid));
    }
    Ok(seen.len() == snapshot.len())
}

/// Workspace snapshot proven, by a host-owned local Git repository, to be the
/// exact tree of the admitted base commit.
#[derive(Debug, PartialEq, Eq)]
pub struct AdmittedBaseTree {
    pub commit: String,
    pub tree: String,
    pub branch: String,
    pub manifest_sha256: String,
}

fn base_binding_error(code: &str, action: &str, details: Value) -> AgentRuntimeErrorV1 {
    AgentRuntimeErrorV1::new(code, false, action, details)
}

fn resolve_git_object(
    repository: &Path,
    expression: &str,
) -> Result<Option<String>, AgentRuntimeErrorV1> {
    match git_output(
        repository,
        &["rev-parse", "--verify", "--quiet", expression],
    ) {
        Ok(output) => Ok(std::str::from_utf8(&output)
            .ok()
            .map(|value| value.trim().to_owned())),
        Err(error) if error.code == "git_verification_failed" => Ok(None),
        Err(error) => Err(error),
    }
}

/// Prove, before any context or tool is exposed, that `base_dir` and the
/// initial `candidate_dir` are both exactly the tree of the full admitted
/// `commit`, and that `commit` is still the head of `branch` in the trusted
/// repository. The commit string alone is never trusted: it must resolve to
/// itself as a commit object in `repository`, and every snapshot file must
/// hash to the blob recorded in that commit's tree.
///
/// This binds the snapshots at the time of the check only; a process running
/// as the same uid can still change either directory afterwards.
pub fn bind_workspace_to_admitted_commit(
    repository: &Path,
    branch: &str,
    commit: &str,
    base_dir: &Path,
    candidate_dir: &Path,
) -> Result<AdmittedBaseTree, AgentRuntimeErrorV1> {
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(base_binding_error(
            "workspace_base_commit_invalid",
            "admit the mission against a full lowercase 40-hex Git commit ID",
            json!({}),
        ));
    }
    if branch.is_empty()
        || branch.starts_with('-')
        || !branch
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(base_binding_error(
            "workspace_base_branch_invalid",
            "use the host-pinned protected base branch name",
            json!({}),
        ));
    }
    let repository = canonical_directory(repository).map_err(|_| {
        base_binding_error(
            "trusted_git_repository_unavailable",
            "configure an absolute host-owned local Git repository",
            json!({}),
        )
    })?;
    let resolved = resolve_git_object(&repository, &format!("{commit}^{{commit}}"))?;
    if resolved.as_deref() != Some(commit) {
        return Err(base_binding_error(
            "workspace_base_commit_unknown",
            "fetch the admitted commit into the trusted repository or re-run admission",
            json!({"base_commit":commit}),
        ));
    }
    let head = resolve_git_object(&repository, &format!("refs/heads/{branch}^{{commit}}"))?;
    if head.as_deref() != Some(commit) {
        return Err(base_binding_error(
            "workspace_base_commit_stale",
            "re-run admission against the current protected branch head",
            json!({"base_commit":commit,"branch":branch,"trusted_head":head}),
        ));
    }
    let tree = resolve_git_object(&repository, &format!("{commit}^{{tree}}"))?
        .filter(|tree| tree.len() == 40 && tree.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            base_binding_error(
                "workspace_base_commit_unknown",
                "fetch the admitted commit tree into the trusted repository",
                json!({"base_commit":commit}),
            )
        })?;
    let base_manifest = snapshot_manifest(base_dir)?;
    if !git_commit_matches_snapshot(&repository, commit, base_dir, &base_manifest)? {
        return Err(base_binding_error(
            "workspace_base_tree_mismatch",
            "recreate the protected base snapshot from the admitted commit",
            json!({"base_commit":commit,"tree":tree}),
        ));
    }
    let candidate_manifest = snapshot_manifest(candidate_dir)?;
    if candidate_manifest != base_manifest
        || !git_commit_matches_snapshot(&repository, commit, candidate_dir, &candidate_manifest)?
    {
        return Err(base_binding_error(
            "candidate_not_at_admitted_base",
            "create a private candidate from the admitted commit tree before starting the agent",
            json!({"base_commit":commit,"tree":tree}),
        ));
    }
    Ok(AdmittedBaseTree {
        commit: commit.to_owned(),
        tree,
        branch: branch.to_owned(),
        manifest_sha256: manifest_sha256(&base_manifest),
    })
}

fn write_source(candidate_dir: &Path, content: &str) -> Result<(), AgentRuntimeErrorV1> {
    if content.len() > MAX_TEXT_BYTES {
        return Err(tool_error(
            "tool_write_too_large",
            "keep the source edit under the host's 64 KiB limit",
            json!({"max_bytes":MAX_TEXT_BYTES}),
        ));
    }
    let candidate_c = CString::new(candidate_dir.as_os_str().as_encoded_bytes()).map_err(|_| {
        tool_error(
            "tool_workspace_invalid",
            "ask the host to recreate the candidate snapshot",
            json!({}),
        )
    })?;
    let candidate_fd = unsafe {
        libc::open(
            candidate_c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if candidate_fd < 0 {
        return Err(tool_error(
            "tool_workspace_invalid",
            "ask the host to recreate the candidate snapshot without symbolic links",
            json!({}),
        ));
    }
    let candidate_dir = unsafe { File::from_raw_fd(candidate_fd) };
    let src_name = c"src";
    let src_fd = unsafe {
        libc::openat(
            candidate_dir.as_raw_fd(),
            src_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if src_fd < 0 {
        return Err(tool_error(
            "tool_workspace_invalid",
            "ask the host to recreate the candidate source directory without symbolic links",
            json!({}),
        ));
    }
    let src_dir = unsafe { File::from_raw_fd(src_fd) };
    let source_name = c"lib.rs";
    let mut source_stat = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe {
        libc::fstatat(
            src_dir.as_raw_fd(),
            source_name.as_ptr(),
            &mut source_stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
        || source_stat.st_mode & libc::S_IFMT != libc::S_IFREG
        || source_stat.st_nlink != 1
    {
        return Err(tool_error(
            "tool_workspace_invalid",
            "restore src/lib.rs as one regular file without hard links or symbolic links",
            json!({"path":"src/lib.rs"}),
        ));
    }

    let temp_name = CString::new(format!(
        ".mission012-src-lib-{}-{}",
        std::process::id(),
        WRITE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated temporary filename has no NUL bytes");
    let temp_fd = unsafe {
        libc::openat(
            src_dir.as_raw_fd(),
            temp_name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if temp_fd < 0 {
        return Err(tool_error(
            "tool_write_failed",
            "ask the host to restore src/lib.rs and retry once",
            json!({"path":"src/lib.rs"}),
        ));
    }
    let mut temp_file = unsafe { File::from_raw_fd(temp_fd) };
    let mode = source_stat.st_mode & 0o777;
    let write_result = temp_file
        .write_all(content.as_bytes())
        .and_then(|()| temp_file.sync_all())
        .and_then(|()| {
            if unsafe { libc::fchmod(temp_file.as_raw_fd(), mode) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    if write_result.is_err() {
        unsafe {
            libc::unlinkat(src_dir.as_raw_fd(), temp_name.as_ptr(), 0);
        }
        return Err(tool_error(
            "tool_write_failed",
            "ask the host to restore src/lib.rs and retry once",
            json!({"path":"src/lib.rs"}),
        ));
    }
    drop(temp_file);
    if unsafe {
        libc::renameat(
            src_dir.as_raw_fd(),
            temp_name.as_ptr(),
            src_dir.as_raw_fd(),
            source_name.as_ptr(),
        )
    } != 0
    {
        unsafe {
            libc::unlinkat(src_dir.as_raw_fd(), temp_name.as_ptr(), 0);
        }
        return Err(tool_error(
            "tool_write_failed",
            "ask the host to restore src/lib.rs and retry once",
            json!({"path":"src/lib.rs"}),
        ));
    }
    if unsafe { libc::fsync(src_dir.as_raw_fd()) } != 0 {
        return Err(tool_error(
            "tool_write_durability_uncertain",
            "inspect the candidate snapshot and restart if the atomic source replacement is not durable",
            json!({"path":"src/lib.rs"}),
        ));
    }
    Ok(())
}

/// Return a digest from a no-follow read of the host-fixed candidate source.
/// The caller still has to hold the mission fence and the final writer must
/// compare this value again immediately before its atomic replacement.
pub fn candidate_source_sha256(candidate_dir: &Path) -> Result<String, AgentRuntimeErrorV1> {
    let source = read_regular_file(candidate_dir, "src/lib.rs")?;
    Ok(sha256(&source))
}

/// Restore the candidate's single authorized source write to its admitted
/// pre-state before a new host replays an unresolved durable write intent.
/// The caller must first bind `base_dir` to the admitted Git commit, acquire a
/// fresh execution lease, and validate the intent through the execution
/// journal. This helper only accepts the exact before/after hashes and refuses
/// any other candidate-path change.
pub fn validate_candidate_for_workspace_write_recovery(
    base_dir: &Path,
    candidate_dir: &Path,
    expected_before_sha256: &str,
    expected_after_sha256: &str,
) -> Result<bool, AgentRuntimeErrorV1> {
    if !valid_sha256_digest(expected_before_sha256)
        || !valid_sha256_digest(expected_after_sha256)
        || expected_before_sha256 != expected_before_sha256.to_ascii_lowercase()
        || expected_after_sha256 != expected_after_sha256.to_ascii_lowercase()
    {
        return Err(tool_error(
            "workspace_recovery_hash_invalid",
            "reload lowercase source hashes from the validated durable write intent",
            json!({}),
        ));
    }
    let base_dir = canonical_directory(base_dir)?;
    let candidate_dir = canonical_directory(candidate_dir)?;
    if paths_overlap(&base_dir, &candidate_dir) {
        return Err(tool_error(
            "sandbox_paths_overlap",
            "use separate protected-base and candidate directories for write recovery",
            json!({}),
        ));
    }
    let base_manifest = snapshot_manifest(&base_dir)?;
    let candidate_manifest = snapshot_manifest(&candidate_dir)?;
    let base_source_entry = base_manifest.get("src/lib.rs").ok_or_else(|| {
        tool_error(
            "workspace_recovery_base_mismatch",
            "recreate the admitted base snapshot with its pinned src/lib.rs",
            json!({}),
        )
    })?;
    let candidate_source_entry = candidate_manifest.get("src/lib.rs").ok_or_else(|| {
        tool_error(
            "workspace_recovery_candidate_diverged",
            "restore the candidate's exact src/lib.rs path before recovering its write",
            json!({}),
        )
    })?;
    let (Some((base_mode, base_manifest_source_hash)), Some((candidate_mode, _))) = (
        base_source_entry.split_once(':'),
        candidate_source_entry.split_once(':'),
    ) else {
        return Err(tool_error(
            "workspace_recovery_candidate_diverged",
            "restore the candidate's regular src/lib.rs file before recovering its write",
            json!({}),
        ));
    };
    let base_source = read_regular_file(&base_dir, "src/lib.rs")?;
    let candidate_source = read_regular_file(&candidate_dir, "src/lib.rs")?;
    let base_source_sha256 = sha256(&base_source);
    let candidate_source_sha256 = sha256(&candidate_source);
    if base_source_sha256 != expected_before_sha256
        || base_manifest_source_hash != expected_before_sha256
    {
        return Err(tool_error(
            "workspace_recovery_base_mismatch",
            "recreate the admitted base source pinned by the unresolved write intent",
            json!({
                "expected_before_sha256":expected_before_sha256,
                "observed_base_sha256":base_source_sha256,
            }),
        ));
    }
    if candidate_mode != base_mode
        || (candidate_source_sha256 != expected_before_sha256
            && candidate_source_sha256 != expected_after_sha256)
    {
        return Err(tool_error(
            "workspace_recovery_candidate_diverged",
            "recreate the candidate with the exact pre-write or post-write source hash and mode recorded by the unresolved intent",
            json!({
                "expected_before_sha256":expected_before_sha256,
                "expected_after_sha256":expected_after_sha256,
                "observed_candidate_sha256":candidate_source_sha256,
            }),
        ));
    }
    let mut base_without_source = base_manifest.clone();
    let mut candidate_without_source = candidate_manifest.clone();
    base_without_source.remove("src/lib.rs");
    candidate_without_source.remove("src/lib.rs");
    if base_without_source != candidate_without_source {
        return Err(tool_error(
            "workspace_recovery_candidate_diverged",
            "discard candidate changes outside src/lib.rs and reconstruct them from the admitted base",
            json!({}),
        ));
    }
    Ok(candidate_source_sha256 == expected_after_sha256)
}

/// Restore a candidate already validated against one unresolved intent to the
/// admitted pre-state so the normal baseline and exact write replay can run.
/// Call only after acquiring a fresh execution lease; validation is repeated
/// immediately before the atomic replacement.
pub fn restore_candidate_for_workspace_write_replay(
    base_dir: &Path,
    candidate_dir: &Path,
    expected_before_sha256: &str,
    expected_after_sha256: &str,
) -> Result<bool, AgentRuntimeErrorV1> {
    if !validate_candidate_for_workspace_write_recovery(
        base_dir,
        candidate_dir,
        expected_before_sha256,
        expected_after_sha256,
    )? {
        return Ok(false);
    }
    let base_dir = canonical_directory(base_dir)?;
    let candidate_dir = canonical_directory(candidate_dir)?;
    let base_manifest = snapshot_manifest(&base_dir)?;
    let base_source =
        String::from_utf8(read_regular_file(&base_dir, "src/lib.rs")?).map_err(|_| {
            tool_error(
                "workspace_recovery_base_mismatch",
                "recreate the admitted source as UTF-8 before replaying its write intent",
                json!({"path":"src/lib.rs"}),
            )
        })?;
    write_source(&candidate_dir, &base_source)?;
    if snapshot_manifest(&candidate_dir)? != base_manifest {
        return Err(tool_error(
            "workspace_recovery_candidate_diverged",
            "verify the candidate matches the admitted base after restoring its pending write",
            json!({}),
        ));
    }
    Ok(true)
}

fn spawn_capture(
    mut child: Child,
    timeout: Duration,
) -> Result<SandboxTestResult, AgentRuntimeErrorV1> {
    let started = Instant::now();
    let stdout = child.stdout.take().ok_or_else(|| {
        tool_error(
            "sandbox_pipe_missing",
            "ask the host to repair the local runner",
            json!({}),
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        tool_error(
            "sandbox_pipe_missing",
            "ask the host to repair the local runner",
            json!({}),
        )
    })?;
    let stdout_task = thread::spawn(move || capture_limited(stdout, MAX_CAPTURE_BYTES));
    let stderr_task = thread::spawn(move || capture_limited(stderr, MAX_CAPTURE_BYTES));

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                timed_out = true;
                kill_process_group(child.id());
                let _ = child.kill();
                break child.wait().ok();
            }
            Err(_) => {
                kill_process_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(tool_error(
                    "sandbox_wait_failed",
                    "inspect the host process state and retry with a fresh candidate",
                    json!({}),
                ));
            }
        }
    };
    let stdout = stdout_task.join().map_err(|_| {
        tool_error(
            "sandbox_capture_failed",
            "inspect the host runner",
            json!({"stream":"stdout"}),
        )
    })?;
    let stderr = stderr_task.join().map_err(|_| {
        tool_error(
            "sandbox_capture_failed",
            "inspect the host runner",
            json!({"stream":"stderr"}),
        )
    })?;
    let exit_code = status.and_then(|status| status.code());
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(SandboxTestResult {
        exit_code,
        timed_out,
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        output_truncated: stdout.truncated || stderr.truncated,
        elapsed_ms,
    })
}

fn capture_limited(mut reader: impl Read, max_bytes: usize) -> CapturedOutput {
    let mut bytes = Vec::with_capacity(max_bytes.min(8192));
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(size) => {
                let remaining = max_bytes.saturating_sub(bytes.len());
                let keep = remaining.min(size);
                bytes.extend_from_slice(&buffer[..keep]);
                truncated |= keep < size;
            }
        }
    }
    CapturedOutput { bytes, truncated }
}

fn kill_process_group(pid: u32) {
    // The sandbox process is its own process-group leader; kill descendants too
    // if the wall-clock deadline expires. The fixed executable and numeric group
    // ID are host-derived, never supplied by the model.
    let group = format!("-{pid}");
    let _ = Command::new("/usr/bin/kill")
        .env_clear()
        .args(["-KILL", "--", &group])
        .status();
}

fn canonical_directory(path: &Path) -> Result<PathBuf, AgentRuntimeErrorV1> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        tool_error(
            "sandbox_path_invalid",
            "ask the host to recreate the private workspace",
            json!({}),
        )
    })?;
    if !path.is_absolute() || !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(tool_error(
            "sandbox_path_invalid",
            "use an absolute private directory owned by the local host",
            json!({}),
        ));
    }
    fs::canonicalize(path).map_err(|_| {
        tool_error(
            "sandbox_path_invalid",
            "ask the host to recreate the private workspace",
            json!({}),
        )
    })
}

fn snapshot_manifest(root: &Path) -> Result<BTreeMap<String, String>, AgentRuntimeErrorV1> {
    fn visit(
        root: &Path,
        directory: &Path,
        manifest: &mut BTreeMap<String, String>,
        total_bytes: &mut u64,
    ) -> Result<(), AgentRuntimeErrorV1> {
        let mut entries = fs::read_dir(directory)
            .map_err(|_| {
                tool_error(
                    "candidate_snapshot_unreadable",
                    "ask the host to restore the candidate snapshot",
                    json!({}),
                )
            })?
            .map(|entry| {
                entry.map_err(|_| {
                    tool_error(
                        "candidate_snapshot_unreadable",
                        "ask the host to restore the candidate snapshot",
                        json!({}),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| {
                tool_error(
                    "candidate_snapshot_unreadable",
                    "ask the host to restore the candidate snapshot",
                    json!({}),
                )
            })?;
            if metadata.file_type().is_symlink() {
                return Err(tool_error(
                    "candidate_snapshot_symlink",
                    "recreate the snapshot from the protected Git commit without symbolic links",
                    json!({"path":path.strip_prefix(root).ok().and_then(Path::to_str)}),
                ));
            }
            if metadata.is_dir() {
                let relative_directory = path
                    .strip_prefix(root)
                    .ok()
                    .and_then(Path::to_str)
                    .ok_or_else(|| {
                        tool_error(
                            "candidate_snapshot_path_invalid",
                            "use UTF-8 relative source paths",
                            json!({}),
                        )
                    })?;
                if relative_directory == ".git" || relative_directory.starts_with(".git/") {
                    return Err(tool_error(
                        "candidate_snapshot_contains_git_metadata",
                        "create the sandbox source snapshot without a .git directory or credentials",
                        json!({}),
                    ));
                }
                visit(root, &path, manifest, total_bytes)?;
                continue;
            }
            if !metadata.is_file() || metadata.len() > MAX_SNAPSHOT_FILE_BYTES {
                return Err(tool_error(
                    "candidate_snapshot_file_invalid",
                    "use regular snapshot files no larger than 1 MiB",
                    json!({"path":path.strip_prefix(root).ok().and_then(Path::to_str)}),
                ));
            }
            if metadata.nlink() != 1 {
                return Err(tool_error(
                    "candidate_snapshot_hardlink",
                    "recreate the snapshot without hard-linked files so writes cannot affect paths outside the candidate",
                    json!({"path":path.strip_prefix(root).ok().and_then(Path::to_str)}),
                ));
            }
            *total_bytes = total_bytes.checked_add(metadata.len()).ok_or_else(|| {
                tool_error(
                    "candidate_snapshot_too_large",
                    "use a source snapshot under the 16 MiB local harness limit",
                    json!({}),
                )
            })?;
            if *total_bytes > MAX_SNAPSHOT_TOTAL_BYTES || manifest.len() >= MAX_SNAPSHOT_FILES {
                return Err(tool_error(
                    "candidate_snapshot_too_large",
                    "use at most 20,000 files and 16 MiB in this local harness",
                    json!({"max_files":MAX_SNAPSHOT_FILES,"max_total_bytes":MAX_SNAPSHOT_TOTAL_BYTES}),
                ));
            }
            let relative = path
                .strip_prefix(root)
                .ok()
                .and_then(Path::to_str)
                .ok_or_else(|| {
                    tool_error(
                        "candidate_snapshot_path_invalid",
                        "use UTF-8 relative source paths",
                        json!({}),
                    )
                })?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if relative == ".git" || relative.starts_with(".git/") {
                return Err(tool_error(
                    "candidate_snapshot_contains_git_metadata",
                    "create the sandbox source snapshot without a .git directory or credentials",
                    json!({}),
                ));
            }
            let bytes = fs::read(&path).map_err(|_| {
                tool_error(
                    "candidate_snapshot_unreadable",
                    "ask the host to restore the candidate snapshot",
                    json!({"path":relative}),
                )
            })?;
            let git_mode = if metadata.mode() & 0o111 == 0 {
                "100644"
            } else {
                "100755"
            };
            manifest.insert(relative, format!("{git_mode}:{}", sha256(&bytes)));
        }
        Ok(())
    }

    let root = canonical_directory(root)?;
    let mut manifest = BTreeMap::new();
    let mut total_bytes = 0_u64;
    visit(&root, &root, &mut manifest, &mut total_bytes)?;
    Ok(manifest)
}

fn materialize_verified_snapshot(
    source: &Path,
    expected_manifest: &BTreeMap<String, String>,
    destination: &Path,
) -> Result<bool, AgentRuntimeErrorV1> {
    fs::create_dir(destination).map_err(|_| {
        tool_error(
            "verification_snapshot_not_fresh",
            "provide a new private verifier target directory",
            json!({}),
        )
    })?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o700)).map_err(|_| {
        tool_error(
            "verification_snapshot_unavailable",
            "ask the host to create private verifier snapshots",
            json!({}),
        )
    })?;

    for (relative, expected) in expected_manifest {
        let source_file = source.join(relative);
        let metadata = fs::symlink_metadata(&source_file).map_err(|_| {
            tool_error(
                "verification_snapshot_changed",
                "recreate the source snapshot from its protected Git commit",
                json!({"path":relative}),
            )
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.len() > MAX_SNAPSHOT_FILE_BYTES
        {
            return Ok(false);
        }
        let bytes = fs::read(&source_file).map_err(|_| {
            tool_error(
                "verification_snapshot_changed",
                "recreate the source snapshot from its protected Git commit",
                json!({"path":relative}),
            )
        })?;
        let mode = if metadata.mode() & 0o111 == 0 {
            "100644"
        } else {
            "100755"
        };
        if expected != &format!("{mode}:{}", sha256(&bytes)) {
            return Ok(false);
        }

        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Ok(false);
        }
        let destination_file = destination.join(relative_path);
        let Some(parent) = destination_file.parent() else {
            return Ok(false);
        };
        fs::create_dir_all(parent).map_err(|_| {
            tool_error(
                "verification_snapshot_unavailable",
                "ask the host to create private verifier snapshots",
                json!({}),
            )
        })?;
        let file_mode = if mode == "100755" { 0o500 } else { 0o400 };
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&destination_file)
            .map_err(|_| {
                tool_error(
                    "verification_snapshot_unavailable",
                    "ask the host to create private verifier snapshots",
                    json!({}),
                )
            })?;
        output.write_all(&bytes).map_err(|_| {
            tool_error(
                "verification_snapshot_unavailable",
                "ask the host to create private verifier snapshots",
                json!({}),
            )
        })?;
        output.sync_all().map_err(|_| {
            tool_error(
                "verification_snapshot_unavailable",
                "ask the host to create private verifier snapshots",
                json!({}),
            )
        })?;
        drop(output);
        fs::set_permissions(&destination_file, fs::Permissions::from_mode(file_mode)).map_err(
            |_| {
                tool_error(
                    "verification_snapshot_unavailable",
                    "ask the host to create private verifier snapshots",
                    json!({}),
                )
            },
        )?;
    }

    let mut directories = Vec::<PathBuf>::new();
    for relative in expected_manifest.keys() {
        let mut current = destination.to_path_buf();
        for component in Path::new(relative)
            .parent()
            .into_iter()
            .flat_map(Path::components)
        {
            current.push(component.as_os_str());
            directories.push(current.clone());
        }
    }
    directories.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.cmp(right))
    });
    directories.dedup();
    for directory in directories {
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).map_err(|_| {
            tool_error(
                "verification_snapshot_unavailable",
                "ask the host to create private verifier snapshots",
                json!({}),
            )
        })?;
    }
    fs::set_permissions(destination, fs::Permissions::from_mode(0o500)).map_err(|_| {
        tool_error(
            "verification_snapshot_unavailable",
            "ask the host to create private verifier snapshots",
            json!({}),
        )
    })?;
    Ok(snapshot_manifest(destination)? == *expected_manifest)
}

pub fn run_sandbox_test(
    config: &SandboxConfig,
    timeout: Duration,
) -> Result<SandboxTestResult, AgentRuntimeErrorV1> {
    if !cfg!(target_os = "linux") {
        return Err(tool_error(
            "sandbox_platform_unsupported",
            "run this local acceptance harness on Linux with bubblewrap",
            json!({}),
        ));
    }
    let base = canonical_directory(&config.base_dir)?;
    let candidate = canonical_directory(&config.candidate_dir)?;
    let target = canonical_directory(&config.target_root)?;
    let sysroot = canonical_directory(&config.rust_sysroot)?;
    for (directory, label) in [(&base, "base"), (&candidate, "candidate")] {
        for relative in [
            "Cargo.toml",
            "Cargo.lock",
            "src/lib.rs",
            "tests/acceptance.rs",
        ] {
            let path = directory.join(relative);
            let metadata = fs::symlink_metadata(&path).map_err(|_| {
                tool_error(
                    "sandbox_snapshot_incomplete",
                    "restore the exact source snapshot",
                    json!({"directory":label,"path":relative}),
                )
            })?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_TEXT_BYTES as u64
            {
                return Err(tool_error(
                    "sandbox_snapshot_invalid",
                    "use regular mission files without symlinks and under the size limit",
                    json!({"directory":label,"path":relative}),
                ));
            }
        }
    }

    if paths_overlap(&base, &target)
        || paths_overlap(&candidate, &target)
        || paths_overlap(&sysroot, &target)
    {
        return Err(tool_error(
            "sandbox_paths_overlap",
            "keep the writable Cargo target outside both source snapshots and the read-only Rust toolchain",
            json!({}),
        ));
    }

    let mut command = Command::new(SYSTEMD_RUN);
    configure_systemd_connection_environment(&mut command);
    command.args(verifier_scope_arguments(timeout)).arg("--");
    command.arg("/usr/bin/bwrap");
    command
        .args([
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--assert-userns-disabled",
            "--die-with-parent",
            "--new-session",
            "--unshare-pid",
            "--dir",
            "/etc",
            "--ro-bind",
            "/etc/alternatives",
            "/etc/alternatives",
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/bin",
            "/bin",
            "--ro-bind",
        ])
        .arg(&sysroot)
        .args(["/toolchain", "--ro-bind"])
        .arg(&candidate)
        .args([
            "/candidate",
            "--size",
            &VERIFIER_TARGET_TMPFS_BYTES.to_string(),
            "--tmpfs",
            "/target",
            "--size",
            &VERIFIER_TMP_TMPFS_BYTES.to_string(),
            "--tmpfs",
            "/tmp",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--chdir",
            "/candidate",
            "--clearenv",
            "--setenv",
            "PATH",
            "/toolchain/bin:/usr/bin:/bin",
            "--setenv",
            "RUSTC",
            "/toolchain/bin/rustc",
            "--setenv",
            "CARGO_HOME",
            "/tmp/cargo-home",
            "--setenv",
            "CARGO_TARGET_DIR",
            "/target",
            "--setenv",
            "CARGO_NET_OFFLINE",
            "true",
            "--setenv",
            "CARGO_BUILD_JOBS",
            "2",
            "--setenv",
            "HOME",
            "/tmp",
            "/usr/bin/prlimit",
            "--cpu=120",
            "--as=4294967296",
            "--nproc=32",
            "--nofile=128",
            "--fsize=67108864",
            "--",
            "/toolchain/bin/cargo",
            "test",
            "--offline",
            "--locked",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = command.spawn().map_err(|_| {
        tool_error(
            "sandbox_unavailable",
            "install bubblewrap, prlimit, systemd-run with a writable user cgroup, and the pinned Rust toolchain for this local harness",
            json!({}),
        )
    })?;
    let result = spawn_capture(child, timeout)?;
    Ok(result)
}

pub struct CandidateToolHost {
    capabilities: RuntimeCapabilitiesV1,
    config: SandboxConfig,
    mission_context: Option<Value>,
    base_source_sha256: String,
    base_acceptance_sha256: String,
    base_manifest: BTreeMap<String, String>,
    baseline_red_observed: bool,
    post_write_green_sha256: Option<String>,
    receipts: Vec<ToolCallReceiptV1>,
    next_attempt: u64,
}

impl CandidateToolHost {
    pub fn new(
        capabilities: RuntimeCapabilitiesV1,
        config: SandboxConfig,
    ) -> Result<Self, AgentRuntimeErrorV1> {
        Self::new_with_mission_context(capabilities, config, None)
    }

    pub fn new_with_mission_context(
        capabilities: RuntimeCapabilitiesV1,
        mut config: SandboxConfig,
        mission_context: Option<Value>,
    ) -> Result<Self, AgentRuntimeErrorV1> {
        let context_tool_allowed = capabilities
            .tools
            .iter()
            .any(|tool| tool == "mission.context");
        if context_tool_allowed != mission_context.is_some() {
            return Err(tool_error(
                "mission_context_config_invalid",
                "advertise mission.context only when the host has a validated context packet",
                json!({"mission_context_allowed":context_tool_allowed,"context_available":mission_context.is_some()}),
            ));
        }
        if mission_context.as_ref().is_some_and(|context| {
            serde_json::to_vec(context)
                .map(|bytes| bytes.len() > 16 * 1024)
                .unwrap_or(true)
        }) {
            return Err(tool_error(
                "mission_context_too_large",
                "rebuild the cited context packet under the 16 KiB MCP response limit",
                json!({"max_bytes":16 * 1024}),
            ));
        }
        config.base_dir = canonical_directory(&config.base_dir)?;
        config.candidate_dir = canonical_directory(&config.candidate_dir)?;
        config.target_root = canonical_directory(&config.target_root)?;
        config.rust_sysroot = canonical_directory(&config.rust_sysroot)?;
        if paths_overlap(&config.base_dir, &config.candidate_dir)
            || paths_overlap(&config.base_dir, &config.target_root)
            || paths_overlap(&config.candidate_dir, &config.target_root)
            || paths_overlap(&config.rust_sysroot, &config.target_root)
        {
            return Err(tool_error(
                "sandbox_paths_overlap",
                "use separate protected-base, candidate, and build-target directories",
                json!({}),
            ));
        }
        let base_manifest = snapshot_manifest(&config.base_dir)?;
        let candidate_manifest = snapshot_manifest(&config.candidate_dir)?;
        if base_manifest != candidate_manifest {
            return Err(tool_error(
                "candidate_not_at_base",
                "create a private candidate from the exact protected base before starting the agent",
                json!({}),
            ));
        }
        let base_source = read_regular_file(&config.base_dir, "src/lib.rs")?;
        let base_acceptance = read_regular_file(&config.base_dir, "tests/acceptance.rs")?;
        let candidate_source = read_regular_file(&config.candidate_dir, "src/lib.rs")?;
        let candidate_acceptance = read_regular_file(&config.candidate_dir, "tests/acceptance.rs")?;
        if sha256(&base_source) != sha256(&candidate_source)
            || sha256(&base_acceptance) != sha256(&candidate_acceptance)
        {
            return Err(tool_error(
                "candidate_not_at_base",
                "create a private candidate from the exact protected base before starting the agent",
                json!({}),
            ));
        }
        Ok(Self {
            capabilities,
            config,
            mission_context,
            base_source_sha256: sha256(&base_source),
            base_acceptance_sha256: sha256(&base_acceptance),
            base_manifest,
            baseline_red_observed: false,
            post_write_green_sha256: None,
            receipts: Vec::new(),
            next_attempt: 0,
        })
    }

    /// Digest of the base snapshot captured at construction.
    pub fn base_manifest_sha256(&self) -> String {
        manifest_sha256(&self.base_manifest)
    }

    /// Host-only candidate directory used by the journaled writer's no-follow
    /// read callback. The model never receives this path.
    pub fn candidate_source_dir(&self) -> &Path {
        &self.config.candidate_dir
    }

    /// Check the fixed tool capability and host preconditions before the
    /// operation journal records an intent. The writer repeats all checks
    /// after the durable intent and rechecks the source digest at the effect.
    pub fn preflight_workspace_write(&self, input: &Value) -> Result<(), AgentRuntimeErrorV1> {
        let max_calls =
            usize::try_from(self.capabilities.budget.max_tool_calls).unwrap_or(usize::MAX);
        if self.receipts.len() >= max_calls {
            return Err(tool_error(
                "tool_budget_exhausted",
                "request a new host-approved execution with a larger tool budget",
                json!({"max_tool_calls":max_calls}),
            ));
        }
        if !self
            .capabilities
            .tools
            .iter()
            .any(|allowed| allowed == "workspace.write_text")
        {
            return Err(tool_error(
                "tool_capability_denied",
                "use a workspace write capability declared in the host-issued execution request",
                json!({"tool_id":"workspace.write_text"}),
            ));
        }
        let request =
            parse_model_tool("workspace.write_text", input).map_err(|refusal| match refusal {
                ToolRefusal::PathNotAllowed => tool_error(
                    "tool_path_denied",
                    "write only a path in the host-issued write set",
                    json!({"tool_id":"workspace.write_text"}),
                ),
                ToolRefusal::InvalidInput | ToolRefusal::UnknownTool => tool_error(
                    "tool_input_invalid",
                    "repair the write arguments to match the exact host schema",
                    json!({"tool_id":"workspace.write_text"}),
                ),
            })?;
        let ToolRequest::WriteText { path, .. } = request else {
            return Err(tool_error(
                "tool_input_invalid",
                "repair the write arguments to match the exact host schema",
                json!({"tool_id":"workspace.write_text"}),
            ));
        };
        if !self
            .capabilities
            .write_paths
            .iter()
            .any(|allowed| allowed == &path)
        {
            return Err(tool_error(
                "tool_path_denied",
                "write only a path in the host-issued write set",
                json!({"path":path}),
            ));
        }
        if !self.baseline_red_observed {
            return Err(tool_error(
                "baseline_not_verified",
                "run cargo.test on the unchanged base and inspect the protected boundary failure before editing",
                json!({}),
            ));
        }
        let acceptance = read_regular_file(&self.config.candidate_dir, "tests/acceptance.rs")?;
        if sha256(&acceptance) != self.base_acceptance_sha256 {
            return Err(tool_error(
                "protected_acceptance_changed",
                "discard the candidate and restart from the protected base; acceptance tests cannot be changed",
                json!({}),
            ));
        }
        Ok(())
    }

    /// Apply the normal capability, baseline, and protected-test checks while
    /// also requiring the candidate source to still have the journaled
    /// pre-state immediately before the atomic replacement.
    pub fn invoke_workspace_write_if_sha256(
        &mut self,
        input: Value,
        expected_before_sha256: &str,
    ) -> Result<Value, AgentRuntimeErrorV1> {
        let started = Instant::now();
        let input_sha256 = sha256(&serde_json::to_vec(&input).unwrap_or_default());
        let result = self.invoke_inner_with_expected_source(
            "workspace.write_text",
            input,
            Some(expected_before_sha256),
        );
        let safe_output = match &result {
            Ok(value) => serde_json::to_vec(value).unwrap_or_default(),
            Err(error) => serde_json::to_vec(&json!({"code":error.code,"action":error.action}))
                .unwrap_or_default(),
        };
        self.receipts.push(ToolCallReceiptV1 {
            tool_id: "workspace.write_text".to_owned(),
            input_sha256,
            output_sha256: sha256(&safe_output),
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            success: result.is_ok(),
            error_code: result.as_ref().err().map(|error| error.code.clone()),
        });
        result
    }

    fn invoke_inner(&mut self, tool_id: &str, input: Value) -> Result<Value, AgentRuntimeErrorV1> {
        self.invoke_inner_with_expected_source(tool_id, input, None)
    }

    fn invoke_inner_with_expected_source(
        &mut self,
        tool_id: &str,
        input: Value,
        expected_source_sha256: Option<&str>,
    ) -> Result<Value, AgentRuntimeErrorV1> {
        let max_calls =
            usize::try_from(self.capabilities.budget.max_tool_calls).unwrap_or(usize::MAX);
        if self.receipts.len() >= max_calls {
            return Err(tool_error(
                "tool_budget_exhausted",
                "request a new host-approved execution with a larger tool budget",
                json!({"max_tool_calls":max_calls}),
            ));
        }
        if !self
            .capabilities
            .tools
            .iter()
            .any(|allowed| allowed == tool_id)
        {
            return Err(tool_error(
                "tool_capability_denied",
                "use a tool declared in the host-issued execution request",
                json!({"tool_id":tool_id}),
            ));
        }

        // `workspace.diff` is reserved for host-side finalization by the adapter.
        // It is accepted here only under its exact spelling and is never returned
        // by `parse_model_tool` as a model-facing capability.
        if tool_id == "workspace.diff" {
            if input != json!({}) {
                return Err(tool_error(
                    "tool_input_invalid",
                    "the host-only diff call takes an empty object",
                    json!({}),
                ));
            }
            return self.finalize_diff();
        }

        match parse_model_tool(tool_id, &input).map_err(|refusal| match refusal {
            ToolRefusal::UnknownTool => tool_error(
                "tool_unknown",
                "use one of the advertised mission tools",
                json!({"tool_id":tool_id}),
            ),
            ToolRefusal::InvalidInput => tool_error(
                "tool_input_invalid",
                "repair the tool arguments to match its exact schema",
                json!({"tool_id":tool_id}),
            ),
            ToolRefusal::PathNotAllowed => tool_error(
                "tool_path_denied",
                "use only the exact paths granted by the execution request",
                json!({"tool_id":tool_id}),
            ),
        })? {
            ToolRequest::MissionContext => self.mission_context.clone().ok_or_else(|| {
                tool_error(
                    "mission_context_unavailable",
                    "request a host-issued execution with a validated context packet",
                    json!({}),
                )
            }),
            ToolRequest::ReadText { path } => {
                if !self
                    .capabilities
                    .read_paths
                    .iter()
                    .any(|allowed| allowed == &path)
                {
                    return Err(tool_error(
                        "tool_path_denied",
                        "read only a path in the host-issued read set",
                        json!({"path":path}),
                    ));
                }
                let bytes = read_regular_file(&self.config.candidate_dir, &path)?;
                let content = String::from_utf8(bytes).map_err(|_| {
                    tool_error(
                        "tool_file_not_utf8",
                        "choose a UTF-8 source or ask the host to provide a text projection",
                        json!({"path":path}),
                    )
                })?;
                Ok(json!({"content":content,"path":path}))
            }
            ToolRequest::WriteText { path, content } => {
                if !self
                    .capabilities
                    .write_paths
                    .iter()
                    .any(|allowed| allowed == &path)
                {
                    return Err(tool_error(
                        "tool_path_denied",
                        "write only a path in the host-issued write set",
                        json!({"path":path}),
                    ));
                }
                if !self.baseline_red_observed {
                    return Err(tool_error(
                        "baseline_not_verified",
                        "run cargo.test on the unchanged base and inspect the protected boundary failure before editing",
                        json!({}),
                    ));
                }
                let acceptance =
                    read_regular_file(&self.config.candidate_dir, "tests/acceptance.rs")?;
                if sha256(&acceptance) != self.base_acceptance_sha256 {
                    return Err(tool_error(
                        "protected_acceptance_changed",
                        "discard the candidate and restart from the protected base; acceptance tests cannot be changed",
                        json!({}),
                    ));
                }
                if let Some(expected) = expected_source_sha256 {
                    if expected.len() != 64
                        || expected.bytes().any(|byte| !byte.is_ascii_hexdigit())
                        || expected != expected.to_ascii_lowercase()
                    {
                        return Err(tool_error(
                            "tool_operation_source_hash_invalid",
                            "retry with the lowercase SHA-256 recorded in the write intent",
                            json!({"path":"src/lib.rs"}),
                        ));
                    }
                    let observed = candidate_source_sha256(&self.config.candidate_dir)?;
                    if observed != expected {
                        return Err(tool_error(
                            "tool_operation_effect_diverged",
                            "inspect the candidate source and retry only after reconciling it with the durable write intent",
                            json!({"path":"src/lib.rs","expected_before_sha256":expected,"observed_sha256":observed}),
                        ));
                    }
                }
                write_source(&self.config.candidate_dir, &content)?;
                self.post_write_green_sha256 = None;
                Ok(json!({"written":true,"path":"src/lib.rs","sha256":sha256(content.as_bytes())}))
            }
            ToolRequest::CargoTest => {
                self.next_attempt = self.next_attempt.saturating_add(1);
                let source = read_regular_file(&self.config.candidate_dir, "src/lib.rs")?;
                let acceptance =
                    read_regular_file(&self.config.candidate_dir, "tests/acceptance.rs")?;
                let source_sha256 = sha256(&source);
                if !self.baseline_red_observed && source_sha256 != self.base_source_sha256 {
                    return Err(tool_error(
                        "baseline_changed_before_test",
                        "restart from the protected base; the first test must run before any edit",
                        json!({}),
                    ));
                }
                if sha256(&acceptance) != self.base_acceptance_sha256 {
                    return Err(tool_error(
                        "protected_acceptance_changed",
                        "discard the candidate and restart from the protected base; acceptance tests cannot be changed",
                        json!({}),
                    ));
                }
                let before_test = source_sha256;
                let mut config = self.config.clone();
                config.target_root = self
                    .config
                    .target_root
                    .join(format!("cargo-test-{}", self.next_attempt));
                fs::create_dir_all(&config.target_root).map_err(|_| {
                    tool_error(
                        "sandbox_target_unavailable",
                        "ask the host for a fresh private build target directory",
                        json!({}),
                    )
                })?;
                let result = run_sandbox_test(&config, TEST_TIMEOUT)?;
                let output = format!("{}\n{}", result.stdout, result.stderr);
                let expected_red = result.exit_code == Some(101)
                    && !result.timed_out
                    && !result.output_truncated
                    && output.contains(EXPECTED_FAILURE)
                    && output.contains("left: -2")
                    && output.contains("right: 10")
                    && output.contains("1 passed; 1 failed");
                if !self.baseline_red_observed
                    && before_test == self.base_source_sha256
                    && expected_red
                {
                    self.baseline_red_observed = true;
                }
                let current_source_sha256 = sha256(&read_regular_file(
                    &self.config.candidate_dir,
                    "src/lib.rs",
                )?);
                let approved_candidate =
                    approved_mission_candidate(&self.base_source_sha256, &source);
                let verified_green = result.exit_code == Some(0)
                    && !result.timed_out
                    && !result.output_truncated
                    && output.contains(EXPECTED_FAILURE)
                    && output.contains("values_inside_the_range_are_unchanged")
                    && output.contains(EXPECTED_GREEN_SUMMARY)
                    && current_source_sha256 != self.base_source_sha256
                    && current_source_sha256 == sha256(&source)
                    && approved_candidate;
                if self.baseline_red_observed && verified_green {
                    self.post_write_green_sha256 = Some(current_source_sha256);
                }
                Ok(json!({
                    "exit_code":result.exit_code,
                    "timed_out":result.timed_out,
                    "elapsed_ms":result.elapsed_ms,
                    "output_truncated":result.output_truncated,
                    "stdout":result.stdout,
                    "stderr":result.stderr,
                    "expected_baseline_failure":expected_red,
                    "approved_candidate_source":approved_candidate,
                    "verified_acceptance_pass":verified_green,
                }))
            }
        }
    }

    fn finalize_diff(&self) -> Result<Value, AgentRuntimeErrorV1> {
        if !self.baseline_red_observed {
            return Err(tool_error(
                "baseline_not_verified",
                "the unchanged protected base must show its expected acceptance failure",
                json!({}),
            ));
        }
        let source = read_regular_file(&self.config.candidate_dir, "src/lib.rs")?;
        let source_sha256 = sha256(&source);
        if source_sha256 == self.base_source_sha256 {
            return Err(tool_error(
                "candidate_diff_empty",
                "make a source change that addresses the verified acceptance failure",
                json!({}),
            ));
        }
        let source = read_regular_file(&self.config.candidate_dir, "src/lib.rs")?;
        if !approved_mission_candidate(&self.base_source_sha256, &source) {
            return Err(tool_error(
                "candidate_source_contract_violation",
                "discard this task-specific candidate and use the reviewed fix for the pinned clamp requirement",
                json!({"current_source_sha256":source_sha256}),
            ));
        }
        if self.post_write_green_sha256.as_deref() != Some(source_sha256.as_str()) {
            return Err(tool_error(
                "candidate_verification_stale",
                "run cargo.test successfully again on the exact current source before claiming a candidate",
                json!({"current_source_sha256":source_sha256}),
            ));
        }
        let acceptance = read_regular_file(&self.config.candidate_dir, "tests/acceptance.rs")?;
        if sha256(&acceptance) != self.base_acceptance_sha256 {
            return Err(tool_error(
                "protected_acceptance_changed",
                "discard the candidate and restart from the protected base",
                json!({}),
            ));
        }
        let current_manifest = snapshot_manifest(&self.config.candidate_dir)?;
        let changed_paths: Vec<String> = self
            .base_manifest
            .keys()
            .chain(current_manifest.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|path| self.base_manifest.get(*path) != current_manifest.get(*path))
            .cloned()
            .collect();
        if changed_paths != ["src/lib.rs"] {
            return Err(tool_error(
                "candidate_write_set_violation",
                "discard this candidate; only src/lib.rs may differ from the protected base",
                json!({"changed_paths":changed_paths}),
            ));
        }
        let diff = make_stable_source_diff(&self.config.base_dir, &self.config.candidate_dir)?;
        Ok(json!({"changed_paths":["src/lib.rs"],"diff_sha256":sha256(&diff)}))
    }
}

impl AgentToolPortV1 for CandidateToolHost {
    fn invoke(&mut self, tool_id: &str, input: Value) -> Result<Value, AgentRuntimeErrorV1> {
        let started = Instant::now();
        let input_sha256 = sha256(&serde_json::to_vec(&input).unwrap_or_default());
        let result = self.invoke_inner(tool_id, input);
        let safe_output = match &result {
            Ok(value) => serde_json::to_vec(value).unwrap_or_default(),
            Err(error) => serde_json::to_vec(&json!({"code":error.code,"action":error.action}))
                .unwrap_or_default(),
        };
        self.receipts.push(ToolCallReceiptV1 {
            tool_id: tool_id.to_owned(),
            input_sha256,
            output_sha256: sha256(&safe_output),
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            success: result.is_ok(),
            error_code: result.as_ref().err().map(|error| error.code.clone()),
        });
        result
    }

    fn receipts(&self) -> &[ToolCallReceiptV1] {
        &self.receipts
    }
}

fn make_stable_source_diff(base: &Path, candidate: &Path) -> Result<Vec<u8>, AgentRuntimeErrorV1> {
    let output = Command::new("/usr/bin/diff")
        .env_clear()
        .args(["-u", "--label", "a/src/lib.rs", "--label", "b/src/lib.rs"])
        .arg(base.join("src/lib.rs"))
        .arg(candidate.join("src/lib.rs"))
        .output()
        .map_err(|_| {
            tool_error(
                "candidate_diff_unavailable",
                "install diffutils in the local host environment",
                json!({}),
            )
        })?;
    if output.status.code() != Some(1) || output.stdout.is_empty() {
        return Err(tool_error(
            "candidate_diff_invalid",
            "inspect the source snapshot and create exactly one non-empty source diff",
            json!({"exit_code":output.status.code()}),
        ));
    }
    Ok(output.stdout)
}

pub fn verify_pair(
    base_dir: &Path,
    candidate_dir: &Path,
    target_root: &Path,
    rust_sysroot: &Path,
    trusted_git_repository: &Path,
    base_commit: &str,
    candidate_commit: &str,
) -> Result<Value, AgentRuntimeErrorV1> {
    if base_commit.len() != 40
        || !base_commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        || candidate_commit.len() != 40
        || !candidate_commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(tool_error(
            "verification_commit_invalid",
            "supply the full 40-character Git commit SHAs of the protected base and candidate head",
            json!({}),
        ));
    }
    let base_dir = canonical_directory(base_dir)?;
    let candidate_dir = canonical_directory(candidate_dir)?;
    let target_root = canonical_directory(target_root)?;
    let rust_sysroot = canonical_directory(rust_sysroot)?;
    let trusted_git_repository = canonical_directory(trusted_git_repository)?;
    if paths_overlap(&base_dir, &candidate_dir)
        || paths_overlap(&base_dir, &target_root)
        || paths_overlap(&candidate_dir, &target_root)
        || paths_overlap(&rust_sysroot, &target_root)
        || paths_overlap(&rust_sysroot, &trusted_git_repository)
        || paths_overlap(&trusted_git_repository, &base_dir)
        || paths_overlap(&trusted_git_repository, &candidate_dir)
        || paths_overlap(&trusted_git_repository, &target_root)
    {
        return Err(tool_error(
            "verification_paths_overlap",
            "use disjoint protected base, candidate, writable target, and Rust toolchain directories",
            json!({}),
        ));
    }
    let base_manifest = snapshot_manifest(&base_dir)?;
    let candidate_manifest = snapshot_manifest(&candidate_dir)?;
    if !git_commit_matches_snapshot(
        &trusted_git_repository,
        base_commit,
        &base_dir,
        &base_manifest,
    )? || !git_commit_matches_snapshot(
        &trusted_git_repository,
        candidate_commit,
        &candidate_dir,
        &candidate_manifest,
    )? {
        return Ok(json!({
            "schema_version":1,
            "verdict":"inconclusive",
            "reason":"snapshot_does_not_match_git_commit",
            "source_commit":base_commit.to_ascii_lowercase(),
            "candidate_commit":candidate_commit.to_ascii_lowercase(),
        }));
    }
    let base_snapshot = target_root.join("verified-base-snapshot");
    let candidate_snapshot = target_root.join("verified-candidate-snapshot");
    let base_snapshot_pinned =
        materialize_verified_snapshot(&base_dir, &base_manifest, &base_snapshot)?;
    let candidate_snapshot_pinned =
        materialize_verified_snapshot(&candidate_dir, &candidate_manifest, &candidate_snapshot)?;
    if !base_snapshot_pinned || !candidate_snapshot_pinned {
        return Ok(json!({
            "schema_version":1,
            "verdict":"inconclusive",
            "reason":"snapshot_changed_while_pinning",
            "source_commit":base_commit.to_ascii_lowercase(),
            "candidate_commit":candidate_commit.to_ascii_lowercase(),
        }));
    }
    let base_requirement =
        read_verification_binding_file(&base_snapshot, "requirements/MISSION-012-CLAMP-001.json")?;
    let candidate_requirement = read_verification_binding_file(
        &candidate_snapshot,
        "requirements/MISSION-012-CLAMP-001.json",
    )?;
    let base_active = read_verification_binding_file(&base_snapshot, "policy/active.json")?;
    let candidate_active =
        read_verification_binding_file(&candidate_snapshot, "policy/active.json")?;
    let base_policy =
        read_verification_binding_file(&base_snapshot, "policy/MISSION-012-CLAMP-001-v4.json")?;
    let candidate_policy = read_verification_binding_file(
        &candidate_snapshot,
        "policy/MISSION-012-CLAMP-001-v4.json",
    )?;
    let requirement_document: Value =
        serde_json::from_slice(&base_requirement).unwrap_or(Value::Null);
    let active_document: Value = serde_json::from_slice(&base_active).unwrap_or(Value::Null);
    let policy_document: Value = serde_json::from_slice(&base_policy).unwrap_or(Value::Null);
    let requirement_actual_sha256 = sha256(&base_requirement);
    let policy_actual_sha256 = sha256(&base_policy);
    let requirement_bound = requirement_actual_sha256
        .eq_ignore_ascii_case(EXPECTED_REQUIREMENT_SHA256)
        && requirement_document["schema_version"] == 2
        && requirement_document["id"] == "MISSION-012-CLAMP-001"
        && requirement_document["revision"] == 1
        && requirement_document["source_issue_number"] == 1;
    let policy_bound = policy_actual_sha256.eq_ignore_ascii_case(EXPECTED_POLICY_SHA256)
        && active_document["schema_version"] == 1
        && active_document["policy_id"] == "MISSION-012-CLAMP-001-POLICY"
        && active_document["revision"] == 4
        && active_document["policy_path"] == "policy/MISSION-012-CLAMP-001-v4.json"
        && active_document["policy_sha256"] == policy_actual_sha256
        && policy_document["schema_version"] == 1
        && policy_document["id"] == "MISSION-012-CLAMP-001-POLICY"
        && policy_document["revision"] == 4
        && policy_document["source_issue_number"] == 1
        && policy_document["requirement_path"] == "requirements/MISSION-012-CLAMP-001.json"
        && policy_document["requirement_sha256"] == requirement_actual_sha256
        && policy_document["verifier_identity"] == TRUSTED_VERIFIER_ID
        && policy_document["verification"]["command"] == json!(VERIFICATION_COMMAND);
    let bindings_unchanged = candidate_requirement == base_requirement
        && candidate_active == base_active
        && candidate_policy == base_policy;
    if !requirement_bound || !policy_bound || !bindings_unchanged {
        return Ok(json!({
            "schema_version":1,
            "verdict":"inconclusive",
            "reason":"requirement_or_policy_binding_invalid_or_stale",
            "source_commit":base_commit.to_ascii_lowercase(),
            "candidate_commit":candidate_commit.to_ascii_lowercase(),
            "requirement_sha256":requirement_actual_sha256,
            "policy_sha256":policy_actual_sha256,
            "active_pointer_sha256":sha256(&base_active),
            "verifier_id":TRUSTED_VERIFIER_ID,
            "base_tree_sha256":manifest_sha256(&base_manifest),
            "candidate_tree_sha256":manifest_sha256(&candidate_manifest),
        }));
    }
    let changed_paths: Vec<String> = base_manifest
        .keys()
        .chain(candidate_manifest.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|path| base_manifest.get(*path) != candidate_manifest.get(*path))
        .cloned()
        .collect();
    let base_source = read_regular_file(&base_snapshot, "src/lib.rs")?;
    let candidate_source = read_regular_file(&candidate_snapshot, "src/lib.rs")?;
    let base_acceptance = read_regular_file(&base_snapshot, "tests/acceptance.rs")?;
    let candidate_acceptance = read_regular_file(&candidate_snapshot, "tests/acceptance.rs")?;
    let base_source_sha256 = sha256(&base_source);
    let candidate_source_sha256 = sha256(&candidate_source);
    let acceptance_sha256 = sha256(&base_acceptance);
    let bound_evidence = json!({
        "requirement_id":requirement_document["id"],
        "requirement_revision":requirement_document["revision"],
        "requirement_sha256":requirement_actual_sha256,
        "policy_id":policy_document["id"],
        "policy_revision":policy_document["revision"],
        "policy_sha256":policy_actual_sha256,
        "active_pointer_sha256":sha256(&base_active),
        "verifier_id":TRUSTED_VERIFIER_ID,
        "git_tree_binding_verified":true,
        "pinned_snapshots_executed":true,
        "base_tree_sha256":manifest_sha256(&base_manifest),
        "candidate_tree_sha256":manifest_sha256(&candidate_manifest),
    });
    if base_manifest == candidate_manifest || changed_paths != ["src/lib.rs"] {
        return Ok(json!({
            "schema_version":1,
            "verdict":"fail",
            "reason":"candidate_write_set_violation",
            "source_commit":base_commit.to_ascii_lowercase(),
            "candidate_commit":candidate_commit.to_ascii_lowercase(),
            "binding":bound_evidence,
            "changed_paths":changed_paths,
            "acceptance_sha256":acceptance_sha256,
            "command":VERIFICATION_COMMAND,
            "environment":"bubblewrap user, mount, PID and network namespaces; nested user namespaces disabled and asserted; candidate read-only; fresh writable Cargo target; host home and credentials absent",
        }));
    }
    if sha256(&candidate_acceptance) != acceptance_sha256 {
        return Ok(json!({
            "schema_version":1,
            "verdict":"fail",
            "reason":"protected_acceptance_changed",
            "source_commit":base_commit.to_ascii_lowercase(),
            "candidate_commit":candidate_commit.to_ascii_lowercase(),
            "binding":bound_evidence,
            "changed_paths":changed_paths,
            "acceptance_sha256":acceptance_sha256,
        }));
    }
    if !approved_mission_candidate(&base_source_sha256, &candidate_source) {
        return Ok(json!({
            "schema_version":1,
            "verdict":"fail",
            "reason":"candidate_source_not_allowed_by_task_profile",
            "source_commit":base_commit.to_ascii_lowercase(),
            "candidate_commit":candidate_commit.to_ascii_lowercase(),
            "binding":bound_evidence,
            "changed_paths":changed_paths,
            "base_source_sha256":base_source_sha256,
            "candidate_source_sha256":candidate_source_sha256,
            "acceptance_sha256":acceptance_sha256,
        }));
    }

    let base_target = target_root.join("base-run");
    let candidate_target = target_root.join("candidate-run");
    fs::create_dir(&base_target).map_err(|_| {
        tool_error(
            "verification_target_not_fresh",
            "provide a new empty private verifier target directory",
            json!({"which":"base"}),
        )
    })?;
    fs::create_dir(&candidate_target).map_err(|_| {
        tool_error(
            "verification_target_not_fresh",
            "provide a new empty private verifier target directory",
            json!({"which":"candidate"}),
        )
    })?;
    let base_result = run_sandbox_test(
        &SandboxConfig::new(&base_snapshot, &base_snapshot, &base_target, &rust_sysroot),
        TEST_TIMEOUT,
    )?;
    let base_output = format!("{}\n{}", base_result.stdout, base_result.stderr);
    let expected_red = base_result.exit_code == Some(101)
        && !base_result.timed_out
        && !base_result.output_truncated
        && base_output.contains(EXPECTED_FAILURE)
        && base_output.contains("left: -2")
        && base_output.contains("right: 10")
        && base_output.contains("1 passed; 1 failed");

    let candidate_result = run_sandbox_test(
        &SandboxConfig::new(
            &base_snapshot,
            &candidate_snapshot,
            &candidate_target,
            &rust_sysroot,
        ),
        TEST_TIMEOUT,
    )?;
    let candidate_output = format!("{}\n{}", candidate_result.stdout, candidate_result.stderr);
    let candidate_green = candidate_result.exit_code == Some(0)
        && !candidate_result.timed_out
        && !candidate_result.output_truncated
        && candidate_output.contains(EXPECTED_FAILURE)
        && candidate_output.contains("values_inside_the_range_are_unchanged")
        && candidate_output.contains(EXPECTED_GREEN_SUMMARY);
    let post_base_manifest = snapshot_manifest(&base_snapshot)?;
    let post_candidate_manifest = snapshot_manifest(&candidate_snapshot)?;
    let post_test_source_sha256 = sha256(&read_regular_file(&candidate_snapshot, "src/lib.rs")?);
    let post_test_acceptance_sha256 = sha256(&read_regular_file(
        &candidate_snapshot,
        "tests/acceptance.rs",
    )?);
    let unchanged_during_test = post_base_manifest == base_manifest
        && post_candidate_manifest == candidate_manifest
        && post_test_source_sha256 == candidate_source_sha256
        && post_test_acceptance_sha256 == acceptance_sha256;

    let candidate_inconclusive =
        candidate_result.timed_out || candidate_result.output_truncated || !unchanged_during_test;
    let verdict = if !expected_red
        || base_result.timed_out
        || base_result.output_truncated
        || candidate_inconclusive
    {
        "inconclusive"
    } else if candidate_green && unchanged_during_test {
        "pass"
    } else {
        "fail"
    };
    let reason = if !expected_red {
        Some("base_did_not_show_the_pinned_acceptance_failure")
    } else if candidate_result.timed_out {
        Some("candidate_verification_timed_out")
    } else if candidate_result.output_truncated {
        Some("candidate_verification_output_truncated")
    } else if !unchanged_during_test {
        Some("pinned_snapshot_changed_during_verification")
    } else if !candidate_green {
        Some("candidate_acceptance_failed")
    } else {
        None
    };
    Ok(json!({
        "schema_version":1,
        "verdict":verdict,
        "reason":reason,
        "source_commit":base_commit.to_ascii_lowercase(),
        "candidate_commit":candidate_commit.to_ascii_lowercase(),
        "binding":bound_evidence,
        "candidate_source_sha256":candidate_source_sha256,
        "base_source_sha256":base_source_sha256,
        "acceptance_sha256":acceptance_sha256,
        "changed_paths":changed_paths,
        "base":{
            "exit_code":base_result.exit_code,
            "timed_out":base_result.timed_out,
            "elapsed_ms":base_result.elapsed_ms,
            "stdout":base_result.stdout,
            "stderr":base_result.stderr,
            "output_truncated":base_result.output_truncated,
            "expected_acceptance_failure":expected_red,
        },
        "candidate":{
            "exit_code":candidate_result.exit_code,
            "timed_out":candidate_result.timed_out,
            "elapsed_ms":candidate_result.elapsed_ms,
            "stdout":candidate_result.stdout,
            "stderr":candidate_result.stderr,
            "output_truncated":candidate_result.output_truncated,
            "acceptance_pass":candidate_green,
            "unchanged_during_verification":unchanged_during_test,
        },
        "command":VERIFICATION_COMMAND,
        "environment":"bubblewrap user, mount, PID and network namespaces; nested user namespaces disabled and asserted; pinned base and candidate snapshots read-only; fresh writable Cargo target; host home and credentials absent",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_forge_core::agent_runtime::{RuntimeBudgetV1, RuntimeCapabilitiesV1};
    use agent_forge_core::{
        ExpectedBinding, GateError, Outcome, ProofEvidence,
        forgejo::{
            BaseBranchScope, MergePreflightContextV1, MergePreflightError, PullRequestSnapshotV1,
            validate_merge_preflight,
        },
        sign_proof,
    };
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    const RECOVERY_FIXED_SOURCE: &str = "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        maximum\n    } else {\n        value\n    }\n}\n";
    const REQUIREMENT_FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/mission-012/requirements/MISSION-012-CLAMP-001.json");
    const POLICY_V1_FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v1.json");
    const POLICY_V2_FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v2.json");
    const POLICY_V3_FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v3.json");
    const POLICY_V4_FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/mission-012/policy/MISSION-012-CLAMP-001-v4.json");
    const ACTIVE_POLICY_FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/mission-012/policy/active.json");

    #[test]
    fn base_binding_requires_a_full_lowercase_commit_and_a_plain_branch() {
        let missing = Path::new("/nonexistent/mission-012-trusted-repo");
        for commit in [
            "",
            "2b230ec",
            "2B230ECDA8D57DABBB36EEE1399EFBE4C8C875AD",
            "2b230ecda8d57dabbb36eee1399efbe4c8c875ag",
            "2b230ecda8d57dabbb36eee1399efbe4c8c875ad2b230ecda8d57dabbb36eee1",
            "HEAD",
        ] {
            let error =
                bind_workspace_to_admitted_commit(missing, "main", commit, missing, missing)
                    .unwrap_err();
            assert_eq!(error.code, "workspace_base_commit_invalid", "{commit:?}");
        }
        let commit = "2b230ecda8d57dabbb36eee1399efbe4c8c875ad";
        for branch in ["", "-main", "main branch", "main\n"] {
            let error =
                bind_workspace_to_admitted_commit(missing, branch, commit, missing, missing)
                    .unwrap_err();
            assert_eq!(error.code, "workspace_base_branch_invalid", "{branch:?}");
        }
        let error = bind_workspace_to_admitted_commit(missing, "main", commit, missing, missing)
            .unwrap_err();
        assert_eq!(error.code, "trusted_git_repository_unavailable");
    }

    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "mission-012-toolhost-{}-{time}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            fs::create_dir_all(root.join("base/src")).unwrap();
            fs::create_dir_all(root.join("base/tests")).unwrap();
            fs::create_dir_all(root.join("base/requirements")).unwrap();
            fs::create_dir_all(root.join("base/policy")).unwrap();
            fs::create_dir_all(root.join("candidate/src")).unwrap();
            fs::create_dir_all(root.join("candidate/tests")).unwrap();
            fs::create_dir_all(root.join("candidate/requirements")).unwrap();
            fs::create_dir_all(root.join("candidate/policy")).unwrap();
            fs::create_dir(root.join("target")).unwrap();
            for snapshot in ["base", "candidate"] {
                fs::write(root.join(snapshot).join("Cargo.toml"), "[package]\nname = \"mission012-clamp\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
                fs::write(root.join(snapshot).join("Cargo.lock"), "# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"mission012-clamp\"\nversion = \"0.1.0\"\n").unwrap();
                fs::write(root.join(snapshot).join("tests/acceptance.rs"), "use mission012_clamp::clamp_to_range;\n\n#[test]\nfn values_outside_the_range_clamp_to_the_nearest_boundary() {\n    assert_eq!(clamp_to_range(-5, -2, 10), -2);\n    assert_eq!(clamp_to_range(15, -2, 10), 10);\n}\n\n#[test]\nfn values_inside_the_range_are_unchanged() {\n    assert_eq!(clamp_to_range(4, -2, 10), 4);\n}\n").unwrap();
            }
            for snapshot in ["base", "candidate"] {
                fs::write(
                    root.join(snapshot)
                        .join("requirements/MISSION-012-CLAMP-001.json"),
                    REQUIREMENT_FIXTURE,
                )
                .unwrap();
                fs::write(
                    root.join(snapshot)
                        .join("policy/MISSION-012-CLAMP-001-v1.json"),
                    POLICY_V1_FIXTURE,
                )
                .unwrap();
                fs::write(
                    root.join(snapshot)
                        .join("policy/MISSION-012-CLAMP-001-v2.json"),
                    POLICY_V2_FIXTURE,
                )
                .unwrap();
                fs::write(
                    root.join(snapshot)
                        .join("policy/MISSION-012-CLAMP-001-v3.json"),
                    POLICY_V3_FIXTURE,
                )
                .unwrap();
                fs::write(
                    root.join(snapshot)
                        .join("policy/MISSION-012-CLAMP-001-v4.json"),
                    POLICY_V4_FIXTURE,
                )
                .unwrap();
                fs::write(
                    root.join(snapshot).join("policy/active.json"),
                    ACTIVE_POLICY_FIXTURE,
                )
                .unwrap();
            }
            fs::write(root.join("base/src/lib.rs"), "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        minimum // BUG: values above the range should clamp to maximum.\n    } else {\n        value\n    }\n}\n").unwrap();
            fs::copy(
                root.join("base/src/lib.rs"),
                root.join("candidate/src/lib.rs"),
            )
            .unwrap();
            Self(root)
        }

        fn config(&self) -> SandboxConfig {
            let output = Command::new("rustc")
                .arg("--print")
                .arg("sysroot")
                .output()
                .unwrap();
            assert!(output.status.success());
            SandboxConfig::new(
                self.0.join("base"),
                self.0.join("candidate"),
                self.0.join("target"),
                String::from_utf8(output.stdout).unwrap().trim(),
            )
        }

        fn trusted_git_repository(&self) -> PathBuf {
            let repository = self.0.join("trusted-git");
            fs::create_dir_all(&repository).unwrap();
            let output = Command::new("/usr/bin/git")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", "/nonexistent")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .arg("-C")
                .arg(&repository)
                .args(["init", "--quiet"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            repository
        }

        fn record_snapshot(&self, repository: &Path, snapshot_name: &str, message: &str) -> String {
            let source = self.0.join(snapshot_name);
            for entry in fs::read_dir(repository).unwrap() {
                let entry = entry.unwrap();
                if entry.file_name() == ".git" {
                    continue;
                }
                if entry.file_type().unwrap().is_dir() {
                    fs::remove_dir_all(entry.path()).unwrap();
                } else {
                    fs::remove_file(entry.path()).unwrap();
                }
            }
            copy_snapshot_contents(&source, repository);
            let git = |args: &[&str]| {
                let output = Command::new("/usr/bin/git")
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .env("HOME", "/nonexistent")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .env("GIT_AUTHOR_NAME", "MISSION-012 test")
                    .env("GIT_AUTHOR_EMAIL", "mission012-test@invalid")
                    .env("GIT_COMMITTER_NAME", "MISSION-012 test")
                    .env("GIT_COMMITTER_EMAIL", "mission012-test@invalid")
                    .arg("-C")
                    .arg(repository)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "git {args:?} failed with {}; stdout={:?}, stderr={:?}",
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                String::from_utf8(output.stdout).unwrap()
            };
            git(&["add", "-A"]);
            git(&["commit", "--quiet", "-m", message]);
            git(&["rev-parse", "HEAD"]).trim().to_owned()
        }

        fn fresh_verification_target(&self, label: &str) -> PathBuf {
            let path = self.0.join(format!("target-{label}"));
            fs::create_dir(&path).unwrap();
            path
        }
    }

    fn copy_snapshot_contents(source: &Path, destination: &Path) {
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                fs::create_dir(&destination_path).unwrap();
                copy_snapshot_contents(&source_path, &destination_path);
            } else {
                fs::copy(&source_path, &destination_path).unwrap();
                fs::set_permissions(
                    &destination_path,
                    fs::metadata(&source_path).unwrap().permissions(),
                )
                .unwrap();
            }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            fn restore_directory_permissions(path: &Path) {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
                            restore_directory_permissions(&entry.path());
                        }
                    }
                }
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            }

            restore_directory_permissions(&self.0);
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn capabilities() -> RuntimeCapabilitiesV1 {
        RuntimeCapabilitiesV1 {
            read_paths: vec![
                "Cargo.toml".into(),
                "Cargo.lock".into(),
                "requirements/MISSION-012-CLAMP-001.json".into(),
                "src/lib.rs".into(),
                "tests/acceptance.rs".into(),
            ],
            write_paths: vec!["src/lib.rs".into()],
            tools: vec![
                "workspace.read_text".into(),
                "workspace.write_text".into(),
                "cargo.test".into(),
                "workspace.diff".into(),
            ],
            budget: RuntimeBudgetV1 {
                max_tool_calls: 12,
                timeout_ms: 120_000,
                max_output_tokens: 8192,
            },
        }
    }

    #[test]
    fn sandbox_reports_real_red_then_green_on_the_candidate() {
        let tree = TempTree::new();
        let config = tree.config();
        let baseline = run_sandbox_test(&config, TEST_TIMEOUT).unwrap();
        let baseline_output = format!("{}\n{}", baseline.stdout, baseline.stderr);
        assert_eq!(baseline.exit_code, Some(101), "{baseline_output}");
        assert!(
            baseline_output.contains(EXPECTED_FAILURE),
            "{baseline_output}"
        );
        assert!(baseline_output.contains("left: -2"), "{baseline_output}");
        assert!(baseline_output.contains("right: 10"), "{baseline_output}");

        fs::write(tree.0.join("candidate/src/lib.rs"), "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        maximum\n    } else {\n        value\n    }\n}\n").unwrap();
        let mut green_config = config;
        green_config.target_root = tree.0.join("target-green");
        fs::create_dir(&green_config.target_root).unwrap();
        let green = run_sandbox_test(&green_config, TEST_TIMEOUT).unwrap();
        let green_output = format!("{}\n{}", green.stdout, green.stderr);
        assert_eq!(green.exit_code, Some(0), "{green_output}");
        assert!(
            green_output.contains(EXPECTED_GREEN_SUMMARY),
            "{green_output}"
        );
    }

    #[test]
    fn verifier_cannot_read_host_control_file_or_write_candidate_snapshot() {
        let tree = TempTree::new();
        let control_file = tree.0.join("host-webhook-inbox.jsonl");
        let control_bytes = b"private-control-plane-marker";
        fs::write(&control_file, control_bytes).unwrap();

        let candidate_source = "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum { minimum } else if value > maximum { maximum } else { value }\n}\n";
        fs::write(tree.0.join("candidate/src/lib.rs"), candidate_source).unwrap();
        let write_probe = tree.0.join("candidate/tests/sandbox-write-probe.txt");
        let write_probe_bytes = b"host-owned verifier input";
        fs::write(&write_probe, write_probe_bytes).unwrap();

        let acceptance_path = tree.0.join("candidate/tests/acceptance.rs");
        let mut acceptance = fs::read_to_string(&acceptance_path).unwrap();
        acceptance.push_str(&format!(
            r#"

#[test]
fn host_control_file_is_not_visible_inside_verifier() {{
    let result = std::fs::read({control_file});
    assert!(result.is_err(), "host control file unexpectedly readable");
}}

#[test]
fn candidate_snapshot_is_read_only_inside_verifier() {{
    let original = include_bytes!("sandbox-write-probe.txt");
    let write_result = std::fs::write("tests/sandbox-write-probe.txt", b"tampered");
    if write_result.is_ok() {{
        std::fs::write("tests/sandbox-write-probe.txt", original).unwrap();
    }}
    assert!(write_result.is_err(), "candidate snapshot unexpectedly writable");
}}
"#,
            control_file = format!("{:?}", control_file.to_string_lossy().as_ref()),
        ));
        fs::write(&acceptance_path, acceptance).unwrap();

        let mut config = tree.config();
        config.target_root = tree.0.join("target-control-plane");
        fs::create_dir(&config.target_root).unwrap();
        let result = run_sandbox_test(&config, TEST_TIMEOUT).unwrap();
        let output = format!("{}\n{}", result.stdout, result.stderr);
        assert_eq!(result.exit_code, Some(0), "{output}");
        assert!(output.contains("4 passed; 0 failed"), "{output}");
        assert!(!output.contains("private-control-plane-marker"));
        assert_eq!(fs::read(&control_file).unwrap(), control_bytes);
        assert_eq!(fs::read(&write_probe).unwrap(), write_probe_bytes);
        assert_eq!(
            fs::read(tree.0.join("candidate/src/lib.rs")).unwrap(),
            candidate_source.as_bytes()
        );
    }

    #[test]
    fn shallow_smoke_green_cannot_authorize_candidate_rejected_by_protected_acceptance() {
        let tree = TempTree::new();
        let regressed_source = "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        maximum\n    } else {\n        minimum // Regression: in-range values must remain unchanged.\n    }\n}\n";
        fs::write(tree.0.join("candidate/src/lib.rs"), regressed_source).unwrap();

        let smoke_candidate = tree.0.join("candidate-smoke");
        fs::create_dir_all(smoke_candidate.join("src")).unwrap();
        fs::create_dir_all(smoke_candidate.join("tests")).unwrap();
        fs::copy(
            tree.0.join("candidate/Cargo.toml"),
            smoke_candidate.join("Cargo.toml"),
        )
        .unwrap();
        fs::copy(
            tree.0.join("candidate/Cargo.lock"),
            smoke_candidate.join("Cargo.lock"),
        )
        .unwrap();
        fs::copy(
            tree.0.join("candidate/src/lib.rs"),
            smoke_candidate.join("src/lib.rs"),
        )
        .unwrap();
        fs::write(
            smoke_candidate.join("tests/acceptance.rs"),
            "use mission012_clamp::clamp_to_range;\n\n#[test]\nfn smoke_checks_only_out_of_range_boundaries() {\n    assert_eq!(clamp_to_range(-5, -2, 10), -2);\n    assert_eq!(clamp_to_range(15, -2, 10), 10);\n}\n",
        )
        .unwrap();

        let mut smoke_config = tree.config();
        smoke_config.candidate_dir = smoke_candidate;
        smoke_config.target_root = tree.0.join("target-smoke");
        fs::create_dir(&smoke_config.target_root).unwrap();
        let smoke = run_sandbox_test(&smoke_config, TEST_TIMEOUT).unwrap();
        let smoke_output = format!("{}\n{}", smoke.stdout, smoke.stderr);
        assert_eq!(smoke.exit_code, Some(0), "{smoke_output}");
        assert!(
            smoke_output.contains("1 passed; 0 failed"),
            "{smoke_output}"
        );

        let mut protected_config = tree.config();
        protected_config.target_root = tree.0.join("target-protected");
        fs::create_dir(&protected_config.target_root).unwrap();
        let candidate_source_sha256 = sha256(regressed_source.as_bytes());
        let protected = run_sandbox_test(&protected_config, TEST_TIMEOUT).unwrap();
        let protected_output = format!("{}\n{}", protected.stdout, protected.stderr);
        assert_eq!(protected.exit_code, Some(101), "{protected_output}");
        assert!(
            protected_output.contains("values_inside_the_range_are_unchanged"),
            "{protected_output}"
        );
        assert!(protected_output.contains("left: -2"), "{protected_output}");
        assert!(protected_output.contains("right: 4"), "{protected_output}");
        assert_eq!(
            sha256(&fs::read(tree.0.join("candidate/src/lib.rs")).unwrap()),
            candidate_source_sha256,
            "the independent test run must not mutate its candidate"
        );

        let requirement_sha256 = "a".repeat(64);
        let policy_sha256 = "b".repeat(64);
        let base_commit = "2".repeat(40);
        let candidate_commit = "1".repeat(40);
        let proof = sign_proof(
            ProofEvidence {
                schema_version: 1,
                requirement_sha256: requirement_sha256.clone(),
                policy_sha256: policy_sha256.clone(),
                candidate_commit: candidate_commit.clone(),
                base_commit: base_commit.clone(),
                merge_commit: None,
                verifier_id: "independent-verifier".into(),
                environment: "bubblewrap local fixture".into(),
                commands: vec![vec![
                    "/toolchain/bin/cargo".into(),
                    "test".into(),
                    "--offline".into(),
                    "--locked".into(),
                ]],
                artifact_sha256: vec![sha256(protected_output.as_bytes())],
                outcome: Outcome::Fail,
                detail: "protected acceptance failed: values_inside_the_range_are_unchanged".into(),
            },
            b"test-only verifier key",
        )
        .unwrap();
        let scope = BaseBranchScope::new(
            "http://127.0.0.1:3130",
            "forge-admin",
            "agent-forge-core",
            "main",
        )
        .unwrap();
        let current = PullRequestSnapshotV1 {
            origin: "http://127.0.0.1:3130".into(),
            owner: "forge-admin".into(),
            repository: "agent-forge-core".into(),
            number: 23,
            state: "open".into(),
            merged: false,
            draft: false,
            mergeable: Some(true),
            base_branch: "main".into(),
            base_sha: base_commit.clone(),
            head_sha: candidate_commit.clone(),
        };
        let context = MergePreflightContextV1 {
            pull_request_number: 23,
            execution_identity: "fixture-agent".into(),
            expected: ExpectedBinding {
                requirement_sha256,
                policy_sha256,
                candidate_commit,
                base_commit,
                merge_commit: None,
                verifier_id: "independent-verifier".into(),
            },
        };
        assert_eq!(
            validate_merge_preflight(
                &scope,
                &current,
                Some(&proof),
                b"test-only verifier key",
                &context,
            ),
            Err(MergePreflightError::Proof(GateError::OutcomeFailed)),
            "a signed refusal based on the protected test result must block merge"
        );
    }

    #[test]
    fn verifier_cpu_reservations_bound_the_policy_timeout_and_observed_wall_time() {
        assert_eq!(
            verifier_cpu_reservation_ms(Duration::from_secs(120)),
            240_000
        );
        assert_eq!(
            verifier_cpu_reservation_ms(Duration::from_millis(120_001)),
            242_000
        );
        assert_eq!(verifier_cpu_usage_upper_bound_ms(500, 240_000), 3_000);
        assert_eq!(verifier_cpu_usage_upper_bound_ms(120_000, 240_000), 240_000);
        assert_eq!(
            verifier_cpu_usage_upper_bound_ms(u64::MAX, 240_000),
            240_000
        );
        let policy_sha256 = verifier_cpu_policy_sha256();
        assert_eq!(policy_sha256.len(), 64);
        assert!(policy_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn sandbox_mounts_have_finite_sizes() {
        let tree = TempTree::new();
        fs::write(
            tree.0.join("candidate/build.rs"),
            r#"use std::process::Command;
fn main() {
    let output = Command::new("/usr/bin/df")
        .args(["-B1", "--output=size", "--", "/target", "/tmp"])
        .output()
        .expect("df is available in the verifier image");
    assert!(output.status.success(), "df failed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let sizes: Vec<u64> = stdout.lines().skip(1).map(|line| line.trim().parse().unwrap()).collect();
    assert_eq!(sizes.len(), 2, "unexpected df output: {stdout}");
    println!("cargo:warning=verifier-mount-sizes={sizes:?}");
    assert!(sizes[0] <= 268_435_456, "/target exceeds 256 MiB: {}", sizes[0]);
    assert!(sizes[1] <= 134_217_728, "/tmp exceeds 128 MiB: {}", sizes[1]);
}
"#,
        )
        .unwrap();

        let result = run_sandbox_test(&tree.config(), TEST_TIMEOUT).unwrap();
        let output = format!("{}\n{}", result.stdout, result.stderr);
        assert_eq!(result.exit_code, Some(101), "{output}");
        assert!(
            output.contains("verifier-mount-sizes=[268435456, 134217728]"),
            "{output}"
        );
        assert!(!output.contains("exceeds 256 MiB"), "{output}");
        assert!(!output.contains("exceeds 128 MiB"), "{output}");
    }

    #[test]
    fn verifier_scope_applies_cgroup_limits_and_kills_detached_children_at_deadline() {
        let mut command = Command::new(SYSTEMD_RUN);
        configure_systemd_connection_environment(&mut command);
        let probe = r#"set -eu
group=$(sed -n 's/^0:://p' /proc/self/cgroup)
test -n "$group"
cat "/sys/fs/cgroup${group}/memory.max"
cat "/sys/fs/cgroup${group}/pids.max"
cat "/sys/fs/cgroup${group}/cpu.max"
"#;
        let output = command
            .args(verifier_scope_arguments(Duration::from_secs(10)))
            .args(["--", "/bin/sh", "-c", probe])
            .output()
            .expect("start the isolated transient verifier scope");
        assert!(
            output.status.success(),
            "systemd-run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        let values: Vec<_> = stdout.lines().collect();
        assert_eq!(values.len(), 3, "{stdout}");
        assert_eq!(values[0], VERIFIER_MEMORY_MAX_BYTES.to_string());
        assert_eq!(values[1], VERIFIER_TASKS_MAX.to_string());
        assert_eq!(
            values[2],
            format!("{} 100000", VERIFIER_CPU_QUOTA_PERCENT * 1000)
        );

        let mut timeout_command = Command::new(SYSTEMD_RUN);
        configure_systemd_connection_environment(&mut timeout_command);
        let detached_child = timeout_command
            .args(verifier_scope_arguments(Duration::from_secs(1)))
            .args([
                "--",
                "/bin/sh",
                "-c",
                "/usr/bin/setsid /usr/bin/sleep 30 & child=$!; echo CHILD=$child; wait \"$child\"",
            ])
            .output()
            .expect("start the runtime-limited transient verifier scope");
        assert!(
            !detached_child.status.success(),
            "RuntimeMaxSec did not terminate the detached child"
        );
        let detached_stdout = String::from_utf8(detached_child.stdout).unwrap();
        let child_pid = detached_stdout
            .lines()
            .find_map(|line| line.strip_prefix("CHILD="))
            .expect("the isolated child reported its pid")
            .parse::<u32>()
            .unwrap();
        let child_proc_path = PathBuf::from(format!("/proc/{child_pid}"));
        let child_deadline = Instant::now() + Duration::from_secs(2);
        while child_proc_path.exists() && Instant::now() < child_deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !child_proc_path.exists(),
            "RuntimeMaxSec left detached child {child_pid} running"
        );
    }

    #[test]
    fn sandbox_blocks_nested_user_namespace_creation() {
        let tree = TempTree::new();
        let toolchain = tree.0.join("probe-toolchain");
        let bin = toolchain.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let cargo = bin.join("cargo");
        fs::write(
            &cargo,
            "#!/bin/sh\nif /usr/bin/unshare --user --map-root-user /usr/bin/true >/dev/null 2>&1; then printf allowed; else printf blocked; fi\n",
        )
        .unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o500)).unwrap();
        fs::write(bin.join("rustc"), "#!/bin/sh\nexit 0\n").unwrap();

        let mut config = tree.config();
        config.rust_sysroot = toolchain;
        let result = run_sandbox_test(&config, TEST_TIMEOUT).unwrap();
        assert_eq!(
            result.exit_code,
            Some(0),
            "{}\n{}",
            result.stdout,
            result.stderr
        );
        assert_eq!(
            result.stdout, "blocked",
            "nested user namespaces remain available"
        );
        assert!(
            fs::read_dir(tree.0.join("target"))
                .unwrap()
                .next()
                .is_none(),
            "the host target root must not receive candidate build output"
        );
    }

    #[test]
    fn sandbox_refuses_overlapping_writable_target_before_mutating_source_tree() {
        let tree = TempTree::new();
        let mut config = tree.config();
        config.target_root = tree.0.join("candidate/target");
        fs::create_dir(&config.target_root).unwrap();
        let error = run_sandbox_test(&config, TEST_TIMEOUT).unwrap_err();
        assert_eq!(error.code, "sandbox_paths_overlap");
        assert!(
            !config
                .target_root
                .join(format!("attempt-{}", std::process::id()))
                .exists()
        );
    }

    #[test]
    fn host_requires_baseline_red_and_fresh_green_and_receipts_refusals() {
        let tree = TempTree::new();
        let mut host = CandidateToolHost::new(capabilities(), tree.config()).unwrap();
        let premature = host.invoke("workspace.diff", json!({})).unwrap_err();
        assert_eq!(premature.code, "baseline_not_verified");
        let protected = host
            .invoke(
                "workspace.write_text",
                json!({"path":"tests/acceptance.rs","content":""}),
            )
            .unwrap_err();
        assert_eq!(protected.code, "tool_path_denied");
        assert_eq!(host.receipts().len(), 2);
        assert!(!host.receipts()[0].success);
        assert!(!host.receipts()[1].success);

        let red = host.invoke("cargo.test", json!({})).unwrap();
        assert_eq!(red["expected_baseline_failure"], true);
        let source = "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        maximum\n    } else {\n        value\n    }\n}\n";
        host.invoke(
            "workspace.write_text",
            json!({"path":"src/lib.rs","content":source}),
        )
        .unwrap();
        let green = host.invoke("cargo.test", json!({})).unwrap();
        assert_eq!(green["verified_acceptance_pass"], true);
        let diff = host.invoke("workspace.diff", json!({})).unwrap();
        assert_eq!(diff["changed_paths"], json!(["src/lib.rs"]));
        assert_eq!(diff["diff_sha256"].as_str().unwrap().len(), 64);

        host.invoke(
            "workspace.write_text",
            json!({"path":"src/lib.rs","content":source}),
        )
        .unwrap();
        assert_eq!(
            host.invoke("workspace.diff", json!({})).unwrap_err().code,
            "candidate_verification_stale"
        );
    }

    #[test]
    fn recovery_restores_only_an_exact_single_source_post_state() {
        let tree = TempTree::new();
        let base = tree.0.join("base");
        let candidate = tree.0.join("candidate");
        let base_source_sha256 = candidate_source_sha256(&base).unwrap();
        let after_source_sha256 = sha256(RECOVERY_FIXED_SOURCE.as_bytes());
        write_source(&candidate, RECOVERY_FIXED_SOURCE).unwrap();

        assert!(
            restore_candidate_for_workspace_write_replay(
                &base,
                &candidate,
                &base_source_sha256,
                &after_source_sha256,
            )
            .unwrap()
        );
        assert_eq!(
            candidate_source_sha256(&candidate).unwrap(),
            base_source_sha256
        );
        assert_eq!(
            snapshot_manifest(&candidate).unwrap(),
            snapshot_manifest(&base).unwrap()
        );
    }

    #[test]
    fn recovery_refuses_a_third_source_hash_or_any_other_candidate_change() {
        let tree = TempTree::new();
        let base = tree.0.join("base");
        let candidate = tree.0.join("candidate");
        let before = candidate_source_sha256(&base).unwrap();
        let after = sha256(RECOVERY_FIXED_SOURCE.as_bytes());
        write_source(&candidate, "unrelated candidate source\n").unwrap();
        let third_state_error =
            restore_candidate_for_workspace_write_replay(&base, &candidate, &before, &after)
                .unwrap_err();
        assert_eq!(
            third_state_error.code,
            "workspace_recovery_candidate_diverged"
        );
        assert_eq!(
            candidate_source_sha256(&candidate).unwrap(),
            sha256(b"unrelated candidate source\n")
        );

        write_source(&candidate, RECOVERY_FIXED_SOURCE).unwrap();
        fs::write(candidate.join("README.md"), "unexpected extra file\n").unwrap();
        let manifest_error =
            restore_candidate_for_workspace_write_replay(&base, &candidate, &before, &after)
                .unwrap_err();
        assert_eq!(manifest_error.code, "workspace_recovery_candidate_diverged");
        assert_eq!(
            candidate_source_sha256(&candidate).unwrap(),
            after,
            "a refused recovery must not rewrite the candidate source"
        );
    }

    #[test]
    fn journaled_writer_refuses_a_candidate_changed_since_its_prestate_read() {
        let tree = TempTree::new();
        let mut host = CandidateToolHost::new(capabilities(), tree.config()).unwrap();
        host.baseline_red_observed = true;
        let source_path = tree.0.join("candidate/src/lib.rs");
        let expected_before = candidate_source_sha256(&tree.0.join("candidate")).unwrap();
        fs::write(
            &source_path,
            "an intervening writer changed the candidate\n",
        )
        .unwrap();

        let refusal = host
            .invoke_workspace_write_if_sha256(
                json!({"path":"src/lib.rs","content":"replacement\n"}),
                &expected_before,
            )
            .unwrap_err();
        assert_eq!(refusal.code, "tool_operation_effect_diverged");
        assert_eq!(
            fs::read_to_string(&source_path).unwrap(),
            "an intervening writer changed the candidate\n"
        );

        let intervening_sha256 = candidate_source_sha256(&tree.0.join("candidate")).unwrap();
        let written = host
            .invoke_workspace_write_if_sha256(
                json!({"path":"src/lib.rs","content":"replacement\n"}),
                &intervening_sha256,
            )
            .unwrap();
        assert_eq!(written["written"], true);
        assert_eq!(fs::read_to_string(&source_path).unwrap(), "replacement\n");
        assert_eq!(host.receipts().len(), 2);
        assert!(!host.receipts()[0].success);
        assert!(host.receipts()[1].success);
    }

    #[test]
    fn candidate_cannot_forge_libtest_success_with_print_and_process_exit() {
        let tree = TempTree::new();
        let mut host = CandidateToolHost::new(capabilities(), tree.config()).unwrap();
        let red = host.invoke("cargo.test", json!({})).unwrap();
        assert_eq!(red["expected_baseline_failure"], true);

        let fake_green = r#"
pub fn clamp_to_range(_value: i32, _minimum: i32, _maximum: i32) -> i32 {
    use std::io::Write;
    let fake_libtest_output = b"values_outside_the_range_clamp_to_the_nearest_boundary left: -2 right: 10 values_inside_the_range_are_unchanged 2 passed; 0 failed";
    let _ = std::io::stdout().write_all(fake_libtest_output);
    std::process::exit(0)
}
"#;
        host.invoke(
            "workspace.write_text",
            json!({"path":"src/lib.rs","content":fake_green}),
        )
        .unwrap();
        let test = host.invoke("cargo.test", json!({})).unwrap();
        assert_eq!(test["exit_code"], 0);
        assert_eq!(test["timed_out"], false);
        assert_eq!(test["output_truncated"], false);
        assert_eq!(test["approved_candidate_source"], false);
        assert_eq!(test["verified_acceptance_pass"], false);
        // process::exit can bypass libtest's flush of captured stdout; the
        // security assertion is that this incomplete run is not verified green.
        assert_eq!(
            host.invoke("workspace.diff", json!({})).unwrap_err().code,
            "candidate_source_contract_violation"
        );
    }

    #[test]
    fn hard_linked_candidate_source_is_refused_without_touching_external_file() {
        let tree = TempTree::new();
        let external = tree.0.join("outside-source.rs");
        let candidate_source = tree.0.join("candidate/src/lib.rs");
        fs::write(&external, "external data must remain intact\n").unwrap();
        fs::remove_file(&candidate_source).unwrap();
        fs::hard_link(&external, &candidate_source).unwrap();

        let error = CandidateToolHost::new(capabilities(), tree.config())
            .err()
            .expect("hard-linked candidate must be rejected during initialization");
        assert_eq!(error.code, "candidate_snapshot_hardlink");
        let write_error = write_source(&tree.0.join("candidate"), "replacement\n").unwrap_err();
        assert_eq!(write_error.code, "tool_workspace_invalid");
        assert_eq!(
            fs::read_to_string(&external).unwrap(),
            "external data must remain intact\n"
        );
    }

    #[test]
    fn symlinked_candidate_source_or_source_directory_is_refused_without_touching_target() {
        let file_tree = TempTree::new();
        let external_file = file_tree.0.join("outside-source.rs");
        let candidate_source = file_tree.0.join("candidate/src/lib.rs");
        fs::write(&external_file, "external file must remain intact\n").unwrap();
        fs::remove_file(&candidate_source).unwrap();
        std::os::unix::fs::symlink(&external_file, &candidate_source).unwrap();

        let file_error = write_source(&file_tree.0.join("candidate"), "replacement\n")
            .expect_err("a source symlink must be refused");
        assert_eq!(file_error.code, "tool_workspace_invalid");
        assert!(
            fs::symlink_metadata(&candidate_source)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(&external_file).unwrap(),
            "external file must remain intact\n"
        );

        let directory_tree = TempTree::new();
        let external_dir = directory_tree.0.join("outside-src");
        fs::create_dir(&external_dir).unwrap();
        let external_source = external_dir.join("lib.rs");
        fs::write(&external_source, "external directory file stays intact\n").unwrap();
        fs::remove_file(directory_tree.0.join("candidate/src/lib.rs")).unwrap();
        fs::remove_dir(directory_tree.0.join("candidate/src")).unwrap();
        std::os::unix::fs::symlink(&external_dir, directory_tree.0.join("candidate/src")).unwrap();

        let directory_error = write_source(&directory_tree.0.join("candidate"), "replacement\n")
            .expect_err("a source-directory symlink must be refused");
        assert_eq!(directory_error.code, "tool_workspace_invalid");
        assert!(
            fs::symlink_metadata(directory_tree.0.join("candidate/src"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(&external_source).unwrap(),
            "external directory file stays intact\n"
        );
    }

    #[test]
    fn writable_test_path_aliases_and_model_diff_are_refused() {
        assert_eq!(
            parse_model_tool("workspace.diff", &json!({})),
            Err(ToolRefusal::UnknownTool)
        );
        assert_eq!(
            parse_model_tool("workspace/diff", &json!({})),
            Err(ToolRefusal::UnknownTool)
        );
        assert_eq!(
            parse_model_tool(
                "workspace.write_text",
                &json!({"path":"tests/../tests/acceptance.rs","content":""})
            ),
            Err(ToolRefusal::PathNotAllowed)
        );
        assert_eq!(
            parse_model_tool(
                "workspace.write_text",
                &json!({"path":"src/lib.rs","content":"x","command":"sh"})
            ),
            Err(ToolRefusal::InvalidInput)
        );
    }

    #[test]
    fn independent_pair_verifier_binds_fresh_red_green_to_exact_snapshots() {
        let tree = TempTree::new();
        let fixed_source = "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum {\n        minimum\n    } else if value > maximum {\n        maximum\n    } else {\n        value\n    }\n}\n";
        fs::write(tree.0.join("candidate/src/lib.rs"), fixed_source).unwrap();
        let config = tree.config();
        let repository = tree.trusted_git_repository();
        let base_commit = tree.record_snapshot(&repository, "base", "protected base");
        let candidate_commit = tree.record_snapshot(&repository, "candidate", "candidate");
        let report = verify_pair(
            &tree.0.join("base"),
            &tree.0.join("candidate"),
            &tree.fresh_verification_target("pass"),
            &config.rust_sysroot,
            &repository,
            &base_commit,
            &candidate_commit,
        )
        .unwrap();
        assert_eq!(report["verdict"], "pass", "{report}");
        assert_eq!(report["source_commit"], base_commit);
        assert_eq!(report["candidate_commit"], candidate_commit);
        assert_eq!(report["base"]["expected_acceptance_failure"], true);
        assert_eq!(report["candidate"]["acceptance_pass"], true);
        assert_eq!(report["candidate"]["unchanged_during_verification"], true);
        assert_eq!(
            report["binding"]["requirement_sha256"],
            EXPECTED_REQUIREMENT_SHA256
        );
        assert_eq!(report["binding"]["policy_sha256"], EXPECTED_POLICY_SHA256);
        assert_eq!(
            report["command"],
            json!(["/toolchain/bin/cargo", "test", "--offline", "--locked"])
        );
        assert_eq!(report["binding"]["verifier_id"], TRUSTED_VERIFIER_ID);
        assert_eq!(report["binding"]["git_tree_binding_verified"], true);
        assert_eq!(
            report["binding"]["candidate_tree_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(report["changed_paths"], json!(["src/lib.rs"]));
        assert_eq!(
            report["candidate_source_sha256"].as_str().unwrap().len(),
            64
        );
        assert_eq!(report["acceptance_sha256"].as_str().unwrap().len(), 64);

        let unrelated_commit = verify_pair(
            &tree.0.join("base"),
            &tree.0.join("candidate"),
            &tree.fresh_verification_target("unrelated-commit"),
            &config.rust_sysroot,
            &repository,
            &base_commit,
            &base_commit,
        )
        .unwrap();
        assert_eq!(unrelated_commit["verdict"], "inconclusive");
        assert_eq!(
            unrelated_commit["reason"],
            "snapshot_does_not_match_git_commit"
        );

        fs::write(
            tree.0.join("candidate/tests/acceptance.rs"),
            "#[test]\nfn opportunistic_criteria_edit() {}\n",
        )
        .unwrap();
        let candidate_commit = tree.record_snapshot(&repository, "candidate", "criteria tamper");
        let rejected = verify_pair(
            &tree.0.join("base"),
            &tree.0.join("candidate"),
            &tree.fresh_verification_target("criteria-tamper"),
            &config.rust_sysroot,
            &repository,
            &base_commit,
            &candidate_commit,
        )
        .unwrap();
        assert_eq!(rejected["verdict"], "fail", "{rejected}");
        assert_eq!(rejected["reason"], "candidate_write_set_violation");
        assert_eq!(
            rejected["changed_paths"],
            json!(["src/lib.rs", "tests/acceptance.rs"])
        );

        fs::write(
            tree.0.join("candidate/requirements/MISSION-012-CLAMP-001.json"),
            br#"{"schema_version":2,"id":"MISSION-012-CLAMP-001","revision":2,"source_issue_number":1}"#,
        )
        .unwrap();
        let candidate_commit = tree.record_snapshot(&repository, "candidate", "requirement tamper");
        let stale = verify_pair(
            &tree.0.join("base"),
            &tree.0.join("candidate"),
            &tree.fresh_verification_target("stale-binding"),
            &config.rust_sysroot,
            &repository,
            &base_commit,
            &candidate_commit,
        )
        .unwrap();
        assert_eq!(stale["verdict"], "inconclusive", "{stale}");
        assert_eq!(
            stale["reason"],
            "requirement_or_policy_binding_invalid_or_stale"
        );
    }

    #[test]
    fn verifier_rejects_a_candidate_policy_that_names_another_command() {
        let tree = TempTree::new();
        let mut altered_policy: Value = serde_json::from_slice(POLICY_V4_FIXTURE).unwrap();
        altered_policy["verification"]["command"] =
            json!(["/tmp/attacker-controlled-command", "test"]);
        let altered_policy_bytes = serde_json::to_vec_pretty(&altered_policy).unwrap();
        let altered_policy_sha256 = sha256(&altered_policy_bytes);
        let mut altered_active: Value = serde_json::from_slice(ACTIVE_POLICY_FIXTURE).unwrap();
        altered_active["policy_sha256"] = json!(altered_policy_sha256);
        let altered_active_bytes = serde_json::to_vec_pretty(&altered_active).unwrap();
        for snapshot in ["base", "candidate"] {
            fs::write(
                tree.0
                    .join(snapshot)
                    .join("policy/MISSION-012-CLAMP-001-v4.json"),
                &altered_policy_bytes,
            )
            .unwrap();
            fs::write(
                tree.0.join(snapshot).join("policy/active.json"),
                &altered_active_bytes,
            )
            .unwrap();
        }
        fs::write(tree.0.join("candidate/src/lib.rs"), RECOVERY_FIXED_SOURCE).unwrap();
        let repository = tree.trusted_git_repository();
        let base_commit = tree.record_snapshot(&repository, "base", "protected base");
        let candidate_commit =
            tree.record_snapshot(&repository, "candidate", "candidate policy edit");
        let report = verify_pair(
            &tree.0.join("base"),
            &tree.0.join("candidate"),
            &tree.fresh_verification_target("policy-command-tamper"),
            &tree.config().rust_sysroot,
            &repository,
            &base_commit,
            &candidate_commit,
        )
        .unwrap();

        assert_eq!(report["verdict"], "inconclusive", "{report}");
        assert_eq!(
            report["reason"],
            "requirement_or_policy_binding_invalid_or_stale"
        );
        assert_eq!(report["policy_sha256"], altered_policy_sha256);
        assert_ne!(report["policy_sha256"], EXPECTED_POLICY_SHA256);
    }

    #[test]
    fn verifier_materializes_a_private_snapshot_and_rejects_changed_source_bytes() {
        let tree = TempTree::new();
        let source = tree.0.join("base");
        let expected = snapshot_manifest(&source).unwrap();
        let pinned = tree.0.join("pinned-snapshot");
        assert!(materialize_verified_snapshot(&source, &expected, &pinned).unwrap());
        assert_eq!(snapshot_manifest(&pinned).unwrap(), expected);
        fn assert_tree_is_read_only(path: &Path) {
            let metadata = fs::symlink_metadata(path).unwrap();
            assert_eq!(metadata.permissions().mode() & 0o222, 0, "{path:?}");
            if metadata.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    assert_tree_is_read_only(&entry.unwrap().path());
                }
            }
        }
        assert_tree_is_read_only(&pinned);

        let original_source = fs::read(pinned.join("src/lib.rs")).unwrap();
        fs::write(
            source.join("src/lib.rs"),
            b"concurrent source replacement\n",
        )
        .unwrap();
        assert_eq!(
            fs::read(pinned.join("src/lib.rs")).unwrap(),
            original_source
        );

        let stale_destination = tree.0.join("stale-pinned-snapshot");
        assert!(!materialize_verified_snapshot(&source, &expected, &stale_destination).unwrap());
    }

    #[test]
    fn independent_pair_verifier_refuses_unapproved_source_even_with_git_binding() {
        let tree = TempTree::new();
        fs::write(
            tree.0.join("candidate/src/lib.rs"),
            "pub fn clamp_to_range(value: i32, minimum: i32, maximum: i32) -> i32 {\n    if value < minimum { minimum } else if value > maximum { maximum } else { value }\n}\n",
        )
        .unwrap();
        let repository = tree.trusted_git_repository();
        let base_commit = tree.record_snapshot(&repository, "base", "protected base");
        let candidate_commit =
            tree.record_snapshot(&repository, "candidate", "unapproved candidate");
        let report = verify_pair(
            &tree.0.join("base"),
            &tree.0.join("candidate"),
            &tree.fresh_verification_target("unapproved-source"),
            &tree.config().rust_sysroot,
            &repository,
            &base_commit,
            &candidate_commit,
        )
        .unwrap();
        assert_eq!(report["verdict"], "fail", "{report}");
        assert_eq!(
            report["reason"],
            "candidate_source_not_allowed_by_task_profile"
        );
    }
}
