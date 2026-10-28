#!/usr/bin/env python3
"""Build and run the fixed-port reconcile CLI e2e in an allowlisted sandbox."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import tempfile

import run_forgejo_mission_delivery_process_e2e as sandbox


REPO = sandbox.REPO
TEST_TARGET = "webhook_receiver_process_e2e"
TEST_NAMES = (
    "actual_receiver_requires_reviewed_action_before_reserving_delivery",
    "actual_receiver_rejects_missing_and_malformed_release_repository_identity_then_accepts_same_delivery",
    "actual_receiver_rejects_missing_and_malformed_workflow_repository_full_name_with_nested_field_path",
    "actual_receiver_rejects_zero_release_id_then_accepts_same_id_retry",
    "actual_receiver_rejects_invalid_release_action_then_accepts_corrected_same_delivery",
    "actual_receiver_refuses_workflow_job_status_header_substitution_before_append",
    "actual_receiver_refuses_issue_comment_without_is_pull_before_reserving_delivery",
    "actual_receiver_refuses_issue_comment_marked_non_pull_with_pr_parent_then_accepts_same_delivery",
    "actual_receiver_refuses_zero_issue_comment_id_with_actionable_field_guidance",
    "actual_receiver_refuses_missing_create_sha_with_actionable_guidance",
    "actual_receiver_refuses_invalid_create_branch_ref_with_actionable_guidance",
    "actual_receiver_reports_tag_create_as_unsupported_payload_field_without_journaling",
    "actual_receiver_classifies_tag_delete_and_non_string_ref_type_without_journaling",
    "actual_receiver_restarts_after_missing_is_pull_and_accepts_corrected_same_delivery",
    "actual_reconcile_cli_process_reads_receiver_journal_against_private_http_fixture",
    "actual_reconcile_cli_process_projects_current_issue_assignees_from_api",
    "actual_reconcile_cli_process_projects_current_issue_milestone_from_api",
    "actual_reconcile_cli_process_projects_current_pull_request_milestone_from_api",
    "actual_reconcile_cli_process_projects_current_pull_request_labels_for_batch",
    "actual_reconcile_cli_process_refuses_malformed_current_pull_request_labels",
    "actual_reconcile_cli_process_projects_current_unassigned_pull_request_milestone_for_batch",
    "actual_reconcile_cli_process_projects_current_pull_request_assignees_from_api",
    "actual_reconcile_cli_process_distinguishes_unknown_and_empty_pull_request_assignees",
    "actual_reconcile_cli_process_distinguishes_unknown_and_unassigned_pull_request_milestone_in_batch",
    "actual_reconcile_cli_process_refuses_malformed_current_pull_request_assignees",
    "actual_reconcile_cli_process_refuses_malformed_current_pull_request_milestone",
    "actual_reconcile_cli_process_reads_corrected_header_retry_from_receiver_journal",
    "actual_reconcile_cli_process_reads_gitea_alias_only_delivery",
    "actual_reconcile_cli_process_reads_mixed_forgejo_gitea_aliases",
    "actual_reconcile_cli_process_reads_workflow_run_canonically_and_projects_safe_fields",
    "actual_reconcile_cli_process_reads_current_release_by_id_and_omits_notes",
    "actual_reconcile_cli_process_keeps_deleted_release_present_when_current_api_returns_200",
    "actual_reconcile_cli_process_keeps_release_forbidden_read_inconclusive_without_projection",
    "actual_reconcile_cli_process_keeps_release_unauthorized_read_inconclusive_without_projection",
    "actual_reconcile_cli_process_recovers_release_read_after_transient_503",
    "actual_reconcile_cli_process_keeps_deleted_release_404_inconclusive_without_projection",
    "actual_reconcile_cli_process_refuses_escape_equivalent_duplicate_workflow_status_before_projection",
    "actual_reconcile_cli_process_refuses_unknown_current_workflow_run_status",
    "actual_reconcile_cli_process_refuses_unknown_current_workflow_job_status_before_parent_read",
    "actual_reconcile_cli_process_refuses_unknown_parent_run_status_after_valid_job_readback",
    "actual_reconcile_cli_process_refuses_missing_parent_run_after_valid_job_readback",
    "actual_reconcile_cli_process_shares_missing_parent_run_readback_across_job_batch",
    "actual_reconcile_cli_process_reads_private_workflow_run_with_host_credential_fd",
    "actual_reconcile_cli_process_refuses_workflow_run_api_id_mismatch",
    "actual_reconcile_cli_process_refuses_workflow_run_foreign_api_html_url",
    "actual_reconcile_cli_process_refuses_foreign_release_before_credential_or_api",
    "actual_reconcile_cli_process_refuses_workflow_run_foreign_api_repository",
    "actual_reconcile_cli_process_refuses_workflow_run_null_api_commit_sha",
    "actual_reconcile_cli_process_refuses_workflow_run_malformed_api_updated",
    "actual_reconcile_cli_process_keeps_workflow_run_404_inconclusive_without_projection",
    "actual_reconcile_cli_process_reads_workflow_job_and_parent_run_without_runner_details",
    "actual_reconcile_cli_process_reads_private_workflow_job_and_parent_run_with_host_credential",
    "actual_reconcile_cli_process_refuses_job_attached_to_another_run_before_parent_read",
    "actual_reconcile_cli_process_refuses_unscoped_workflow_job_url",
    "actual_reconcile_cli_process_refuses_api_url_for_another_workflow_job_index",
    "actual_reconcile_cli_process_keeps_workflow_job_404_inconclusive_without_parent_read",
    "actual_reconcile_cli_process_refuses_persisted_unknown_workflow_action_before_api_read",
    "actual_reconcile_cli_process_refuses_persisted_incompatible_workflow_action_before_api_read",
    "actual_reconcile_cli_process_refuses_persisted_incompatible_workflow_job_action_before_api_read",
    "actual_reconcile_cli_process_refuses_unknown_future_event_kind_before_api_read",
    "actual_reconcile_cli_process_refuses_persisted_review_without_action_before_api_read",
    "actual_reconcile_cli_process_rejects_legacy_issue_review_member_before_api_read",
    "actual_reconcile_cli_process_continues_batch_after_legacy_issue_review_member",
    "actual_reconcile_cli_process_uses_current_state_after_out_of_order_webhooks",
    "actual_reconcile_cli_process_shares_current_comment_read_across_create_delete_batch",
    "actual_reconcile_cli_process_reuses_pull_request_parent_and_comment_read_across_batch_orders",
    "actual_reconcile_cli_process_enforces_batch_cache_budget_against_private_http_fixture",
    "actual_reconcile_cli_process_refuses_repeated_comment_batch_output_budget",
    "actual_reconcile_cli_process_refuses_unbounded_chunked_current_api_response",
    "actual_reconcile_cli_process_refuses_oversized_current_api_header_json",
    "actual_commit_status_snapshot_cli_refuses_duplicate_case_variant_total_count_headers",
    "actual_reconcile_cli_process_requires_trusted_attestation_policy_before_host_credential_read",
    "actual_reconcile_cli_process_reports_credential_pipe_read_failure_with_selected_sources",
    "actual_reconcile_cli_process_redacts_malformed_open_credential_pipe_and_binds_sources",
    "actual_reconcile_cli_process_times_out_open_credential_pipe_and_binds_sources",
    "actual_reconcile_cli_process_refuses_oversized_credential_pipe_without_leaking_bytes",
    "actual_reconcile_cli_process_refuses_named_fifo_as_host_credential_source",
    "actual_reconcile_cli_process_refuses_untrusted_attestation_before_host_credential_read",
    "actual_reconcile_cli_process_refuses_semantically_tampered_attestation_before_host_credential_read",
    "actual_reconcile_cli_process_uses_host_credential_fd_for_receiver_journal",
    "actual_reconcile_cli_process_keeps_invalid_host_credential_inconclusive",
    "actual_reconcile_cli_process_reports_actionable_upstream_401_without_leaking_credential",
    "actual_reconcile_cli_process_stops_batch_reads_after_401_and_resumes_with_rotated_credential",
    "actual_reconcile_cli_process_preserves_successful_issue_when_pull_read_latches_401",
    "actual_reconcile_cli_process_preserves_out_of_scope_refusal_after_401",
    "actual_reconcile_cli_process_denied_batch_does_not_read_host_credential_fd",
    "actual_reconcile_cli_process_refuses_partial_selection_before_credential_read",
    "actual_reconcile_cli_process_refuses_duplicate_delivery_selection_before_credential_read",
    "actual_reconcile_cli_process_mixed_batch_limits_credential_to_configured_repository",
    "actual_reconcile_cli_process_reads_pull_request_current_state_from_private_http_fixture",
    "actual_reconcile_cli_process_refuses_missing_or_malformed_current_pull_request_draft",
    "actual_reconcile_cli_process_projects_current_pull_request_review_teams",
    "actual_reconcile_cli_process_refuses_duplicate_pull_request_review_team_ids",
    "actual_reconcile_cli_process_keeps_reviewer_teams_on_pr_comment_batch_parent",
    "actual_reconcile_cli_process_preserves_configured_same_number_pull_request_in_both_batch_orders",
    "actual_reconcile_cli_process_uses_credential_only_after_refusing_foreign_first_pr",
    "actual_reconcile_cli_process_preserves_foreign_first_pr_refusal_after_configured_401",
    "actual_reconcile_cli_process_reads_fork_pull_request_from_configured_target_api",
    "actual_reconcile_cli_process_reads_fork_pull_request_with_host_credential_fd",
    "actual_reconcile_cli_process_refuses_fork_pull_request_with_invalid_host_credential",
    "actual_reconcile_cli_process_keeps_fork_pull_request_inconclusive_without_host_credential",
    "actual_reconcile_cli_process_refuses_foreign_current_base_repository_after_fork_delivery",
    "actual_reconcile_cli_process_refuses_current_base_repository_without_identity",
    "scripted_http_fixture_stays_open_until_explicitly_stopped",
    "actual_reconcile_cli_process_refuses_merged_pull_request_without_merge_commit_sha",
    "actual_reconcile_cli_process_refuses_merged_pull_request_with_zero_merge_commit_sha",
    "actual_reconcile_cli_process_routes_approved_review_to_pull_request_api",
    "actual_reconcile_cli_process_routes_rejected_review_to_pull_request_api",
    "actual_reconcile_cli_process_keeps_issue_and_pull_request_number_collision_separate",
    "actual_reconcile_cli_process_routes_pull_request_review_comment_to_pull_request_api",
    "actual_reconcile_cli_process_routes_issue_comment_pr_conversation_to_pull_request_api",
    "actual_reconcile_cli_process_reconciles_same_id_corrected_pr_comment_after_receiver_refusal",
    "actual_reconcile_cli_process_routes_issue_comment_to_issue_api",
    "actual_reconcile_cli_process_uses_present_comment_readback_after_deleted_webhook",
    "actual_reconcile_cli_process_keeps_deleted_comment_inconclusive_when_parent_issue_is_unreadable",
    "actual_reconcile_cli_process_keeps_deleted_pr_comment_inconclusive_when_parent_access_is_revoked_between_reads",
    "actual_reconcile_cli_process_reads_current_branch_after_stale_push_webhook",
    "actual_reconcile_cli_process_reads_current_branch_after_native_delete_notification",
    "actual_reconcile_cli_process_keeps_native_delete_404_inconclusive",
    "actual_reconcile_cli_process_reads_new_branch_with_literal_escape_from_zero_before_push",
    "actual_reconcile_cli_process_refuses_zero_current_branch_commit",
    "actual_reconcile_cli_process_reads_sha256_current_branch_commit",
    "actual_reconcile_cli_process_reads_current_branch_after_native_create_notification",
    "actual_reconcile_cli_process_refuses_mismatched_branch_snapshot_without_journal_mutation",
    "actual_reconcile_cli_process_keeps_branch_404_ambiguous_and_inconclusive",
    "actual_reconcile_cli_process_negative_caches_missing_branch_across_deliveries",
    "actual_reconcile_cli_process_keeps_missing_merge_sha_inconclusive_in_mixed_batch",
    "actual_reconcile_cli_process_refuses_open_pull_request_reported_as_merged",
    "actual_reconcile_cli_process_refuses_unmerged_pull_request_with_merge_commit_sha",
    "actual_reconcile_cli_process_refuses_success_http_with_invalid_timestamp",
    "actual_reconcile_cli_process_rejects_foreign_canonical_html_url",
    "actual_reconcile_cli_process_rejects_same_origin_foreign_repository_html_url",
    "actual_reconcile_cli_process_refuses_issue_repository_metadata_mismatch",
    "actual_reconcile_cli_process_reports_invalid_pull_request_base_sha_field",
    "actual_reconcile_cli_process_keeps_forbidden_upstream_inconclusive",
    "actual_reconcile_cli_process_continues_mixed_batch_after_forbidden_resource",
    "actual_reconcile_cli_process_continues_branch_batch_with_legacy_tag_delete",
    "actual_reconcile_cli_process_reports_deleted_resource_in_single_delivery",
    "actual_reconcile_cli_process_reports_deleted_pull_request_in_single_delivery",
    "actual_reconcile_cli_process_keeps_deleted_pull_request_collision_separate_in_batch",
    "actual_reconcile_cli_process_reports_deleted_resource_without_aborting_batch",
    "actual_reconcile_cli_process_bounds_transient_upstream_retries",
    "actual_reconcile_cli_process_respects_retry_after_beyond_read_budget",
    "actual_reconcile_cli_process_recovers_from_in_budget_retry_after",
    "actual_reconcile_cli_process_waits_for_nonzero_in_budget_retry_after",
    "interrupted_reconcile_during_retry_after_resumes_from_unchanged_journal",
    "killed_reconcile_during_retry_after_resumes_from_unchanged_journal",
    "actual_reconcile_cli_process_shares_recovered_issue_read_across_batch_deliveries",
    "actual_reconcile_cli_process_shares_recovered_parent_issue_read_with_comment_batch",
    "actual_reconcile_cli_process_resumes_mixed_batch_after_transient_failure",
    "actual_reconcile_cli_process_preserves_retry_after_for_replayed_batch_deliveries",
    "actual_reconcile_cli_process_handles_invalid_and_http_date_retry_after",
    "actual_reconcile_cli_process_disables_retry_for_duplicate_retry_after_headers",
    "interrupted_reconcile_cli_can_resume_from_the_unchanged_journal",
    "reconcile_cli_terminates_curl_child_when_parent_is_signalled",
    "actual_reconcile_cli_process_cancels_slow_reads_at_the_shared_budget",
    "actual_reconcile_cli_process_accepts_complete_request_before_client_half_close",
    "actual_reconcile_cli_process_does_not_follow_redirect_with_host_credential",
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--scenario",
        choices=TEST_NAMES,
        action="append",
        help="run one isolated test instead of the full reconcile CLI matrix",
    )
    args = parser.parse_args()
    selected_tests = tuple(args.scenario) if args.scenario else TEST_NAMES

    rustc = shutil.which("rustc")
    bwrap = Path("/usr/bin/bwrap")
    if rustc is None:
        raise SystemExit("rustc not found in PATH")
    if not bwrap.is_file():
        raise SystemExit("/usr/bin/bwrap is required to isolate loopback fixtures")

    rust_sysroot = Path(
        sandbox.subprocess.run(
            [rustc, "--print", "sysroot"], check=True, capture_output=True, text=True
        ).stdout.strip()
    ).resolve()
    if not rust_sysroot.is_dir():
        raise SystemExit(f"rustc returned a non-directory sysroot: {rust_sysroot}")
    masked_home = Path.home().resolve()
    registry_source = masked_home / ".cargo" / "registry"
    if not registry_source.is_dir():
        raise SystemExit(f"offline Cargo registry cache is unavailable: {registry_source}")

    with tempfile.TemporaryDirectory(prefix="webhook-reconcile-cli-e2e-", dir="/tmp") as temp:
        temp_root = Path(temp).resolve()
        source_tree = temp_root / "source"
        sandbox.copy_source_tree(source_tree)
        registry_cache = temp_root / "cargo-registry"
        sandbox.copy_locked_registry(registry_source, registry_cache)

        binary_targets = sandbox.required_binary_targets((TEST_TARGET,))
        artifacts = {}
        for name in (TEST_TARGET, *binary_targets):
            artifact = temp_root / name
            artifact.touch(mode=0o700)
            artifact.chmod(0o700)
            artifacts[name] = artifact

        binaries = sandbox.compile_targets(
            bwrap,
            (TEST_TARGET,),
            binary_targets,
            artifacts,
            source_tree,
            rust_sysroot,
            masked_home,
            registry_cache,
        )

        for test_name in selected_tests:
            command = sandbox.isolated_command(
                bwrap,
                source_tree,
                binaries[TEST_TARGET],
                test_name,
                binary_targets,
                rust_sysroot,
                masked_home,
            )
            sandbox.run(command, cwd=REPO, test_name=test_name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
