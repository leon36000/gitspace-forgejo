use agent_forge_core::{agent_runtime::derive_mission_identity_v1, forgejo::BaseBranchScope};

fn scope(owner: &str, repository: &str, branch: &str) -> BaseBranchScope {
    BaseBranchScope::new("http://127.0.0.1:3130", owner, repository, branch).unwrap()
}

fn requirements_sha() -> String {
    "a".repeat(64)
}

#[test]
fn new_webhook_delivery_ids_do_not_change_the_same_mission_identity() {
    let current_event_delivery = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();
    let replayed_event_delivery = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();

    assert_eq!(current_event_delivery, replayed_event_delivery);
}

#[test]
fn new_intentional_execution_generation_keeps_mission_but_changes_idempotency_key() {
    let first = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();
    let second = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &requirements_sha(),
        2,
    )
    .unwrap();

    assert_eq!(first.mission_id, second.mission_id);
    assert_ne!(first.idempotency_key, second.idempotency_key);
}

#[test]
fn changed_requirements_create_a_new_versioned_mission() {
    let first = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();
    let revised = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &"b".repeat(64),
        1,
    )
    .unwrap();

    assert_ne!(first.mission_id, revised.mission_id);
    assert_ne!(first.idempotency_key, revised.idempotency_key);
}

#[test]
fn mission_is_scoped_to_repository_and_issue_not_base_branch() {
    let main = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();
    let release = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "release"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();
    let other_issue = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        42,
        &requirements_sha(),
        1,
    )
    .unwrap();
    let other_repository = derive_mission_identity_v1(
        &scope("forge-admin", "different-repo", "main"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();

    assert_eq!(main.mission_id, release.mission_id);
    assert_ne!(main.mission_id, other_issue.mission_id);
    assert_ne!(main.mission_id, other_repository.mission_id);
}

#[test]
fn normalized_origin_and_hex_case_do_not_create_duplicate_missions() {
    let canonical = derive_mission_identity_v1(
        &scope("forge-admin", "agent-forge-core", "main"),
        41,
        &requirements_sha(),
        1,
    )
    .unwrap();
    let alternate_input = BaseBranchScope::new(
        "HTTP://127.0.0.1:3130/",
        "forge-admin",
        "agent-forge-core",
        "main",
    )
    .unwrap();
    let uppercase_hash = requirements_sha().to_ascii_uppercase();
    let normalized = derive_mission_identity_v1(&alternate_input, 41, &uppercase_hash, 1).unwrap();

    assert_eq!(canonical, normalized);
}

#[test]
fn invalid_requirement_hash_or_zero_identity_values_are_refused() {
    assert_eq!(
        derive_mission_identity_v1(
            &scope("forge-admin", "agent-forge-core", "main"),
            41,
            "not-a-sha256",
            1,
        )
        .unwrap_err()
        .code,
        "requirement_hash_invalid"
    );
    assert_eq!(
        derive_mission_identity_v1(
            &scope("forge-admin", "agent-forge-core", "main"),
            0,
            &requirements_sha(),
            1,
        )
        .unwrap_err()
        .code,
        "mission_source_invalid"
    );
    assert_eq!(
        derive_mission_identity_v1(
            &scope("forge-admin", "agent-forge-core", "main"),
            41,
            &requirements_sha(),
            0,
        )
        .unwrap_err()
        .code,
        "execution_generation_invalid"
    );
}
