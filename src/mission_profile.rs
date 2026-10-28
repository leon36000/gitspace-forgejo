//! Host-owned demo profiles shared by local admission and the claim-bound MCP host.
//!
//! These loopback profiles are test adapters, not production configuration.
//! Keeping their repository, issue, requirement bytes, workspace, and runtime
//! capabilities in one catalog prevents admission and execution pins drifting
//! apart. Adding a profile does not grant it merge authority.

use crate::agent_runtime::{RuntimeBudgetV1, RuntimeCapabilitiesV1};

pub const DURATION_PARSER_PROFILE_ID: &str = "duration-parser";
pub const MISSION_012_CLAMP_PROFILE_ID: &str = "mission-012-clamp";

pub const MISSION_012_CLAMP_REQUIREMENTS: &[u8] =
    include_bytes!("../tests/fixtures/mission-012/requirements/MISSION-012-CLAMP-001.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissionProfileV1 {
    pub id: &'static str,
    pub origin: &'static str,
    pub owner: &'static str,
    pub repository: &'static str,
    pub branch: &'static str,
    pub issue_number: u64,
    pub requirement_id: &'static str,
    pub requirement_path: &'static str,
    pub requirement_revision: u64,
    pub workspace_id: &'static str,
    pub pinned_requirements: Option<&'static [u8]>,
    pub read_paths: &'static [&'static str],
    pub write_paths: &'static [&'static str],
    pub tools: &'static [&'static str],
    pub max_tool_calls: u32,
    pub timeout_ms: u64,
    pub max_output_tokens: u64,
}

impl MissionProfileV1 {
    pub fn capabilities(self) -> RuntimeCapabilitiesV1 {
        RuntimeCapabilitiesV1 {
            read_paths: self
                .read_paths
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
            write_paths: self
                .write_paths
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
            tools: self.tools.iter().map(|tool| (*tool).to_owned()).collect(),
            budget: RuntimeBudgetV1 {
                max_tool_calls: self.max_tool_calls,
                timeout_ms: self.timeout_ms,
                max_output_tokens: self.max_output_tokens,
            },
        }
    }
}

pub const DURATION_PARSER_PROFILE: MissionProfileV1 = MissionProfileV1 {
    id: DURATION_PARSER_PROFILE_ID,
    origin: "http://127.0.0.1:3130",
    owner: "forge-admin",
    repository: "agent-forge-core",
    branch: "main",
    issue_number: 41,
    requirement_id: "REQ-001",
    requirement_path: "requirements/REQ-001.json",
    requirement_revision: 1,
    workspace_id: "local-demo",
    pinned_requirements: None,
    read_paths: &["requirements/REQ-001.json", "src/lib.rs"],
    write_paths: &["src/lib.rs"],
    tools: &["workspace.read_text", "workspace.write_text", "cargo.test"],
    max_tool_calls: 20,
    timeout_ms: 60_000,
    max_output_tokens: 5_000,
};

pub const MISSION_012_CLAMP_PROFILE: MissionProfileV1 = MissionProfileV1 {
    id: MISSION_012_CLAMP_PROFILE_ID,
    origin: "http://127.0.0.1:3132",
    owner: "forge-admin",
    repository: "mission-012-clamp",
    branch: "main",
    issue_number: 1,
    requirement_id: "MISSION-012-CLAMP-001",
    requirement_path: "requirements/MISSION-012-CLAMP-001.json",
    requirement_revision: 1,
    workspace_id: "mission-012-clamp-issue-1",
    pinned_requirements: Some(MISSION_012_CLAMP_REQUIREMENTS),
    read_paths: &[
        "Cargo.toml",
        "Cargo.lock",
        "requirements/MISSION-012-CLAMP-001.json",
        "src/lib.rs",
        "tests/acceptance.rs",
    ],
    write_paths: &["src/lib.rs"],
    tools: &[
        "mission.context",
        "workspace.read_text",
        "workspace.write_text",
        "cargo.test",
    ],
    max_tool_calls: 12,
    timeout_ms: 300_000,
    max_output_tokens: 4096,
};

pub const MISSION_PROFILES: &[MissionProfileV1] =
    &[DURATION_PARSER_PROFILE, MISSION_012_CLAMP_PROFILE];

pub fn profile_by_id(id: &str) -> Option<&'static MissionProfileV1> {
    MISSION_PROFILES.iter().find(|profile| profile.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Requirements, sha256};

    #[test]
    fn demo_profile_catalog_has_unique_ids_and_exact_requirement_binding() {
        assert_ne!(DURATION_PARSER_PROFILE.id, MISSION_012_CLAMP_PROFILE.id);
        assert_eq!(
            profile_by_id(DURATION_PARSER_PROFILE_ID),
            Some(&DURATION_PARSER_PROFILE)
        );
        assert_eq!(
            profile_by_id(MISSION_012_CLAMP_PROFILE_ID),
            Some(&MISSION_012_CLAMP_PROFILE)
        );
        assert_eq!(profile_by_id("unknown"), None);

        let requirements: Requirements =
            serde_json::from_slice(MISSION_012_CLAMP_REQUIREMENTS).unwrap();
        assert_eq!(requirements.id, MISSION_012_CLAMP_PROFILE.requirement_id);
        assert_eq!(
            requirements.revision,
            MISSION_012_CLAMP_PROFILE.requirement_revision
        );
        assert_eq!(
            requirements.source_issue_number,
            Some(MISSION_012_CLAMP_PROFILE.issue_number)
        );
        assert_eq!(sha256(MISSION_012_CLAMP_REQUIREMENTS).len(), 64);
    }

    #[test]
    fn mcp_host_capabilities_are_the_admission_profile_capabilities() {
        let capabilities = MISSION_012_CLAMP_PROFILE.capabilities();
        assert_eq!(capabilities.read_paths.len(), 5);
        assert_eq!(capabilities.write_paths, ["src/lib.rs"]);
        assert_eq!(capabilities.tools[0], "mission.context");
        assert_eq!(capabilities.budget.max_tool_calls, 12);
        assert_eq!(capabilities.budget.timeout_ms, 300_000);
        assert_eq!(capabilities.budget.max_output_tokens, 4096);

        let generic = DURATION_PARSER_PROFILE.capabilities();
        assert!(!generic.tools.iter().any(|tool| tool == "mission.context"));
    }
}
