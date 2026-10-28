#!/usr/bin/env python3
"""Capture native Forgejo branch ref events and reconcile their current state."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
from urllib.parse import quote
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_e2e as helpers  # noqa: E402
import run_forgejo_live_webhook_reconcile_emitted as emitted  # noqa: E402
import run_forgejo_native_push_webhook_reconcile as native_push  # noqa: E402

OWNER = helpers.OWNER
REPOSITORY = helpers.REPOSITORY
READER = helpers.READER
ORIGIN = helpers.ORIGIN
FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
BRANCH = "agent/native-delete-proof"


def validate_branch_name(branch: str) -> str:
    """Accept only a valid Git branch ref before using it in the disposable fixture."""
    if not isinstance(branch, str) or not branch:
        raise ValueError("branch must be a valid Git branch name")
    try:
        checked = subprocess.run(
            ["git", "check-ref-format", f"refs/heads/{branch}"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError("could not validate branch as a valid Git branch name") from error
    if checked.returncode != 0:
        raise ValueError("branch must be a valid Git branch name")
    return branch


def run_git(args: list[str], *, cwd: Path | None, env: dict[str, str], stage: str) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=60,
        check=False,
    )
    if completed.returncode:
        raise RuntimeError(f"local Git {stage} failed ({completed.returncode})")
    return completed.stdout.decode("utf-8", errors="strict").strip()


def validate_delete_reconciliation_result(
    status: int,
    result: dict[str, Any],
    *,
    delivery_id: str,
    owner: str,
    repository: str,
    branch: str,
    expected_present: bool,
    expected_sha: str | None = None,
) -> None:
    resource = {
        "kind": "branch",
        "owner": owner,
        "repository": repository,
        "branch": branch,
    }
    if (
        result.get("schema_version") != emitted.RECONCILE_SINGLE_SCHEMA_VERSION
        or result.get("api_version") != "forgejo-rest-v1"
        or result.get("delivery_id") != delivery_id
        or result.get("upstream_read_attempts") != 1
    ):
        raise ValueError("branch-delete reconciliation identity or schema does not match")

    if expected_present:
        canonical = result.get("canonical_state")
        if (
            status != 0
            or result.get("verdict") != "current_state_read"
            or result.get("resource") != resource
            or not isinstance(expected_sha, str)
            or not isinstance(canonical, dict)
            or canonical.get("state") != "present"
            or canonical.get("head_sha") != expected_sha
        ):
            raise ValueError("reconciliation did not read the expected current branch")
        return

    error = result.get("error")
    details = error.get("details") if isinstance(error, dict) else None
    if (
        status != 1
        or result.get("verdict") != "inconclusive"
        or not isinstance(error, dict)
        or error.get("code") != "resource_not_found"
        or error.get("retryable") is not False
        or not isinstance(details, dict)
        or details.get("resource") != resource
        or "canonical_state" in result
    ):
        raise ValueError("deleted branch must remain inconclusive with resource_not_found")


def validate_branch_ref_reconcile_batch(
    exit_status: int,
    requested_events: list[dict[str, Any]],
    batch: dict[str, Any],
    *,
    owner: str,
    repository: str,
    branch: str,
    current_head_sha: str | None,
    expected_present: bool = True,
) -> list[dict[str, Any]]:
    expected_exit_status = 0 if expected_present else 1
    if type(exit_status) is not int or exit_status != expected_exit_status:
        raise ValueError("branch ref reconciliation batch exit status does not match expected state")
    expected_batch_verdict = "current_state_read" if expected_present else "inconclusive"
    if (
        not isinstance(batch, dict)
        or batch.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or batch.get("api_version") != "forgejo-rest-v1"
        or batch.get("verdict") != expected_batch_verdict
    ):
        raise ValueError("branch ref reconciliation batch envelope is invalid")
    if type(batch.get("upstream_read_attempts")) is not int or batch["upstream_read_attempts"] != 1:
        raise ValueError("same-resource branch ref batch must use one upstream read")
    if not isinstance(requested_events, list) or len(requested_events) != 2:
        raise ValueError("batch must contain unique create and delete deliveries in order")
    delivery_ids: list[str] = []
    for event, expected_kind in zip(requested_events, ("create", "delete"), strict=True):
        if (
            not isinstance(event, dict)
            or event.get("event") != expected_kind
            or event.get("branch") != branch
            or not isinstance(event.get("delivery_id"), str)
            or not event["delivery_id"]
        ):
            raise ValueError("batch must contain unique create and delete deliveries in order")
        delivery_ids.append(event["delivery_id"])
    if len(set(delivery_ids)) != len(delivery_ids):
        raise ValueError("batch must contain unique create and delete deliveries in order")
    results = batch.get("results")
    if not isinstance(results, list) or len(results) != len(requested_events):
        raise ValueError("branch ref reconciliation result count is invalid")
    expected_resource = {
        "kind": "branch",
        "owner": owner,
        "repository": repository,
        "branch": branch,
    }
    expected_html_url = f"{ORIGIN}/{owner}/{repository}/src/branch/{quote(branch, safe='')}"
    checked: list[dict[str, Any]] = []
    for event, result in zip(requested_events, results, strict=True):
        emitted.validate_reconciliation_delivery_binding(event, result)
        if not isinstance(result, dict):
            raise ValueError("branch ref batch item is not an object")
        if expected_present:
            canonical = result.get("canonical_state")
            if (
                result.get("verdict") != "current_state_read"
                or result.get("resource") != expected_resource
                or not isinstance(current_head_sha, str)
                or not isinstance(canonical, dict)
                or canonical.get("state") != "present"
                or canonical.get("head_sha") != current_head_sha
                or canonical.get("html_url") != expected_html_url
            ):
                raise ValueError("branch ref batch item is not the current branch state")
        else:
            error = result.get("error")
            details = error.get("details") if isinstance(error, dict) else None
            action = error.get("action") if isinstance(error, dict) else None
            if (
                result.get("verdict") != "inconclusive"
                or not isinstance(error, dict)
                or error.get("code") != "resource_not_found"
                or error.get("retryable") is not False
                or not isinstance(details, dict)
                or details.get("resource") != expected_resource
                or not isinstance(action, str)
                or "HTTP 404 does not distinguish an absent branch" not in action
                or "canonical_state" in result
            ):
                raise ValueError("branch ref batch absence inconclusive contract is invalid")
        checked.append(result)
    return checked


def validate_branch_ref_recreated_retry(
    initial_exit_status: int,
    initial_batch: dict[str, Any],
    retry_exit_status: int,
    retry_batch: dict[str, Any],
    requested_events: list[dict[str, Any]],
    *,
    owner: str,
    repository: str,
    branch: str,
    current_head_sha: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Require the same captured deliveries to move from ambiguous 404 to current state."""
    first = validate_branch_ref_reconcile_batch(
        initial_exit_status,
        requested_events,
        initial_batch,
        owner=owner,
        repository=repository,
        branch=branch,
        current_head_sha=None,
        expected_present=False,
    )
    retried = validate_branch_ref_reconcile_batch(
        retry_exit_status,
        requested_events,
        retry_batch,
        owner=owner,
        repository=repository,
        branch=branch,
        current_head_sha=current_head_sha,
        expected_present=True,
    )
    initial_ids = [item.get("delivery_id") for item in first]
    retry_ids = [item.get("delivery_id") for item in retried]
    if initial_ids != retry_ids:
        raise ValueError("recreated-branch retry changed the original delivery IDs or order")
    return first, retried


def validate_recreate_retry_options(
    *,
    include_create: bool,
    expect_absent_after_delete: bool,
    retry_after_recreate: bool,
    advance_head_after_delete: bool = False,
) -> None:
    if retry_after_recreate and not (include_create and expect_absent_after_delete):
        raise ValueError(
            "--retry-after-recreate requires --include-create and "
            "--expect-branch-absent-after-delete"
        )
    if advance_head_after_delete and not retry_after_recreate:
        raise ValueError("--advance-head-after-delete requires --retry-after-recreate")


def run(args: argparse.Namespace) -> dict[str, Any]:
    retry_after_recreate = bool(getattr(args, "retry_after_recreate", False))
    try:
        validate_recreate_retry_options(
            include_create=bool(args.include_create),
            expect_absent_after_delete=bool(args.expect_branch_absent_after_delete),
            retry_after_recreate=retry_after_recreate,
            advance_head_after_delete=bool(getattr(args, "advance_head_after_delete", False)),
        )
    except ValueError as error:
        raise RuntimeError(str(error)) from None
    binary_arg = Path(args.forgejo_binary)
    if binary_arg.is_symlink():
        raise RuntimeError("Forgejo binary must not be a symlink")
    forgejo = binary_arg.resolve(strict=True)
    repo = Path(args.repo_root).resolve(strict=True)
    state = Path(args.state_dir)
    evidence_dir = Path(args.evidence_dir).resolve()
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("state dir must be a new direct child of /tmp")
    if evidence_dir.exists():
        raise RuntimeError("refusing to overwrite existing evidence directory")
    if not (repo / "Cargo.toml").is_file() or not forgejo.is_file():
        raise RuntimeError("a Cargo repository and regular Forgejo binary are required")
    hook_event_selectors = ("create", "delete") if args.include_create else ("delete",)
    if shutil.which("git") is None or shutil.which("rustc") is None or shutil.which("bwrap") is None:
        raise RuntimeError("git, rustc and Bubblewrap are required")
    branch = validate_branch_name(getattr(args, "branch_name", BRANCH))
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", 3130)) == 0:
            raise RuntimeError("127.0.0.1:3130 is already in use")

    version = subprocess.run(
        [str(forgejo), "--version"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env={"PATH": "/usr/bin:/bin", "HOME": "/tmp"},
        timeout=10,
        check=False,
    )
    if version.returncode or FORGEJO_VERSION not in version.stdout:
        raise RuntimeError("this E2E requires the locally verified Forgejo 17.0.0 binary")

    state.mkdir(mode=0o700)
    (state / "home").mkdir(mode=0o700)
    (state / "tmp").mkdir(mode=0o700)
    (state / "log").mkdir(mode=0o700)
    helpers.forgejo_helpers.create_app_ini(state)
    emitted.private_config_with_test_allowlist(state)
    evidence_dir.mkdir(mode=0o700, parents=True)

    tokens: list[str] = []
    server_log = (state / "log/forgejo-server.log").open("wb")
    receiver_log = (state / "log/webhook-receiver.log").open("wb")
    server: subprocess.Popen[bytes] | None = None
    receiver: subprocess.Popen[bytes] | None = None
    receiver_port: int | None = None
    receiver_bin: Path | None = None
    reconcile_bin: Path | None = None
    build_provenance: dict[str, Any] | None = None
    owner_api: helpers.LocalForgejoApi | None = None
    hook_id: int | None = None
    hook_disabled = False
    hook_readback: dict[str, Any] | None = None
    expected_hook_url: str | None = None
    webhook_secret: str | None = None
    owner_token: str | None = None
    reader_token: str | None = None
    event: dict[str, Any] | None = None
    create_event: dict[str, Any] | None = None
    hook_tasks: list[dict[str, Any]] = []
    reconciliation: dict[str, Any] | None = None
    reconciliation_attempts: list[dict[str, Any]] = []
    retry_reconciliation: dict[str, Any] | None = None
    retry_exit_status: int | None = None
    retry_attempts: list[dict[str, Any]] = []
    retry_branch_readback: dict[str, Any] | None = None
    retry_hook_tasks: list[dict[str, Any]] | None = None
    branch_readback: dict[str, Any] | None = None
    journal_path = state / "data/webhook-inbox.jsonl"
    stage = "isolated_build"
    failure: str | None = None
    cleanup_failure: str | None = None
    server_stop: dict[str, Any] | None = None
    receiver_stop: dict[str, Any] | None = None
    result: dict[str, Any] | None = None

    try:
        receiver_bin, reconcile_bin, build_provenance = helpers.build_local_binaries(
            repo, state, evidence_dir
        )
        stage = "start_disposable_forgejo"
        server = subprocess.Popen(
            [str(forgejo), "--work-path", str(state), "--config", str(state / "custom/conf/app.ini"), "web"],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=server_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        helpers.forgejo_helpers.wait_for_health(server)
        helpers.forgejo_helpers.assert_forgejo_listener(server)

        stage = "provision_private_repository_and_reader"
        helpers.admin_command(
            forgejo,
            state,
            [
                "admin", "user", "create", "--username", OWNER,
                "--random-password", "--email", f"{OWNER}@local.invalid",
                "--admin", "--must-change-password=false",
            ],
        )
        bootstrap_token = helpers.create_access_token(
            forgejo, state, OWNER, "native-delete-bootstrap", "write:user,write:repository"
        )
        tokens.append(bootstrap_token)
        bootstrap_api = helpers.LocalForgejoApi(bootstrap_token)
        created_repo, _ = bootstrap_api.request(
            "POST",
            "/user/repos",
            {
                "name": REPOSITORY,
                "description": "Disposable local native branch-delete reconciliation proof",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if created_repo.get("private") is not True or created_repo.get("full_name") != f"{OWNER}/{REPOSITORY}":
            raise RuntimeError("Forgejo did not create the expected private repository")
        tokens.remove(bootstrap_token)
        bootstrap_api = None
        bootstrap_token = None
        owner_token = helpers.create_access_token(
            forgejo, state, OWNER, "native-delete-owner", "read:user,read:repository,write:repository"
        )
        tokens.append(owner_token)
        owner_api = helpers.LocalForgejoApi(owner_token)
        helpers.admin_command(
            forgejo,
            state,
            [
                "admin", "user", "create", "--username", READER,
                "--random-password", "--email", f"{READER}@local.invalid",
                "--must-change-password=false",
            ],
        )
        reader_token = helpers.create_access_token(
            forgejo, state, READER, "native-delete-read-only", "read:user,read:repository"
        )
        tokens.append(reader_token)
        reader_api = helpers.LocalForgejoApi(reader_token)
        owner_api.request(
            "PUT",
            f"/repos/{OWNER}/{REPOSITORY}/collaborators/{READER}",
            {"permission": "read"},
        )
        reader_repo, _ = reader_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}")
        permissions = reader_repo.get("permissions")
        if (
            reader_repo.get("private") is not True
            or not isinstance(permissions, dict)
            or permissions.get("push") is not False
            or permissions.get("pull") is not True
        ):
            raise RuntimeError("Forgejo did not confirm a private repository and read-only reader")

        stage = "start_receiver_and_subscribe_to_branch_ref_events"
        webhook_secret = secrets.token_hex(32)
        secret_path = state / "webhook.secret"
        helpers.forgejo_helpers.write_private(secret_path, webhook_secret.encode("ascii"))
        attestation_seed_path = state / "receiver-attestation.seed"
        helpers.forgejo_helpers.write_private(attestation_seed_path, secrets.token_bytes(32))
        receiver_attestation_public_key = helpers.derive_receiver_attestation_public_key(
            receiver_bin, state, attestation_seed_path
        )
        helpers.forgejo_helpers.write_private(
            state / "receiver-attestation.public", receiver_attestation_public_key.encode("ascii")
        )
        receiver_port = helpers.select_receiver_port()
        receiver_address = f"127.0.0.1:{receiver_port}"
        expected_hook_url = f"http://{receiver_address}/forgejo/webhook"
        receiver = subprocess.Popen(
            [
                str(receiver_bin),
                receiver_address,
                str(secret_path),
                str(journal_path),
                str(attestation_seed_path),
            ],
            cwd=state,
            env=helpers.private_command_environment(state),
            stdout=receiver_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        helpers.wait_for_listener(receiver, receiver_port)
        hook_response, _ = owner_api.request(
            "POST",
            f"/repos/{OWNER}/{REPOSITORY}/hooks",
            {
                "type": "gitea",
                "config": {
                    "url": expected_hook_url,
                    "content_type": "json",
                    "secret": webhook_secret,
                },
                "events": list(hook_event_selectors),
                "active": True,
            },
        )
        hook_id = hook_response.get("id")
        if not isinstance(hook_id, int) or hook_id < 1 or hook_response.get("active") is not True:
            raise RuntimeError("Forgejo did not create the expected active delete webhook")
        hook_config, _ = owner_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}")
        hook_readback = emitted.hook_readback_record(
            hook_id, hook_config, expected_hook_url, hook_event_selectors
        )
        if hook_readback["configuration_matches_expected"] is not True:
            raise RuntimeError("Forgejo branch-ref hook readback differs from the requested configuration")

        stage = "create_branch_with_local_git_http"
        checkout = state / "git-checkout"
        askpass = state / "git-askpass"
        helpers.forgejo_helpers.write_private(
            askpass,
            b'#!/bin/sh\ncase "$1" in\n  *Username*) printf "%s\\n" "$FORGEJO_TEST_USER" ;;\n'
            b'  *Password*) printf "%s\\n" "$FORGEJO_TEST_TOKEN" ;;\n  *) exit 1 ;;\nesac\n',
        )
        os.chmod(askpass, 0o700)
        git_env = helpers.private_command_environment(state)
        git_env.update(
            {
                "GIT_ASKPASS": str(askpass),
                "GIT_TERMINAL_PROMPT": "0",
                "GIT_CONFIG_GLOBAL": "/dev/null",
                "GIT_CONFIG_NOSYSTEM": "1",
                "FORGEJO_TEST_USER": OWNER,
                "FORGEJO_TEST_TOKEN": owner_token,
            }
        )
        repository_url = f"{ORIGIN}/{OWNER}/{REPOSITORY}.git"
        run_git(
            ["clone", "--quiet", "--single-branch", "--branch", "main", repository_url, str(checkout)],
            cwd=None,
            env=git_env,
            stage="clone",
        )
        run_git(["config", "user.name", "Forgejo Native Delete E2E"], cwd=checkout, env=git_env, stage="configure author")
        run_git(["config", "user.email", "forgejo-native-delete@local.invalid"], cwd=checkout, env=git_env, stage="configure email")
        run_git(["checkout", "--quiet", "-b", branch], cwd=checkout, env=git_env, stage="create branch")
        (checkout / "native-delete-proof.txt").write_text(
            "A branch used only to test Forgejo's native delete webhook.\n", encoding="utf-8"
        )
        run_git(["add", "native-delete-proof.txt"], cwd=checkout, env=git_env, stage="stage change")
        run_git(["commit", "--quiet", "-m", "Add native branch-delete proof"], cwd=checkout, env=git_env, stage="commit change")
        commit_sha = run_git(["rev-parse", "HEAD"], cwd=checkout, env=git_env, stage="read commit")
        run_git(["push", "--quiet", "--set-upstream", "origin", branch], cwd=checkout, env=git_env, stage="push branch")
        if args.include_create:
            create_records = emitted.wait_for_journal_count(journal_path, 1)
            if len(create_records) != 1:
                raise RuntimeError("Forgejo emitted an unexpected number of branch-create events")
            create_event = emitted.parse_live_create_delivery_record(
                create_records[0], owner=OWNER, repository=REPOSITORY
            )
            if create_event["ref"] != branch or create_event["sha"] != commit_sha:
                raise RuntimeError("Forgejo's create event did not bind to the new branch head")
            with (checkout / "native-delete-proof.txt").open("a", encoding="utf-8") as branch_file:
                branch_file.write("A later commit makes the create payload stale.\n")
            run_git(["add", "native-delete-proof.txt"], cwd=checkout, env=git_env, stage="stage branch advance")
            run_git(["commit", "--quiet", "-m", "Advance native create proof branch"], cwd=checkout, env=git_env, stage="commit branch advance")
            current_commit_sha = run_git(["rev-parse", "HEAD"], cwd=checkout, env=git_env, stage="read advanced commit")
            run_git(["push", "--quiet", "origin", branch], cwd=checkout, env=git_env, stage="push branch advance")
            if len(emitted.read_journal_records(journal_path)) != 1:
                raise RuntimeError("branch advance emitted an unexpected subscribed event")
        else:
            current_commit_sha = commit_sha
            if emitted.read_journal_records(journal_path):
                raise RuntimeError("delete-only hook received an event for branch creation")

        stage = "delete_branch_and_capture_native_event"
        run_git(["push", "--quiet", "origin", "--delete", branch], cwd=checkout, env=git_env, stage="delete branch")
        remote_refs_after_delete = run_git(
            ["ls-remote", "--heads", "origin", f"refs/heads/{branch}"],
            cwd=checkout,
            env=git_env,
            stage="confirm deleted remote ref",
        )
        if remote_refs_after_delete:
            raise RuntimeError("Forgejo still advertises the branch after the delete push")
        expected_delivery_count = 2 if args.include_create else 1
        records = emitted.wait_for_journal_count(journal_path, expected_delivery_count)
        if len(records) != expected_delivery_count:
            raise RuntimeError("Forgejo emitted an unexpected number of delete events")
        event = emitted.parse_live_delete_delivery_record(
            records[-1], owner=OWNER, repository=REPOSITORY
        )
        if event["ref"] != branch or event["branch"] != branch:
            raise RuntimeError("Forgejo's delete event did not bind to the deleted branch")
        if args.include_create:
            observed_create = emitted.parse_live_create_delivery_record(
                records[0], owner=OWNER, repository=REPOSITORY
            )
            if observed_create != create_event:
                raise RuntimeError("Forgejo's create journal event changed before delete")
            create_event = observed_create
        hook_tasks = emitted.wait_for_hook_tasks(
            state / "data/forgejo.db", hook_id, expected_delivery_count
        )
        if len(hook_tasks) != expected_delivery_count:
            raise RuntimeError("Forgejo recorded an unexpected number of branch ref deliveries")
        emitted.validate_hook_task_bindings(
            hook_tasks, ([create_event, event] if create_event is not None else [event])
        )

        branch_path = quote(branch, safe="")
        if args.expect_branch_absent_after_delete:
            stage = "reconcile_deleted_branch_after_native_event"
            branch_readback = {
                "status": "not_separately_read",
                "remote_ref_absent_after_delete": remote_refs_after_delete == "",
                "reader_permission": "read_only",
                "response_body_recorded": False,
            }
            current_sha = None
        else:
            stage = "recreate_branch_before_reconciling_stale_delete"
            run_git(["push", "--quiet", "origin", branch], cwd=checkout, env=git_env, stage="recreate branch")
            branch_response, _ = reader_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{branch_path}"
            )
            current_sha = branch_response.get("commit", {}).get("id")
            if branch_response.get("name") != branch or current_sha != current_commit_sha:
                raise RuntimeError("Forgejo did not confirm the recreated branch's current commit")
            if create_event is not None and create_event["sha"] == current_sha:
                raise RuntimeError("create payload unexpectedly remained current after branch advance")
            branch_readback = {
                "status": 200,
                "name": branch_response["name"],
                "commit_sha": current_sha,
                "reader_permission": "read_only",
            }
        journal_before = journal_path.read_bytes()
        stage = "invoke_reconciler_after_native_branch_ref_events"
        events_to_reconcile = [create_event, event] if create_event is not None else [event]
        if create_event is not None:
            status, reconciliation, remaining_credential = native_push.run_reconcile_batch(
                reconcile_bin,
                state,
                journal_path,
                [target_event["delivery_id"] for target_event in events_to_reconcile],
                reader_token,
            )
            if remaining_credential:
                raise RuntimeError("batch reconciliation CLI did not consume its private credential")
            checked_results = validate_branch_ref_reconcile_batch(
                status,
                events_to_reconcile,
                reconciliation,
                owner=OWNER,
                repository=REPOSITORY,
                branch=branch,
                current_head_sha=current_sha,
                expected_present=not args.expect_branch_absent_after_delete,
            )
            reconciliation_attempts.extend(
                {
                    "event": target_event["event"],
                    "exit_status": status,
                    "batch_upstream_read_attempts": reconciliation["upstream_read_attempts"],
                    "result": checked_result,
                }
                for target_event, checked_result in zip(
                    events_to_reconcile, checked_results, strict=True
                )
            )
        else:
            status, reconciliation = helpers.run_reconcile(
                reconcile_bin, state, journal_path, event["delivery_id"], reader_token
            )
            validate_delete_reconciliation_result(
                status,
                reconciliation,
                delivery_id=event["delivery_id"],
                owner=OWNER,
                repository=REPOSITORY,
                branch=branch,
                expected_present=not args.expect_branch_absent_after_delete,
                expected_sha=current_sha,
            )
            emitted.validate_reconciliation_delivery_binding(event, reconciliation)
            reconciliation_attempts.append(
                {"event": event["event"], "exit_status": status, "result": reconciliation}
            )
        helpers.write_safe_json(
            evidence_dir / "reconciliation-attempts.json", reconciliation_attempts, tokens
        )
        journal_after = journal_path.read_bytes()
        if journal_before != journal_after:
            raise RuntimeError("reconciliation modified the append-only webhook journal")

        if retry_after_recreate:
            stage = "disable_hook_before_recreating_branch_for_same_delivery_retry"
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                emitted.hook_deactivation_patch(hook_event_selectors),
            )
            hook_config, _ = owner_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
            )
            hook_readback = emitted.hook_readback_record(
                hook_id, hook_config, expected_hook_url, hook_event_selectors
            )
            hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
            if not hook_disabled or not hook_readback["configuration_matches_expected"]:
                raise RuntimeError("Forgejo did not confirm hook deactivation before branch recreation")

            stage = "recreate_branch_after_inconclusive_404_and_read_current_state"
            retry_commit_sha = current_commit_sha
            if args.advance_head_after_delete:
                with (checkout / "native-delete-proof.txt").open("a", encoding="utf-8") as branch_file:
                    branch_file.write("A new commit was created after the inconclusive delete read.\n")
                run_git(["add", "native-delete-proof.txt"], cwd=checkout, env=git_env, stage="stage post-delete branch advance")
                run_git(["commit", "--quiet", "-m", "Advance branch after inconclusive delete read"], cwd=checkout, env=git_env, stage="commit post-delete branch advance")
                retry_commit_sha = run_git(["rev-parse", "HEAD"], cwd=checkout, env=git_env, stage="read post-delete commit")
                if retry_commit_sha == current_commit_sha:
                    raise RuntimeError("post-delete commit did not advance the branch head")
            run_git(["push", "--quiet", "origin", branch], cwd=checkout, env=git_env, stage="recreate branch after 404")
            retry_branch_response, _ = reader_api.request(
                "GET", f"/repos/{OWNER}/{REPOSITORY}/branches/{branch_path}"
            )
            retry_head_sha = retry_branch_response.get("commit", {}).get("id")
            if retry_branch_response.get("name") != branch or retry_head_sha != retry_commit_sha:
                raise RuntimeError("Forgejo did not confirm the recreated branch's current commit")
            retry_branch_readback = {
                "status": 200,
                "name": retry_branch_response["name"],
                "commit_sha": retry_head_sha,
                "new_commit_after_delete": args.advance_head_after_delete,
                "reader_permission": "read_only",
            }
            retry_hook_tasks = emitted.hook_task_readback(state / "data/forgejo.db", hook_id)
            if retry_hook_tasks != hook_tasks:
                raise RuntimeError("branch recreation produced an additional or changed webhook task")
            retry_journal_before = journal_path.read_bytes()
            if retry_journal_before != journal_before:
                raise RuntimeError("branch recreation changed the append-only webhook journal")

            stage = "retry_same_webhook_deliveries_after_branch_recreation"
            retry_exit_status, retry_reconciliation, remaining_credential = native_push.run_reconcile_batch(
                reconcile_bin,
                state,
                journal_path,
                [target_event["delivery_id"] for target_event in events_to_reconcile],
                reader_token,
            )
            if remaining_credential:
                raise RuntimeError("retry batch reconciliation CLI did not consume its private credential")
            first_checked, retry_checked = validate_branch_ref_recreated_retry(
                status,
                reconciliation,
                retry_exit_status,
                retry_reconciliation,
                events_to_reconcile,
                owner=OWNER,
                repository=REPOSITORY,
                branch=branch,
                current_head_sha=retry_head_sha,
            )
            retry_attempts = [
                {
                    "event": target_event["event"],
                    "exit_status": retry_exit_status,
                    "batch_upstream_read_attempts": retry_reconciliation["upstream_read_attempts"],
                    "result": checked_result,
                }
                for target_event, checked_result in zip(
                    events_to_reconcile, retry_checked, strict=True
                )
            ]
            retry_journal_after = journal_path.read_bytes()
            if retry_journal_after != journal_before:
                raise RuntimeError("retry reconciliation modified the append-only webhook journal")
            # Recheck the first round's validated contract while preserving both raw batch results.
            if [item["delivery_id"] for item in first_checked] != [
                target_event["delivery_id"] for target_event in events_to_reconcile
            ]:
                raise RuntimeError("first reconciliation round lost the original delivery ordering")
            helpers.write_safe_json(
                evidence_dir / "reconciliation-retry.json",
                {
                    "initial_exit_status": status,
                    "initial_verdict": reconciliation.get("verdict"),
                    "retry_exit_status": retry_exit_status,
                    "retry_result": retry_reconciliation,
                    "delivery_ids": [target_event["delivery_id"] for target_event in events_to_reconcile],
                    "delivery_ids_unchanged": True,
                    "webhook_journal_unchanged": retry_journal_before == retry_journal_after,
                    "hook_tasks_unchanged": retry_hook_tasks == hook_tasks,
                    "branch_readback": retry_branch_readback,
                },
                tokens,
            )
            helpers.write_safe_json(evidence_dir / "branch-retry-readback.json", retry_branch_readback, tokens)
            helpers.write_safe_json(evidence_dir / "hook-retry-task-readback.json", retry_hook_tasks, tokens)
            helpers.write_safe_json(evidence_dir / "reconciliation-retry-attempts.json", retry_attempts, tokens)

        stage = "deactivate_delete_hook_and_read_back"
        if not hook_disabled:
            owner_api.request(
                "PATCH",
                f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                emitted.hook_deactivation_patch(hook_event_selectors),
            )
            hook_config, _ = owner_api.request("GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}")
            hook_readback = emitted.hook_readback_record(
                hook_id, hook_config, expected_hook_url, hook_event_selectors
            )
            hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
            if not hook_disabled or not hook_readback["configuration_matches_expected"]:
                raise RuntimeError("Forgejo did not confirm branch-ref hook deactivation")

        result = {
            "schema_version": 1,
            "verdict": "passed",
            "forgejo": {
                "version": version.stdout.strip(),
                "binary_sha256": helpers.sha256_file(forgejo),
                "listener": "127.0.0.1:3130",
                "webhook_allowlist": "127.0.0.1",
            },
            "repository": {
                "full_name": f"{OWNER}/{REPOSITORY}",
                "private": True,
                "reader_permission": permissions,
            },
            "branch": {
                "name": branch,
                "initial_commit_sha": commit_sha,
                "current_commit_sha": current_commit_sha,
                "remote_ref_absent_after_delete": remote_refs_after_delete == "",
                "recreated_before_reconciliation": not args.expect_branch_absent_after_delete,
                "current_api_sha": current_sha,
                "recreated_after_initial_inconclusive_read": retry_after_recreate,
                "retry_api_sha": retry_branch_readback.get("commit_sha") if retry_branch_readback else None,
                "advanced_after_delete_before_retry": bool(getattr(args, "advance_head_after_delete", False)),
            },
            "native_create_delivery": (
                {
                    "event": create_event,
                    "payload_sha_is_stale_at_reconciliation": create_event["sha"] != current_sha,
                    "forgejo_delivery_tasks": hook_tasks,
                }
                if create_event is not None
                else None
            ),
            "native_delete_delivery": {
                "event": event,
                "forgejo_delivery_tasks": hook_tasks,
                "hook": {**hook_readback, "secret_value_recorded": False},
            },
            "reconciliation": {
                "exit_status": status,
                "result": reconciliation,
                "attempts": reconciliation_attempts,
                "batch_mode": create_event is not None,
                "upstream_read_attempts": reconciliation.get("upstream_read_attempts"),
                "credential_source": "private_inherited_pipe_fd",
                "credential_recorded": False,
                "journal_unchanged": journal_before == journal_after,
                "retry_after_recreate": (
                    {
                        "exit_status": retry_exit_status,
                        "result": retry_reconciliation,
                        "attempts": retry_attempts,
                        "upstream_read_attempts": retry_reconciliation.get("upstream_read_attempts"),
                        "same_delivery_ids": True,
                        "journal_unchanged": journal_path.read_bytes() == journal_before,
                        "hook_tasks_unchanged": retry_hook_tasks == hook_tasks,
                    }
                    if retry_reconciliation is not None
                    else None
                ),
            },
            "limitations": [
                "Only a disposable local Forgejo instance and one branch were used; no external or production service was contacted.",
                "A branch endpoint HTTP 404 remains inconclusive because it can also mask an inaccessible private resource.",
                "Forgejo source was inspected at the recorded v17.0.0 commit, but this binary's build commit is not established by the local binary metadata.",
                "This proves native event capture and current-state reconciliation only; it does not prove an LLM, AgentOS, PR review, merge authority, or comparative productivity gain.",
            ],
        }
        stage = "complete"
    except Exception as error:
        failure = type(error).__name__
    finally:
        if hook_id is not None and owner_api is not None and not hook_disabled:
            try:
                owner_api.request(
                    "PATCH",
                    f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}",
                    emitted.hook_deactivation_patch(hook_event_selectors),
                )
                hook_config, _ = owner_api.request(
                    "GET", f"/repos/{OWNER}/{REPOSITORY}/hooks/{hook_id}"
                )
                hook_readback = emitted.hook_readback_record(
                    hook_id, hook_config, expected_hook_url, hook_event_selectors
                )
                hook_disabled = hook_readback["deactivation_readback_confirmed"] is True
                if not hook_disabled:
                    cleanup_failure = "branch_ref_hook_deactivation_not_confirmed"
            except Exception as error:
                cleanup_failure = type(error).__name__
        receiver_stop = helpers.stop_process(receiver)
        server_stop = helpers.stop_process(server)
        receiver_log.close()
        server_log.close()

    cleanup = {
        "forgejo": {
            "stop": server_stop,
            "listener_addresses_after_stop": helpers.listener_addresses_for_port(3130),
        },
        "receiver": {
            "stop": receiver_stop,
            "listener_addresses_after_stop": (
                helpers.listener_addresses_for_port(receiver_port) if receiver_port else []
            ),
        },
        "hook_disabled": hook_disabled,
        "hook_readback": hook_readback,
        "cleanup_failure": cleanup_failure,
        "private_state_dir": str(state),
    }
    if result is not None:
        result["cleanup"] = cleanup
        source_files = sorted(
            {
                repo / "Cargo.toml",
                repo / "Cargo.lock",
                Path(__file__).resolve(),
                (SCRIPT_DIR / "run_forgejo_live_webhook_reconcile_e2e.py").resolve(),
                (SCRIPT_DIR / "run_forgejo_live_webhook_reconcile_emitted.py").resolve(),
                (repo / "tests/test_forgejo_emitted_webhook_e2e.py").resolve(),
                *(repo / "src").rglob("*.rs"),
            }
        )
        result["provenance"] = {
            "source_sha256": {
                str(path.relative_to(repo)): helpers.sha256_file(path)
                for path in source_files
            },
            "forgejo_binary_sha256": helpers.sha256_file(forgejo),
            "receiver_sha256": helpers.sha256_file(receiver_bin),
            "reconcile_cli_sha256": helpers.sha256_file(reconcile_bin),
            "cargo_build": build_provenance,
            "webhook_secret_recorded": False,
            "credentials_recorded": False,
        }
        helpers.write_safe_json(evidence_dir / "delete-event.json", event, tokens)
        if create_event is not None:
            helpers.write_safe_json(evidence_dir / "create-event.json", create_event, tokens)
        helpers.write_safe_json(evidence_dir / "hook-tasks.json", hook_tasks, tokens)
        helpers.write_safe_json(evidence_dir / "reconciliation.json", reconciliation, tokens)
        helpers.write_safe_json(evidence_dir / "branch-readback.json", branch_readback, tokens)
    else:
        result = {
            "schema_version": 1,
            "verdict": "failed",
            "stage": stage,
            "failure_type": failure,
            "cleanup": cleanup,
        }
    helpers.write_safe_json(evidence_dir / "result.json", result, tokens)
    helpers.write_checksums(evidence_dir)
    if failure is not None:
        raise RuntimeError(f"native branch-delete E2E failed at {stage} ({failure})")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--repo-root", default=str(SCRIPT_DIR.parent))
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument(
        "--branch-name",
        default=BRANCH,
        help="valid Git branch name for the disposable native webhook exercise",
    )
    parser.add_argument("--expect-branch-absent-after-delete", action="store_true")
    parser.add_argument(
        "--retry-after-recreate",
        action="store_true",
        help="after an inconclusive 404, disable the hook, recreate the branch, and retry the same deliveries",
    )
    parser.add_argument(
        "--advance-head-after-delete",
        action="store_true",
        help="commit a new head after the inconclusive read and before recreating the branch",
    )
    parser.add_argument(
        "--include-create",
        action="store_true",
        help="also capture a native create event, advance the branch, and reconcile both events",
    )
    args = parser.parse_args()
    try:
        validate_recreate_retry_options(
            include_create=args.include_create,
            expect_absent_after_delete=args.expect_branch_absent_after_delete,
            retry_after_recreate=args.retry_after_recreate,
            advance_head_after_delete=args.advance_head_after_delete,
        )
    except ValueError as error:
        parser.error(str(error))
    result = run(args)
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
