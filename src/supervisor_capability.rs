use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    fs,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    path::PathBuf,
};

const MAX_CAPABILITY_PACKET_BYTES: usize = 8 * 1024;
const MAX_CAPABILITY_LIFETIME_NS: u64 = 120_000_000_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MergeCapabilityTargetV1 {
    pub profile_selector: String,
    pub origin: String,
    pub owner: String,
    pub repository: String,
    pub pull_request_number: u64,
    pub base_branch: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessBindingV1 {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub pid_namespace: String,
    pub network_namespace: String,
}

impl ProcessBindingV1 {
    pub fn current() -> Result<Self, SupervisorCapabilityErrorV1> {
        let pid = unsafe { libc::getpid() };
        if pid <= 0 {
            return Err(SupervisorCapabilityErrorV1::ProcessUnavailable);
        }
        process_binding(pid as u32)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SupervisorMergeCapabilityEnvelopeV1 {
    pub schema_version: u32,
    pub target: MergeCapabilityTargetV1,
    pub supervisor: ProcessBindingV1,
    pub supervisor_uid: u32,
    pub gate: ProcessBindingV1,
    pub forgejo: ProcessBindingV1,
    pub expires_at_monotonic_ns: u64,
    pub nonce: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupervisorCapabilityErrorV1 {
    InvalidDescriptor,
    WrongSocketType,
    PeerCredentialUnavailable,
    PeerIdentityMismatch,
    MissingPacket,
    Malformed,
    TargetMismatch,
    Expired,
    ProcessUnavailable,
    ProcessBindingMismatch,
    Replayed,
    Revoked,
    UnsupportedPlatform,
}

impl SupervisorCapabilityErrorV1 {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidDescriptor => "supervisor_capability_invalid_descriptor",
            Self::WrongSocketType => "supervisor_capability_wrong_socket_type",
            Self::PeerCredentialUnavailable => "supervisor_capability_peer_unavailable",
            Self::PeerIdentityMismatch => "supervisor_capability_peer_identity_mismatch",
            Self::MissingPacket => "supervisor_capability_missing",
            Self::Malformed => "supervisor_capability_malformed",
            Self::TargetMismatch => "supervisor_capability_target_mismatch",
            Self::Expired => "supervisor_capability_expired",
            Self::ProcessUnavailable => "supervisor_capability_process_unavailable",
            Self::ProcessBindingMismatch => "supervisor_capability_process_binding_mismatch",
            Self::Replayed => "supervisor_capability_replayed",
            Self::Revoked => "supervisor_capability_revoked",
            Self::UnsupportedPlatform => "supervisor_capability_platform_unsupported",
        }
    }

    pub const fn action(self) -> &'static str {
        match self {
            Self::InvalidDescriptor | Self::WrongSocketType | Self::MissingPacket => {
                "launch the merge gate with one inherited AF_UNIX SOCK_SEQPACKET capability from the trusted host supervisor"
            }
            Self::PeerCredentialUnavailable | Self::PeerIdentityMismatch => {
                "restart the merge gate under the trusted supervisor in the same PID namespace"
            }
            Self::Malformed | Self::Replayed => {
                "issue one fresh bounded capability packet for this merge attempt"
            }
            Self::TargetMismatch => {
                "issue a capability bound to the selected repository, pull request, profile and base branch"
            }
            Self::Expired => {
                "rerun the complete merge preflight with a fresh short-lived capability"
            }
            Self::ProcessUnavailable | Self::ProcessBindingMismatch => {
                "restart Forgejo and the merge gate under the supervisor and issue a fresh process-bound capability"
            }
            Self::Revoked => {
                "rerun the complete merge preflight while the supervisor keeps the capability channel open"
            }
            Self::UnsupportedPlatform => {
                "run this local single-writer profile on a supported Linux host or keep merge admission refused"
            }
        }
    }
}

/// A verified, one-packet capability from the host supervisor. The live socket
/// is retained so supervisor death/closure can revoke admission immediately
/// before the Forgejo merge POST. This proves channel and process bindings, not
/// that the trusted supervisor's policy is correct.
#[derive(Debug)]
pub struct VerifiedSupervisorMergeCapabilityV1 {
    socket: OwnedFd,
    envelope: SupervisorMergeCapabilityEnvelopeV1,
    peer: PeerIdentity,
    pre_post_revalidated: Cell<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PeerIdentity {
    pid: u32,
    uid: u32,
}

impl VerifiedSupervisorMergeCapabilityV1 {
    /// Safely takes ownership of the inherited descriptor named by the local
    /// host launcher. Standard input/output/error are never accepted as a
    /// capability channel.
    pub fn receive_inherited_fd(
        fd: RawFd,
        expected_target: MergeCapabilityTargetV1,
    ) -> Result<Self, SupervisorCapabilityErrorV1> {
        if fd < 3 || unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
            return Err(SupervisorCapabilityErrorV1::InvalidDescriptor);
        }
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        Self::receive(socket, expected_target)
    }

    /// Takes ownership of an inherited descriptor, consumes exactly one
    /// bounded SOCK_SEQPACKET, and binds it to the selected merge target and
    /// live Linux process identities before any merge credential is read.
    pub fn receive(
        socket: OwnedFd,
        expected_target: MergeCapabilityTargetV1,
    ) -> Result<Self, SupervisorCapabilityErrorV1> {
        #[cfg(target_os = "linux")]
        {
            let peer = socket_peer_identity(socket.as_raw_fd())?;
            validate_socket(socket.as_raw_fd())?;
            let bytes = receive_one_packet(socket.as_raw_fd())?;
            let envelope: SupervisorMergeCapabilityEnvelopeV1 = serde_json::from_slice(&bytes)
                .map_err(|_| SupervisorCapabilityErrorV1::Malformed)?;
            validate_envelope(&envelope, &expected_target, peer)?;
            let capability = Self {
                socket,
                envelope,
                peer,
                pre_post_revalidated: Cell::new(false),
            };
            capability.ensure_live()?;
            Ok(capability)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (socket, expected_target);
            Err(SupervisorCapabilityErrorV1::UnsupportedPlatform)
        }
    }

    /// Re-checks the expiring target/process bindings and the still-open,
    /// single-use supervisor channel. Call this in the final pre-POST window.
    pub fn ensure_live(&self) -> Result<(), SupervisorCapabilityErrorV1> {
        #[cfg(target_os = "linux")]
        {
            let peer = socket_peer_identity(self.socket.as_raw_fd())?;
            if peer != self.peer
                || peer.pid != self.envelope.supervisor.pid
                || peer.uid != self.envelope.supervisor_uid
                || peer.uid != unsafe { libc::geteuid() }
            {
                return Err(SupervisorCapabilityErrorV1::PeerIdentityMismatch);
            }
            validate_envelope(&self.envelope, &self.envelope.target, peer)?;
            validate_channel_live(self.socket.as_raw_fd())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(SupervisorCapabilityErrorV1::UnsupportedPlatform)
        }
    }

    /// Performs the check at the exact pre-POST hook and records that fact for
    /// the gate's resulting audit record.
    pub fn ensure_live_before_merge_post(&self) -> Result<(), SupervisorCapabilityErrorV1> {
        self.ensure_live()?;
        self.pre_post_revalidated.set(true);
        Ok(())
    }

    /// Safe, non-secret audit fields attached to a gate decision. The nonce is
    /// represented only by its digest; this summary does not attest that the
    /// supervisor's exclusion policy was correct.
    pub fn audit_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": self.envelope.schema_version,
            "target": self.envelope.target,
            "supervisor": self.envelope.supervisor,
            "supervisor_uid": self.envelope.supervisor_uid,
            "gate": self.envelope.gate,
            "forgejo": self.envelope.forgejo,
            "expires_at_monotonic_ns": self.envelope.expires_at_monotonic_ns,
            "nonce_sha256": crate::sha256(self.envelope.nonce.as_bytes()),
            "pre_post_revalidated": self.pre_post_revalidated.get(),
            "supervisor_policy_independently_verified": false,
        })
    }
}

pub fn monotonic_time_nanoseconds() -> Result<u64, SupervisorCapabilityErrorV1> {
    #[cfg(target_os = "linux")]
    {
        let mut value: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) } != 0
            || value.tv_sec < 0
            || value.tv_nsec < 0
            || value.tv_nsec >= 1_000_000_000
        {
            return Err(SupervisorCapabilityErrorV1::ProcessUnavailable);
        }
        (value.tv_sec as u64)
            .checked_mul(1_000_000_000)
            .and_then(|seconds| seconds.checked_add(value.tv_nsec as u64))
            .ok_or(SupervisorCapabilityErrorV1::ProcessUnavailable)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(SupervisorCapabilityErrorV1::UnsupportedPlatform)
    }
}

fn validate_envelope(
    envelope: &SupervisorMergeCapabilityEnvelopeV1,
    expected_target: &MergeCapabilityTargetV1,
    peer: PeerIdentity,
) -> Result<(), SupervisorCapabilityErrorV1> {
    if envelope.schema_version != 1
        || !valid_target(&envelope.target)
        || &envelope.target != expected_target
    {
        return Err(SupervisorCapabilityErrorV1::TargetMismatch);
    }
    if envelope.nonce.len() != 64
        || !envelope
            .nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SupervisorCapabilityErrorV1::Malformed);
    }
    let now = monotonic_time_nanoseconds()?;
    if envelope.expires_at_monotonic_ns <= now
        || envelope.expires_at_monotonic_ns > now.saturating_add(MAX_CAPABILITY_LIFETIME_NS)
    {
        return Err(SupervisorCapabilityErrorV1::Expired);
    }
    if envelope.supervisor.pid != peer.pid
        || envelope.supervisor_uid != peer.uid
        || peer.uid != unsafe { libc::geteuid() }
    {
        return Err(SupervisorCapabilityErrorV1::PeerIdentityMismatch);
    }
    let current_gate = ProcessBindingV1::current()?;
    if envelope.gate != current_gate {
        return Err(SupervisorCapabilityErrorV1::ProcessBindingMismatch);
    }
    for claimed in [&envelope.supervisor, &envelope.forgejo] {
        if process_binding(claimed.pid)? != *claimed {
            return Err(SupervisorCapabilityErrorV1::ProcessBindingMismatch);
        }
    }
    if envelope.gate.pid_namespace != envelope.forgejo.pid_namespace
        || envelope.gate.network_namespace != envelope.forgejo.network_namespace
        || envelope.gate.pid_namespace != envelope.supervisor.pid_namespace
    {
        return Err(SupervisorCapabilityErrorV1::ProcessBindingMismatch);
    }
    Ok(())
}

fn valid_target(target: &MergeCapabilityTargetV1) -> bool {
    target.pull_request_number > 0
        && target.base_branch == "main"
        && [
            target.profile_selector.as_str(),
            target.origin.as_str(),
            target.owner.as_str(),
            target.repository.as_str(),
            target.base_branch.as_str(),
        ]
        .iter()
        .all(|value| !value.is_empty() && value.len() <= 255)
}

#[cfg(target_os = "linux")]
fn validate_socket(fd: RawFd) -> Result<(), SupervisorCapabilityErrorV1> {
    if fd < 3 {
        return Err(SupervisorCapabilityErrorV1::InvalidDescriptor);
    }
    let socket_type = getsockopt_int(fd, libc::SO_TYPE)?;
    if socket_type != libc::SOCK_SEQPACKET {
        return Err(SupervisorCapabilityErrorV1::WrongSocketType);
    }
    let mut address: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut address_len = std::mem::size_of_val(&address) as libc::socklen_t;
    let result = unsafe {
        libc::getsockname(
            fd,
            (&mut address as *mut libc::sockaddr_storage).cast(),
            &mut address_len,
        )
    };
    if result != 0 || address.ss_family as libc::c_int != libc::AF_UNIX {
        return Err(SupervisorCapabilityErrorV1::WrongSocketType);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn getsockopt_int(
    fd: RawFd,
    option: libc::c_int,
) -> Result<libc::c_int, SupervisorCapabilityErrorV1> {
    let mut value = 0;
    let mut len = std::mem::size_of_val(&value) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&mut value as *mut libc::c_int).cast(),
            &mut len,
        )
    };
    if result != 0 {
        return Err(SupervisorCapabilityErrorV1::InvalidDescriptor);
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
fn socket_peer_identity(fd: RawFd) -> Result<PeerIdentity, SupervisorCapabilityErrorV1> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&credentials) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if result != 0 || len as usize != std::mem::size_of::<libc::ucred>() {
        return Err(SupervisorCapabilityErrorV1::PeerCredentialUnavailable);
    }
    if credentials.pid <= 0 || credentials.uid != unsafe { libc::geteuid() } {
        return Err(SupervisorCapabilityErrorV1::PeerIdentityMismatch);
    }
    Ok(PeerIdentity {
        pid: credentials.pid as u32,
        uid: credentials.uid,
    })
}

#[cfg(target_os = "linux")]
fn receive_one_packet(fd: RawFd) -> Result<Vec<u8>, SupervisorCapabilityErrorV1> {
    let mut bytes = vec![0_u8; MAX_CAPABILITY_PACKET_BYTES + 1];
    let received = unsafe {
        libc::recv(
            fd,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            libc::MSG_DONTWAIT | libc::MSG_TRUNC,
        )
    };
    if received < 0 {
        let error = std::io::Error::last_os_error();
        if error
            .raw_os_error()
            .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
        {
            return Err(SupervisorCapabilityErrorV1::MissingPacket);
        }
        return Err(SupervisorCapabilityErrorV1::InvalidDescriptor);
    }
    if received == 0 {
        return Err(SupervisorCapabilityErrorV1::Revoked);
    }
    if received as usize > MAX_CAPABILITY_PACKET_BYTES {
        return Err(SupervisorCapabilityErrorV1::Malformed);
    }
    bytes.truncate(received as usize);
    validate_channel_live(fd)?;
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn validate_channel_live(fd: RawFd) -> Result<(), SupervisorCapabilityErrorV1> {
    let mut poll_descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let poll_result = unsafe { libc::poll(&mut poll_descriptor, 1, 0) };
    if poll_result < 0
        || poll_descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
    {
        return Err(SupervisorCapabilityErrorV1::Revoked);
    }
    if poll_descriptor.revents & libc::POLLIN != 0 {
        let mut byte = 0_u8;
        let extra = unsafe {
            libc::recv(
                fd,
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT | libc::MSG_TRUNC,
            )
        };
        if extra >= 0 {
            return Err(SupervisorCapabilityErrorV1::Replayed);
        }
        let error = std::io::Error::last_os_error();
        if !error
            .raw_os_error()
            .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
        {
            return Err(SupervisorCapabilityErrorV1::InvalidDescriptor);
        }
    }
    Ok(())
}

fn process_binding(pid: u32) -> Result<ProcessBindingV1, SupervisorCapabilityErrorV1> {
    #[cfg(target_os = "linux")]
    {
        if pid == 0 {
            return Err(SupervisorCapabilityErrorV1::ProcessUnavailable);
        }
        let start_time_ticks = process_start_time_ticks(pid)?;
        let namespace = |name: &str| -> Result<String, SupervisorCapabilityErrorV1> {
            fs::read_link(PathBuf::from(format!("/proc/{pid}/ns/{name}")))
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|_| SupervisorCapabilityErrorV1::ProcessUnavailable)
        };
        let binding = ProcessBindingV1 {
            pid,
            start_time_ticks,
            pid_namespace: namespace("pid")?,
            network_namespace: namespace("net")?,
        };
        // Bracket namespace reads with the kernel start time so a rapid PID
        // exit/reuse cannot combine one process's stat with another's ns links.
        if process_start_time_ticks(pid)? != start_time_ticks {
            return Err(SupervisorCapabilityErrorV1::ProcessUnavailable);
        }
        Ok(binding)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Err(SupervisorCapabilityErrorV1::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
fn process_start_time_ticks(pid: u32) -> Result<u64, SupervisorCapabilityErrorV1> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| SupervisorCapabilityErrorV1::ProcessUnavailable)?;
    let close_paren = stat
        .rfind(')')
        .ok_or(SupervisorCapabilityErrorV1::ProcessUnavailable)?;
    stat[close_paren + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or(SupervisorCapabilityErrorV1::ProcessUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    fn target() -> MergeCapabilityTargetV1 {
        MergeCapabilityTargetV1 {
            profile_selector: "mission-012-clamp".into(),
            origin: "http://127.0.0.1:3132".into(),
            owner: "forge-admin".into(),
            repository: "mission-012-clamp".into(),
            pull_request_number: 23,
            base_branch: "main".into(),
        }
    }

    fn socket_pair(socket_type: libc::c_int) -> (OwnedFd, OwnedFd) {
        let mut descriptors = [-1; 2];
        let result = unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                socket_type | libc::SOCK_CLOEXEC,
                0,
                descriptors.as_mut_ptr(),
            )
        };
        assert_eq!(result, 0);
        unsafe {
            (
                OwnedFd::from_raw_fd(descriptors[0]),
                OwnedFd::from_raw_fd(descriptors[1]),
            )
        }
    }

    fn envelope(target: MergeCapabilityTargetV1) -> SupervisorMergeCapabilityEnvelopeV1 {
        let process = ProcessBindingV1::current().expect("current process identity");
        SupervisorMergeCapabilityEnvelopeV1 {
            schema_version: 1,
            target,
            supervisor: process.clone(),
            supervisor_uid: unsafe { libc::geteuid() },
            gate: process.clone(),
            forgejo: process,
            expires_at_monotonic_ns: monotonic_time_nanoseconds().unwrap() + 60_000_000_000,
            nonce: "a".repeat(64),
        }
    }

    fn send_envelope(peer: &OwnedFd, envelope: &SupervisorMergeCapabilityEnvelopeV1) {
        let bytes = serde_json::to_vec(envelope).unwrap();
        let sent = unsafe {
            libc::send(
                peer.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        assert_eq!(sent, bytes.len() as isize);
    }

    #[test]
    fn accepts_one_live_peer_bound_capability_and_rechecks_it() {
        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        send_envelope(&supervisor, &envelope(target()));

        let capability = VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
            .expect("trusted live supervisor capability should verify");

        capability.ensure_live().expect("capability remains live");
        capability
            .ensure_live_before_merge_post()
            .expect("capability remains live at the final POST check");
        let audit = capability.audit_summary();
        assert_eq!(audit["pre_post_revalidated"], true);
        assert_eq!(audit["supervisor_policy_independently_verified"], false);
        assert_eq!(audit["target"]["pull_request_number"], 23);
    }

    #[test]
    fn refuses_wrong_target_and_expired_capabilities() {
        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        send_envelope(&supervisor, &envelope(target()));
        let mut wrong_target = target();
        wrong_target.pull_request_number += 1;
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, wrong_target)
                .expect_err("target mismatch must refuse")
                .code(),
            "supervisor_capability_target_mismatch"
        );

        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        let mut expired = envelope(target());
        expired.expires_at_monotonic_ns = monotonic_time_nanoseconds().unwrap().saturating_sub(1);
        send_envelope(&supervisor, &expired);
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
                .expect_err("expired capability must refuse")
                .code(),
            "supervisor_capability_expired"
        );
    }

    #[test]
    fn refuses_wrong_process_bindings_and_namespaces() {
        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        let mut invalid = envelope(target());
        invalid.forgejo.network_namespace = "net:[wrong]".into();
        send_envelope(&supervisor, &invalid);
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
                .expect_err("Forgejo must share the private network namespace")
                .code(),
            "supervisor_capability_process_binding_mismatch"
        );

        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        let mut wrong_peer = envelope(target());
        wrong_peer.supervisor.pid = wrong_peer.supervisor.pid.saturating_add(1);
        send_envelope(&supervisor, &wrong_peer);
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
                .expect_err("claimed supervisor PID must match kernel peer credentials")
                .code(),
            "supervisor_capability_peer_identity_mismatch"
        );
    }

    #[test]
    fn refuses_stream_sockets_malformed_packets_and_duplicate_packets() {
        let (gate, supervisor) = socket_pair(libc::SOCK_STREAM);
        send_envelope(&supervisor, &envelope(target()));
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
                .expect_err("stream socket must not be treated as a packet capability")
                .code(),
            "supervisor_capability_wrong_socket_type"
        );

        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        let bytes = b"not-json";
        let sent = unsafe {
            libc::send(
                supervisor.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        assert_eq!(sent, bytes.len() as isize);
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
                .expect_err("malformed packet must refuse")
                .code(),
            "supervisor_capability_malformed"
        );

        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        let valid = serde_json::to_vec(&envelope(target())).unwrap();
        for _ in 0..2 {
            let sent = unsafe {
                libc::send(
                    supervisor.as_raw_fd(),
                    valid.as_ptr().cast(),
                    valid.len(),
                    libc::MSG_NOSIGNAL,
                )
            };
            assert_eq!(sent, valid.len() as isize);
        }
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
                .expect_err("a duplicate queued packet must refuse")
                .code(),
            "supervisor_capability_replayed"
        );

        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        let oversized = vec![b'x'; MAX_CAPABILITY_PACKET_BYTES + 1];
        let sent = unsafe {
            libc::send(
                supervisor.as_raw_fd(),
                oversized.as_ptr().cast(),
                oversized.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        assert_eq!(sent, oversized.len() as isize);
        assert_eq!(
            VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
                .expect_err("oversized packet must refuse")
                .code(),
            "supervisor_capability_malformed"
        );
    }

    #[test]
    fn refuses_a_supervisor_that_disconnects_after_receipt() {
        let (gate, supervisor) = socket_pair(libc::SOCK_SEQPACKET);
        send_envelope(&supervisor, &envelope(target()));
        let capability = VerifiedSupervisorMergeCapabilityV1::receive(gate, target())
            .expect("initial live capability");
        drop(supervisor);

        assert_eq!(
            capability
                .ensure_live()
                .expect_err("closed supervisor must revoke capability")
                .code(),
            "supervisor_capability_revoked"
        );
    }
}
