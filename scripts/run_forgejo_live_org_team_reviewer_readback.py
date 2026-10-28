#!/usr/bin/env python3
"""Verify a non-empty team-reviewer readback against disposable local Forgejo."""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import time
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import run_forgejo_live_webhook_reconcile_e2e as fixture  # noqa: E402
import run_forgejo_live_webhook_reconcile_emitted as emitted  # noqa: E402
import run_forgejo_live_pr_merge_e2e as forgejo_helpers  # noqa: E402
import run_forgejo_live_webhook_native_replay_e2e as native_replay  # noqa: E402


FORGEJO_VERSION = "17.0.0+gitea-1.22.0"
ADMIN = fixture.OWNER
REVIEWER = fixture.READER
ORGANIZATION = "forge-team-org"
REPOSITORY = "team-review-readback"
TEAM = "reviewers"
BRANCH = "agent/team-review"
GIT_WRITE_PROBE_BRANCH = "agent/read-only-git-write-probe"
GIT_WRITE_CONTROL_BRANCH = "agent/read-write-control-probe"
PR_COMMENT_BODY = "Synthetic local PR comment used only for Forgejo adapter batch readback."


def validate_team_readback(
    *,
    request_status: int | None,
    pull_request: Any,
    expected_team_id: int,
    expected_team_name: str,
) -> list[dict[str, Any]]:
    if request_status != 201:
        raise RuntimeError("Forgejo did not accept the team-review request with HTTP 201")
    if (
        type(expected_team_id) is not int
        or expected_team_id < 1
        or not isinstance(expected_team_name, str)
        or not expected_team_name
    ):
        raise ValueError("expected Forgejo team identity is malformed")
    projection = emitted.normalize_requested_reviewer_teams_readback(pull_request)
    expected = [{"id": expected_team_id, "name": expected_team_name}]
    if projection != expected:
        raise RuntimeError("Forgejo current PR readback did not contain the requested team")
    return projection


def validate_pr_comment_batch_result(
    value: Any,
    *,
    pull_delivery_id: str,
    comment_delivery_id: str,
    owner: str,
    repository: str,
    pull_request_number: int,
    comment_id: int,
    expected_head_sha: str,
    expected_base_sha: str,
    expected_team_id: int,
    expected_team_name: str,
    expected_comment_body: str,
) -> dict[str, Any]:
    """Require a native PR/comment batch to share fresh parent state and keep comment trust explicit."""
    if (
        not isinstance(value, dict)
        or value.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or value.get("verdict") != "current_state_read"
        or value.get("upstream_read_attempts") != 2
    ):
        raise RuntimeError("Forgejo PR/comment batch did not return the expected two-read snapshot")
    results = value.get("results")
    if not isinstance(results, list) or len(results) != 2:
        raise RuntimeError("Forgejo PR/comment batch did not return exactly two ordered results")
    expected_ids = [pull_delivery_id, comment_delivery_id]
    expected_resource = {
        "kind": "pull_request",
        "owner": owner,
        "repository": repository,
        "number": pull_request_number,
    }
    if (
        not isinstance(pull_delivery_id, str)
        or not pull_delivery_id
        or not isinstance(comment_delivery_id, str)
        or not comment_delivery_id
        or pull_delivery_id == comment_delivery_id
        or type(pull_request_number) is not int
        or pull_request_number < 1
        or type(comment_id) is not int
        or comment_id < 1
        or type(expected_team_id) is not int
        or expected_team_id < 1
        or not isinstance(expected_team_name, str)
        or not expected_team_name
        or not isinstance(expected_comment_body, str)
        or not expected_comment_body
    ):
        raise ValueError("expected Forgejo PR/comment batch bindings are malformed")
    for digest in (expected_head_sha, expected_base_sha):
        if (
            not isinstance(digest, str)
            or len(digest) not in {40, 64}
            or any(character not in "0123456789abcdef" for character in digest)
            or set(digest) == {"0"}
        ):
            raise ValueError("expected Forgejo PR/comment commit binding is malformed")

    expected_teams = [{"id": expected_team_id, "name": expected_team_name}]
    snapshots = []
    for delivery_id, result in zip(expected_ids, results, strict=True):
        resource = result.get("resource") if isinstance(result, dict) else None
        canonical = result.get("canonical_state") if isinstance(result, dict) else None
        if (
            not isinstance(result, dict)
            or result.get("delivery_id") != delivery_id
            or result.get("verdict") != "current_state_read"
            or resource != expected_resource
            or not isinstance(canonical, dict)
            or canonical.get("state") != "open"
            or canonical.get("merged") is not False
            or canonical.get("head_sha") != expected_head_sha
            or canonical.get("base_sha") != expected_base_sha
            or canonical.get("requested_reviewers_teams") != expected_teams
        ):
            raise RuntimeError("Forgejo batch result lost its ordered current PR/team binding")
        snapshots.append(canonical)
    if snapshots[0].get("comment") is not None:
        raise RuntimeError("the PR event result unexpectedly projected a comment")
    comment = snapshots[1].get("comment")
    if (
        not isinstance(comment, dict)
        or comment.get("id") != comment_id
        or comment.get("body") != expected_comment_body
        or comment.get("body_trust") != "untrusted_user_content"
    ):
        raise RuntimeError("Forgejo comment readback lost its identity, current body, or untrusted label")
    return {
        "delivery_ids": expected_ids,
        "event_order": ["pull_request", "issue_comment"],
        "upstream_read_attempts": 2,
        "shared_current_parent_confirmed": True,
        "requested_reviewers_teams": expected_teams,
        "comment_id": comment_id,
        "comment_body_matches_current_api_readback": True,
        "comment_body_trust": "untrusted_user_content",
    }


def validate_pr_comment_batch_api_reads(
    server_log_delta: str,
    *,
    owner: str,
    repository: str,
    pull_request_number: int,
    comment_id: int,
) -> dict[str, Any]:
    if (
        not isinstance(server_log_delta, str)
        or len(server_log_delta.encode("utf-8")) > 1024 * 1024
        or not isinstance(owner, str)
        or not owner
        or not isinstance(repository, str)
        or not repository
        or type(pull_request_number) is not int
        or pull_request_number < 1
        or type(comment_id) is not int
        or comment_id < 1
    ):
        raise ValueError("Forgejo PR/comment API read log binding is malformed")
    base = f"/api/v1/repos/{owner}/{repository}"
    expected = {
        f"GET {base}/pulls/{pull_request_number}": "pull_request_get_confirmed",
        f"GET {base}/issues/comments/{comment_id}": "comment_get_confirmed",
    }
    api_reads = [
        line for line in server_log_delta.splitlines()
        if " completed GET /api/v1/repos/" in line
    ]
    if len(api_reads) != 2:
        raise RuntimeError("Forgejo PR/comment batch did not perform exactly two repository GET requests")
    summary: dict[str, Any] = {"upstream_get_count": len(api_reads)}
    for prefix, confirmation in expected.items():
        matching = [line for line in api_reads if f" completed {prefix} " in line]
        if len(matching) != 1 or "200 OK" not in matching[0]:
            raise RuntimeError("Forgejo PR/comment batch did not read both current resources exactly once")
        summary[confirmation] = True
    return summary


def validate_pr_comment_batch_receiver_restart(
    *,
    journal_before: bytes,
    journal_after: bytes,
    records_before: Any,
    records_after: Any,
    hook_tasks: Any,
    hook_id: int,
    delivery_ids: list[str],
    killed_returncode: int,
    listeners_after_kill: list[str],
    restarted_listener_owned: bool,
) -> dict[str, Any]:
    """Require acknowledged native deliveries and byte-identical journal after SIGKILL/reopen."""
    expected_events = ["pull_request", "issue_comment"]
    expected_task_events = ["pull_request_review_request", "pull_request_comment"]
    if (
        not isinstance(journal_before, bytes)
        or not journal_before
        or not isinstance(journal_after, bytes)
        or journal_after != journal_before
        or not isinstance(records_before, list)
        or not isinstance(records_after, list)
        or len(records_before) != 2
        or records_after != records_before
        or not isinstance(delivery_ids, list)
        or len(delivery_ids) != 2
        or any(not isinstance(item, str) or not item for item in delivery_ids)
        or len(set(delivery_ids)) != 2
        or type(hook_id) is not int
        or hook_id < 1
        or type(killed_returncode) is not int
        or killed_returncode != -9
        or not isinstance(listeners_after_kill, list)
        or listeners_after_kill
        or restarted_listener_owned is not True
    ):
        raise RuntimeError("receiver restart did not preserve the stopped process and exact journal boundary")

    projected_events = []
    projected_ids = []
    for sequence, record in enumerate(records_before, start=1):
        if not isinstance(record, dict) or record.get("sequence") != sequence:
            raise RuntimeError("receiver restart journal sequence is missing or reordered")
        envelope = record.get("payload")
        if not isinstance(envelope, dict):
            raise RuntimeError("receiver restart journal envelope is invalid")
        event = envelope.get("event")
        delivery_id = envelope.get("delivery_id")
        if (
            event != expected_events[sequence - 1]
            or delivery_id != delivery_ids[sequence - 1]
            or record.get("kind") != f"forgejo.webhook.v1.{event}"
            or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
        ):
            raise RuntimeError("receiver restart journal lost the native PR/comment event order or identity")
        projected_events.append(event)
        projected_ids.append(delivery_id)

    if not isinstance(hook_tasks, list) or len(hook_tasks) != 2:
        raise RuntimeError("Forgejo did not retain exactly two native hook task readbacks")
    for event, task_event, delivery_id, task in zip(
        expected_events, expected_task_events, delivery_ids, hook_tasks, strict=True
    ):
        if (
            not isinstance(task, dict)
            or task.get("hook_id") != hook_id
            or task.get("uuid") != delivery_id
            or task.get("event_type") != task_event
            or task.get("is_delivered") is not True
            or task.get("is_succeed") is not True
            or task.get("response_status") != 202
        ):
            raise RuntimeError("receiver restart started before both Forgejo webhook tasks had HTTP 202 acknowledgements")

    return {
        "receiver_kill_signal": "SIGKILL",
        "receiver_kill_exit_code": killed_returncode,
        "old_listener_absent_before_restart": True,
        "restarted_listener_owned": True,
        "hook_task_count": len(hook_tasks),
        "both_hook_tasks_acked_202_before_kill": True,
        "event_types": projected_events,
        "forgejo_hook_task_event_types": expected_task_events,
        "delivery_ids": projected_ids,
        "journal_sha256_before": hashlib.sha256(journal_before).hexdigest(),
        "journal_sha256_after": hashlib.sha256(journal_after).hexdigest(),
        "journal_byte_identical_after_restart": True,
        "record_count_after_restart": len(records_after),
    }


def _comment_fingerprint(comments: Any) -> list[tuple[int, str]]:
    if not isinstance(comments, list) or len(comments) > 1000:
        raise ValueError("Forgejo comment readback is malformed or too large")
    result = []
    seen_ids: set[int] = set()
    for comment in comments:
        comment_id = comment.get("id") if isinstance(comment, dict) else None
        body = comment.get("body") if isinstance(comment, dict) else None
        if (
            type(comment_id) is not int
            or comment_id < 1
            or comment_id in seen_ids
            or not isinstance(body, str)
        ):
            raise ValueError("Forgejo comment readback contains an invalid identity")
        seen_ids.add(comment_id)
        result.append((comment_id, body))
    return result


def _pull_request_write_fingerprint(pull_request: Any) -> dict[str, Any]:
    if not isinstance(pull_request, dict):
        raise ValueError("Forgejo pull-request write readback is malformed")
    base = pull_request.get("base")
    head = pull_request.get("head")
    if not isinstance(base, dict) or not isinstance(head, dict):
        raise ValueError("Forgejo pull-request write readback omitted its branch bindings")
    base_repository = base.get("repo")
    if not isinstance(base_repository, dict):
        raise ValueError("Forgejo pull-request write readback omitted its base repository")
    reviewers = pull_request.get("requested_reviewers")
    if not isinstance(reviewers, list) or len(reviewers) > 1024:
        raise ValueError("Forgejo pull-request user-reviewer readback is malformed")
    reviewer_projection = []
    for reviewer in reviewers:
        reviewer_id = reviewer.get("id") if isinstance(reviewer, dict) else None
        login = reviewer.get("login") if isinstance(reviewer, dict) else None
        if type(reviewer_id) is not int or reviewer_id < 1 or not isinstance(login, str) or not login:
            raise ValueError("Forgejo pull-request user-reviewer identity is malformed")
        reviewer_projection.append({"id": reviewer_id, "login": login})
    reviewer_projection.sort(key=lambda item: (item["id"], item["login"]))
    number = pull_request.get("number")
    title = pull_request.get("title")
    body = pull_request.get("body")
    state = pull_request.get("state")
    merged = pull_request.get("merged")
    if (
        type(number) is not int
        or number < 1
        or not isinstance(title, str)
        or not isinstance(body, str)
        or not isinstance(state, str)
        or state not in {"open", "closed"}
        or not isinstance(merged, bool)
        or not isinstance(pull_request.get("updated_at"), str)
    ):
        raise ValueError("Forgejo pull-request write readback has invalid state fields")
    return {
        "number": number,
        "title": title,
        "body": body,
        "state": state,
        "merged": merged,
        "base_ref": base.get("ref"),
        "base_sha": base.get("sha"),
        "base_repository_full_name": base_repository.get("full_name"),
        "head_ref": head.get("ref"),
        "head_sha": head.get("sha"),
        "requested_reviewers": reviewer_projection,
        "requested_reviewers_teams": emitted.normalize_requested_reviewer_teams_readback(
            pull_request
        ),
        "updated_at": pull_request["updated_at"],
    }


def validate_read_only_write_scope_probe(
    *,
    http_status: int | None,
    marker: str,
    comments_before: Any,
    comments_after: Any,
    pull_request_before: Any,
    pull_request_after: Any,
) -> dict[str, Any]:
    if http_status not in {401, 403}:
        raise RuntimeError("Forgejo did not explicitly deny the read-only credential's write")
    before_comments = _comment_fingerprint(comments_before)
    after_comments = _comment_fingerprint(comments_after)
    if before_comments != after_comments:
        raise RuntimeError("comment state changed during the read-only credential probe")
    if any(marker in comment_body for _, comment_body in after_comments):
        raise RuntimeError("read-only credential probe marker appeared in current comments")
    if _pull_request_write_fingerprint(pull_request_before) != _pull_request_write_fingerprint(
        pull_request_after
    ):
        raise RuntimeError("pull-request state changed during the read-only credential probe")
    return {
        "http_status": http_status,
        "write_effect": "denied",
        "comments_unchanged": True,
        "pull_request_binding_unchanged": True,
        "probe_marker_absent": True,
    }


def build_receive_pack_request_body(
    *, old_oid: str, new_oid: str, ref: str, pack: bytes
) -> bytes:
    if (
        not isinstance(old_oid, str)
        or len(old_oid) not in {40, 64}
        or any(character not in "0123456789abcdef" for character in old_oid)
        or not isinstance(new_oid, str)
        or len(new_oid) != len(old_oid)
        or any(character not in "0123456789abcdef" for character in new_oid)
        or not isinstance(ref, str)
        or not ref.startswith("refs/heads/")
        or any(ord(character) < 0x21 or ord(character) > 0x7E for character in ref)
        or not isinstance(pack, bytes)
        or not pack.startswith(b"PACK")
    ):
        raise ValueError("Git receive-pack request fields are malformed")
    command = f"{old_oid} {new_oid} {ref}".encode("ascii") + b"\x00report-status\n"
    packet_length = len(command) + 4
    if packet_length > 0xFFFF:
        raise ValueError("Git receive-pack command packet exceeds the protocol limit")
    command_packet = f"{packet_length:04x}".encode("ascii") + command
    return command_packet + b"0000" + pack


def validate_read_only_git_receive_pack_probe(
    *,
    read_exit_code: int,
    read_stdout: str,
    read_stderr: str,
    push_exit_code: int,
    push_stdout: str,
    push_stderr: str,
    empty_flush_post_status: int | None,
    valid_push_post_status: int | None,
    target_branch: str,
    branches_before: Any,
    branches_after: Any,
    server_log: str,
    token: str,
) -> dict[str, Any]:
    if type(read_exit_code) is not int or read_exit_code != 0 or "refs/heads/main" not in read_stdout:
        raise RuntimeError("read-only token could not read the repository through Git HTTP")
    if not isinstance(read_stderr, str) or not isinstance(push_stdout, str) or not isinstance(push_stderr, str):
        raise ValueError("Git probe output must be text")
    if type(push_exit_code) is not int or push_exit_code == 0:
        raise RuntimeError("Forgejo accepted a write through Git receive-pack")
    if empty_flush_post_status != 403:
        raise RuntimeError("Forgejo did not explicitly deny empty Git receive-pack POST")
    if valid_push_post_status != 403:
        raise RuntimeError("Forgejo did not explicitly deny a valid Git receive-pack POST")
    if not isinstance(target_branch, str) or not target_branch:
        raise ValueError("Git write probe target branch is malformed")
    if not isinstance(token, str) or not token:
        raise ValueError("Git write probe token is malformed")
    encoded_credential = base64.b64encode(f"{REVIEWER}:{token}".encode("utf-8")).decode("ascii")
    git_output = "\n".join((read_stdout, read_stderr, push_stdout, push_stderr))
    if token in git_output or encoded_credential in git_output:
        raise RuntimeError("read-only Git credential appeared in command output")

    def branch_names(value: Any) -> set[str]:
        if not isinstance(value, list) or len(value) > 1000:
            raise ValueError("Forgejo branch readback is malformed or too large")
        names: set[str] = set()
        for branch in value:
            name = branch.get("name") if isinstance(branch, dict) else None
            if not isinstance(name, str) or not name or name in names:
                raise ValueError("Forgejo branch readback contains an invalid or duplicate name")
            names.add(name)
        return names

    before = branch_names(branches_before)
    after = branch_names(branches_after)
    if target_branch in before:
        raise RuntimeError("read-only Git write probe target already existed before the probe")
    if target_branch in after:
        raise RuntimeError("read-only Git write probe created a target branch")

    route = f"/{ORGANIZATION}/{REPOSITORY}.git/"
    advertisement_lines = [
        line
        for line in server_log.splitlines()
        if f"GET {route}info/refs?service=git-receive-pack" in line
    ]
    if not any("403 Forbidden" in line for line in advertisement_lines):
        raise RuntimeError("Forgejo did not log HTTP 403 for the receive-pack advertisement")
    post_route = f"POST {route}git-receive-pack"
    direct_post_lines = [
        line for line in server_log.splitlines() if post_route in line
    ]
    if len(direct_post_lines) != 2 or any(
        "403 Forbidden" not in line for line in direct_post_lines
    ):
        raise RuntimeError(
            "Forgejo did not log exactly two HTTP 403 responses for direct Git receive-pack POSTs"
        )
    return {
        "authenticated_git_read_confirmed": True,
        "receive_pack_advertisement_http_status": 403,
        "empty_flush_post_http_status": 403,
        "valid_push_post_http_status": 403,
        "target_ref_absent_before": True,
        "target_ref_absent_after": True,
        "write_effect": "denied",
        "credential_absent_from_git_output": True,
    }


def validate_write_credential_receive_pack_control(
    *,
    post_status: int | None,
    response_body: bytes,
    control_branch: str,
    expected_oid: str,
    branches_before: Any,
    branches_after: Any,
    server_log: str,
) -> dict[str, Any]:
    if type(post_status) is not int or post_status != 200:
        raise RuntimeError("write-enabled control credential did not complete receive-pack")
    if (
        not isinstance(response_body, bytes)
        or len(response_body) > 65_536
        or b"unpack ok\n" not in response_body
        or not isinstance(control_branch, str)
        or not control_branch.startswith("agent/")
        or not isinstance(expected_oid, str)
        or len(expected_oid) not in {40, 64}
        or any(character not in "0123456789abcdef" for character in expected_oid)
        or f"ok refs/heads/{control_branch}\n".encode("ascii") not in response_body
    ):
        raise RuntimeError("write-enabled receive-pack report-status is missing or malformed")

    def branch_projection(value: Any) -> dict[str, str | None]:
        if not isinstance(value, list) or len(value) > 1000:
            raise ValueError("Forgejo branch readback is malformed or too large")
        result: dict[str, str | None] = {}
        for item in value:
            name = item.get("name") if isinstance(item, dict) else None
            commit = item.get("commit") if isinstance(item, dict) else None
            oid = commit.get("id") if isinstance(commit, dict) else None
            if (
                not isinstance(name, str)
                or not name
                or name in result
                or (oid is not None and (not isinstance(oid, str) or len(oid) not in {40, 64}))
            ):
                raise ValueError("Forgejo branch readback contains invalid or duplicate entries")
            result[name] = oid
        return result

    before = branch_projection(branches_before)
    after = branch_projection(branches_after)
    if control_branch in before or after.get(control_branch) != expected_oid:
        raise RuntimeError("write-enabled receive-pack branch effect did not match the proposed commit")
    route = f"POST /{ORGANIZATION}/{REPOSITORY}.git/git-receive-pack"
    matching = [line for line in server_log.splitlines() if route in line]
    if len(matching) != 1 or "200 OK" not in matching[0]:
        raise RuntimeError("Forgejo access log did not confirm the single successful control POST")
    return {
        "write_effect_confirmed": True,
        "http_status": 200,
        "receive_pack_report_status_ok": True,
        "target_branch_readback_matches": True,
        "target_branch": control_branch,
        "target_oid": expected_oid,
    }


def run_read_only_git_receive_pack_probe(
    *,
    state: Path,
    evidence: Path,
    repo_path: str,
    api: fixture.LocalForgejoApi,
    reviewer_token: str,
    writer_token: str,
    server_log_path: Path,
) -> dict[str, Any]:
    source = state / "read-only-git-write-source"
    source.mkdir(mode=0o700)
    env = fixture.private_command_environment(state)
    basic = base64.b64encode(f"{REVIEWER}:{reviewer_token}".encode("utf-8")).decode("ascii")
    env.update(
        {
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": "http.extraHeader",
            "GIT_CONFIG_VALUE_0": f"Authorization: Basic {basic}",
            "GIT_TERMINAL_PROMPT": "0",
        }
    )

    def git(arguments: list[str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *arguments],
            cwd=source,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=20,
            check=False,
        )

    def require_git_setup(arguments: list[str]) -> None:
        result = git(arguments)
        if result.returncode != 0:
            raise RuntimeError(f"local Git write probe setup failed at git {arguments[0]}")

    require_git_setup(["init", "--quiet", "--initial-branch=main"])
    require_git_setup(["config", "user.name", "Local Forgejo Read-only Probe"])
    require_git_setup(["config", "user.email", "read-only-probe@local.invalid"])
    (source / "read-only-git-write-probe.txt").write_text(
        "Synthetic content for a denied Git receive-pack probe.\n", encoding="utf-8"
    )
    require_git_setup(["add", "read-only-git-write-probe.txt"])
    require_git_setup(["commit", "--quiet", "-m", "Create local read-only permission probe"])
    head_sha = git(["rev-parse", "HEAD"]).stdout.strip()
    if len(head_sha) != 40 or any(ch not in "0123456789abcdef" for ch in head_sha):
        raise RuntimeError("local Git write probe did not produce a SHA-1 commit id")
    packed = subprocess.run(
        ["git", "pack-objects", "--stdout", "--revs"],
        cwd=source,
        env=env,
        input=f"{head_sha}\n".encode("ascii"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=20,
        check=False,
    )
    if packed.returncode != 0 or not packed.stdout.startswith(b"PACK"):
        raise RuntimeError("local Git write probe could not construct a packfile")
    valid_push_body = build_receive_pack_request_body(
        old_oid="0" * 40,
        new_oid=head_sha,
        ref=f"refs/heads/{GIT_WRITE_PROBE_BRANCH}",
        pack=packed.stdout,
    )

    remote = f"http://127.0.0.1:3130/{ORGANIZATION}/{REPOSITORY}.git"
    read = git(["ls-remote", remote, "refs/heads/main"])
    branches_before, _ = api.request("GET", f"{repo_path}/branches")
    if api.last_status != 200:
        raise RuntimeError("admin branch readback failed before the Git permission probe")
    push = git(
        [
            "push",
            "--porcelain",
            remote,
            f"HEAD:refs/heads/{GIT_WRITE_PROBE_BRANCH}",
        ]
    )

    def direct_receive_pack_post(
        body: bytes, *, username: str, credential_token: str
    ) -> tuple[int, bytes]:
        connection = http.client.HTTPConnection("127.0.0.1", 3130, timeout=10)
        try:
            connection.request(
                "POST",
                f"/{ORGANIZATION}/{REPOSITORY}.git/git-receive-pack",
                body=body,
                headers={
                    "Authorization": "Basic "
                    + base64.b64encode(f"{username}:{credential_token}".encode("utf-8")).decode("ascii"),
                    "Content-Type": "application/x-git-receive-pack-request",
                    "Accept": "application/x-git-receive-pack-result",
                },
            )
            response = connection.getresponse()
            status = response.status
            response_body = response.read(65_537)
            response.close()
            if len(response_body) > 65_536:
                raise RuntimeError("Forgejo receive-pack response exceeded the local probe limit")
            return status, response_body
        except (OSError, http.client.HTTPException) as error:
            raise RuntimeError("direct Git receive-pack POST did not reach local Forgejo") from error
        finally:
            connection.close()

    empty_flush_post_status, _ = direct_receive_pack_post(
        b"0000", username=REVIEWER, credential_token=reviewer_token
    )
    valid_push_post_status, _ = direct_receive_pack_post(
        valid_push_body, username=REVIEWER, credential_token=reviewer_token
    )
    branches_after, _ = api.request("GET", f"{repo_path}/branches")
    if api.last_status != 200:
        raise RuntimeError("admin branch readback failed after the Git permission probe")
    server_log = server_log_path.read_text(encoding="utf-8", errors="replace")
    route = f"/{ORGANIZATION}/{REPOSITORY}.git/"
    receive_pack_lines = [
        line
        for line in server_log.splitlines()
        if route in line and "git-receive-pack" in line
    ]
    git_output = "\n".join((read.stdout, read.stderr, push.stdout, push.stderr))
    summary = validate_read_only_git_receive_pack_probe(
        read_exit_code=read.returncode,
        read_stdout=read.stdout,
        read_stderr=read.stderr,
        push_exit_code=push.returncode,
        push_stdout=push.stdout,
        push_stderr=push.stderr,
        empty_flush_post_status=empty_flush_post_status,
        valid_push_post_status=valid_push_post_status,
        target_branch=GIT_WRITE_PROBE_BRANCH,
        branches_before=branches_before,
        branches_after=branches_after,
        server_log=server_log,
        token=reviewer_token,
    )

    control_body = build_receive_pack_request_body(
        old_oid="0" * 40,
        new_oid=head_sha,
        ref=f"refs/heads/{GIT_WRITE_CONTROL_BRANCH}",
        pack=packed.stdout,
    )
    control_branches_before = branches_after
    control_log_prefix = server_log
    control_post_status, control_response_body = direct_receive_pack_post(
        control_body, username=ADMIN, credential_token=writer_token
    )
    control_branches_after, _ = api.request("GET", f"{repo_path}/branches")
    if api.last_status != 200:
        raise RuntimeError("admin branch readback failed after the write-enabled control")
    complete_server_log = server_log_path.read_text(encoding="utf-8", errors="replace")
    if not complete_server_log.startswith(control_log_prefix):
        raise RuntimeError("Forgejo access log changed unexpectedly during the write control")
    control_log_delta = complete_server_log[len(control_log_prefix) :]
    control_summary = validate_write_credential_receive_pack_control(
        post_status=control_post_status,
        response_body=control_response_body,
        control_branch=GIT_WRITE_CONTROL_BRANCH,
        expected_oid=head_sha,
        branches_before=control_branches_before,
        branches_after=control_branches_after,
        server_log=control_log_delta,
    )
    write_private_json(
        evidence / "git-receive-pack-attempt.json",
        {
            "authenticated_read_exit_code": read.returncode,
            "authenticated_main_ref_observed": "refs/heads/main" in read.stdout,
            "push_exit_code": push.returncode,
            "push_output_contains_credential": (
                reviewer_token in git_output or basic in git_output
            ),
            "empty_flush_post_status": empty_flush_post_status,
            "valid_push_post_status": valid_push_post_status,
            "valid_push_packet_bytes": len(valid_push_body),
            "valid_push_pack_bytes": len(packed.stdout),
            "valid_push_pack_sha256": hashlib.sha256(packed.stdout).hexdigest(),
            "writer_control_post_status": control_post_status,
            "writer_control_response_contains_report_status": (
                b"unpack ok\n" in control_response_body
            ),
            "writer_control_new_oid": head_sha,
            "writer_control_target_branch_present_after": GIT_WRITE_CONTROL_BRANCH
            in {item.get("name") for item in control_branches_after if isinstance(item, dict)},
            "target_ref_absent_before": GIT_WRITE_PROBE_BRANCH
            not in {item.get("name") for item in branches_before if isinstance(item, dict)},
            "target_ref_absent_after": GIT_WRITE_PROBE_BRANCH
            not in {item.get("name") for item in branches_after if isinstance(item, dict)},
            "receive_pack_route_log_lines": len(receive_pack_lines),
            "receive_pack_advertisement_http_403_observed": any(
                f"GET {route}info/refs?service=git-receive-pack" in line
                and "403 Forbidden" in line
                for line in receive_pack_lines
            ),
            "receive_pack_post_http_403_observed": sum(
                f"POST {route}git-receive-pack" in line and "403 Forbidden" in line
                for line in receive_pack_lines
            ) == 2,
        },
        [reviewer_token, writer_token],
    )
    write_private_json(
        evidence / "git-receive-pack-write-probe.json",
        {
            "repository": f"{ORGANIZATION}/{REPOSITORY}",
            "target_branch": GIT_WRITE_PROBE_BRANCH,
            "git_read_exit_code": read.returncode,
            "git_read_main_ref_observed": "refs/heads/main" in read.stdout,
            "git_push_exit_code": push.returncode,
            "direct_empty_flush_post_http_status": empty_flush_post_status,
            "direct_valid_push_post_http_status": valid_push_post_status,
            "valid_push_packet_bytes": len(valid_push_body),
            "valid_push_pack_bytes": len(packed.stdout),
            "valid_push_pack_sha256": hashlib.sha256(packed.stdout).hexdigest(),
            **summary,
        },
        [reviewer_token, writer_token],
    )
    write_private_json(
        evidence / "git-receive-pack-write-control.json",
        control_summary,
        [reviewer_token, writer_token],
    )
    return {"read_only": summary, "write_enabled_control": control_summary}


def write_private_json(
    path: Path, value: Any, private_tokens: list[str] | None = None
) -> None:
    fixture.write_safe_json(path, value, private_tokens or [])


def enable_console_info_access_logs(state: Path) -> None:
    config_path = state / "custom/conf/app.ini"
    config = config_path.read_bytes()
    default = b"[log]\nMODE = file\nLEVEL = Warn\n"
    instrumented = b"[log]\nMODE = console\nLEVEL = Info\n"
    if config.count(default) != 1:
        raise RuntimeError("local Forgejo config did not contain the expected default log section")
    config_path.write_bytes(config.replace(default, instrumented, 1))
    config_path.chmod(0o600)


def validate_review_requested_delivery_record(
    record: Any, *, owner: str, repository: str, pull_request_number: int
) -> dict[str, Any]:
    if not isinstance(record, dict):
        raise ValueError("receiver journal record must be an object")
    sequence = record.get("sequence")
    envelope = record.get("payload")
    if (
        type(sequence) is not int
        or sequence < 1
        or record.get("kind") != "forgejo.webhook.v1.pull_request"
        or not isinstance(envelope, dict)
        or envelope.get("schema_version") != 1
        or envelope.get("provider") != "forgejo"
        or envelope.get("event") != "pull_request"
    ):
        raise ValueError("receiver journal record is not a valid Forgejo pull-request delivery")
    delivery_id = envelope.get("delivery_id")
    payload = envelope.get("payload")
    if (
        not isinstance(delivery_id, str)
        or not emitted.DELIVERY_ID_RE.fullmatch(delivery_id)
        or record.get("idempotency_key") != f"forgejo-delivery-v1:{delivery_id}"
        or not isinstance(payload, dict)
    ):
        raise ValueError("receiver journal delivery envelope is invalid")
    repository_ref = payload.get("repository")
    pull_request = payload.get("pull_request")
    if (
        payload.get("action") != "review_requested"
        or not isinstance(repository_ref, dict)
        or repository_ref.get("full_name") != f"{owner}/{repository}"
        or not isinstance(pull_request, dict)
        or type(pull_request.get("number")) is not int
        or pull_request.get("number") != pull_request_number
        or pull_request.get("state") != "open"
        or pull_request.get("merged") is not False
    ):
        raise ValueError("receiver journal event does not match the expected open PR review request")
    head = pull_request.get("head")
    base = pull_request.get("base")
    head_sha = head.get("sha") if isinstance(head, dict) else None
    base_sha = base.get("sha") if isinstance(base, dict) else None
    if (
        not isinstance(head_sha, str)
        or not emitted.COMMIT_SHA_RE.fullmatch(head_sha)
        or not isinstance(base_sha, str)
        or not emitted.COMMIT_SHA_RE.fullmatch(base_sha)
    ):
        raise ValueError("receiver journal pull-request event has invalid commit bindings")
    requested_team = payload.get("requested_team")
    team_summary = None
    if requested_team is not None:
        if not isinstance(requested_team, dict):
            raise ValueError("receiver journal requested-team payload is malformed")
        team_id = requested_team.get("id")
        team_name = requested_team.get("name")
        if type(team_id) is not int or team_id < 1 or not isinstance(team_name, str) or not team_name:
            raise ValueError("receiver journal requested-team identity is malformed")
        team_summary = {"id": team_id, "name": team_name}
    return {
        "sequence": sequence,
        "delivery_id": delivery_id,
        "event": "pull_request",
        "action": "review_requested",
        "pull_request_number": pull_request_number,
        "head_sha": head_sha,
        "base_sha": base_sha,
        "requested_team": team_summary,
    }


def start_reconciliation_cli(
    binary: Path,
    state: Path,
    journal: Path,
    delivery_ids: list[str],
    credential: str,
) -> subprocess.Popen[bytes]:
    if not delivery_ids or len(delivery_ids) > 100 or len(set(delivery_ids)) != len(delivery_ids):
        raise ValueError("interrupted reconciliation requires unique delivery IDs")
    read_fd, write_fd = os.pipe()
    try:
        credential_bytes = credential.encode("utf-8")
        view = memoryview(credential_bytes)
        while view:
            count = os.write(write_fd, view)
            if count <= 0:
                raise OSError("short write to private reconciliation credential pipe")
            view = view[count:]
    finally:
        os.close(write_fd)
    command = [
        str(binary),
        "--reconcile-target-profile",
        "org-team-review-e2e",
        "--host-credential-fd",
        str(read_fd),
        str(journal),
        *delivery_ids,
    ]
    try:
        process = subprocess.Popen(
            command,
            cwd=state,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=fixture.private_command_environment(state),
            pass_fds=(read_fd,),
            start_new_session=True,
        )
    finally:
        os.close(read_fd)
    return process


def kill_reconciliation_process_group(
    process: subprocess.Popen[bytes], credential: str
) -> dict[str, Any]:
    if process.poll() is not None:
        raise RuntimeError("reconciliation CLI exited before the API barrier was reached")
    process_group = os.getpgid(process.pid)
    if process_group != process.pid:
        raise RuntimeError("reconciliation CLI did not own its isolated process group")
    os.killpg(process_group, signal.SIGKILL)
    stdout, stderr = process.communicate(timeout=10)
    group_gone = False
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline:
        try:
            os.killpg(process_group, 0)
        except ProcessLookupError:
            group_gone = True
            break
        except PermissionError:
            break
        time.sleep(0.01)
    credential_bytes = credential.encode("utf-8")
    return {
        "pid": process.pid,
        "process_group": process_group,
        "returncode": process.returncode,
        "signal": -process.returncode if process.returncode is not None and process.returncode < 0 else None,
        "group_gone": group_gone,
        "stdout_bytes": len(stdout),
        "stderr_bytes": len(stderr),
        "stdout_sha256": hashlib.sha256(stdout).hexdigest(),
        "stderr_sha256": hashlib.sha256(stderr).hexdigest(),
        "credential_exposed": credential_bytes in stdout or credential_bytes in stderr,
    }


def validate_interrupted_cli_attempt(
    *,
    proxy_records: Any,
    parent_get_path: str,
    comment_get_path: str,
    server_log_delta: str,
    journal_before: bytes,
    journal_after: bytes,
    barrier: Any,
    killed_process: Any,
    forbidden_route_record: Any,
    forbidden_status: int,
    forbidden_post_upstream_hits: Any,
) -> dict[str, Any]:
    if (
        not isinstance(parent_get_path, str)
        or not parent_get_path.startswith("/api/v1/repos/")
        or not isinstance(comment_get_path, str)
        or not comment_get_path.startswith("/api/v1/repos/")
        or parent_get_path == comment_get_path
        or not isinstance(proxy_records, list)
        or len(proxy_records) != 2
        or not isinstance(server_log_delta, str)
        or len(server_log_delta.encode("utf-8")) > 1024 * 1024
        or not isinstance(journal_before, bytes)
        or not journal_before
        or journal_after != journal_before
        or not isinstance(barrier, dict)
        or barrier.get("path") != comment_get_path
        or barrier.get("claimed") is not True
        or barrier.get("reached") is not True
        or barrier.get("released") is not True
        or barrier.get("forward") is not False
        or barrier.get("finished") is not True
        or not isinstance(killed_process, dict)
        or killed_process.get("returncode") != -signal.SIGKILL
        or killed_process.get("signal") != signal.SIGKILL
        or killed_process.get("group_gone") is not True
        or killed_process.get("credential_exposed") is not False
        or forbidden_status != 403
        or forbidden_post_upstream_hits != []
        or not isinstance(forbidden_route_record, dict)
        or forbidden_route_record.get("method") != "POST"
        or forbidden_route_record.get("status") != 403
        or forbidden_route_record.get("forwarded") is not False
        or forbidden_route_record.get("gate") != "allowlisted"
        or forbidden_route_record.get("barrier") != "not_allowlisted"
        or forbidden_route_record.get("reader_credential_match") is not True
    ):
        raise RuntimeError("interrupted API read did not stop at the bound barrier or preserve its evidence")

    first, held = proxy_records
    if (
        not isinstance(first, dict)
        or first.get("method") != "GET"
        or first.get("path") != parent_get_path
        or first.get("status") != 200
        or first.get("forwarded") is not True
        or first.get("reader_credential_match") is not True
        or first.get("gate") != "allowlisted"
        or first.get("barrier") != "none"
        or not isinstance(held, dict)
        or held.get("method") != "GET"
        or held.get("path") != comment_get_path
        or held.get("status") != 503
        or held.get("forwarded") is not False
        or held.get("reader_credential_match") is not True
        or held.get("gate") != "allowlisted"
        or held.get("barrier") != "cancelled_before_upstream"
    ):
        raise RuntimeError("first CLI attempt did not read the PR then cancel the comment GET before Forgejo")

    upstream_gets = [
        line for line in server_log_delta.splitlines()
        if " completed GET /api/v1/repos/" in line
    ]
    if (
        len(upstream_gets) != 1
        or f" completed GET {parent_get_path} " not in upstream_gets[0]
        or "200 OK" not in upstream_gets[0]
        or comment_get_path in server_log_delta
    ):
        raise RuntimeError("cancelled comment GET reached Forgejo or the parent GET was not observed once")

    return {
        "parent_get_completed_before_interruption": True,
        "comment_get_held_then_cancelled_before_upstream": True,
        "upstream_get_count_before_restart": 1,
        "gate_refused_unlisted_post": True,
        "forbidden_post_reached_forgejo": False,
        "cli_group_sigkill": True,
        "cli_group_gone": True,
        "journal_sha256_before": hashlib.sha256(journal_before).hexdigest(),
        "journal_sha256_after_kill": hashlib.sha256(journal_after).hexdigest(),
        "journal_unchanged_after_kill": True,
        "barrier": barrier,
        "proxy_records": proxy_records,
        "killed_process": killed_process,
        "forbidden_route_record": forbidden_route_record,
    }


def validate_interrupted_cli_resume(
    *,
    proxy_records: Any,
    parent_get_path: str,
    comment_get_path: str,
    reconciled: Any,
    journal_before: bytes,
    journal_after: bytes,
) -> dict[str, Any]:
    if (
        not isinstance(proxy_records, list)
        or len(proxy_records) != 2
        or not isinstance(reconciled, dict)
        or reconciled.get("schema_version") != emitted.RECONCILE_BATCH_SCHEMA_VERSION
        or reconciled.get("verdict") != "current_state_read"
        or reconciled.get("upstream_read_attempts") != 2
        or journal_after != journal_before
    ):
        raise RuntimeError("restarted reconciliation did not complete the original read batch unchanged")
    expected = [parent_get_path, comment_get_path]
    for request, path in zip(proxy_records, expected, strict=True):
        if (
            not isinstance(request, dict)
            or request.get("method") != "GET"
            or request.get("path") != path
            or request.get("status") != 200
            or request.get("forwarded") is not True
            or request.get("reader_credential_match") is not True
            or request.get("gate") != "allowlisted"
            or request.get("barrier") != "none"
        ):
            raise RuntimeError("resumed CLI requests escaped their exact API read allowlist")
    return {
        "resumed_cli_completed_same_two_delivery_batch": True,
        "resumed_api_get_order": expected,
        "inbox_unchanged": True,
        "verdict": "current_state_read",
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    if args.kill_receiver_before_reconcile and not args.probe_pr_comment_batch:
        raise RuntimeError("hard receiver restart requires the native PR/comment batch probe")
    if args.interrupt_cli_between_parent_and_comment and not args.probe_pr_comment_batch:
        raise RuntimeError("CLI interruption requires the native PR/comment batch probe")
    if args.interrupt_cli_between_parent_and_comment and args.kill_receiver_before_reconcile:
        raise RuntimeError("receiver restart and CLI interruption are separate failure boundaries")
    binary_arg = Path(args.forgejo_binary)
    if binary_arg.is_symlink():
        raise RuntimeError("Forgejo binary must not be a symlink")
    binary = binary_arg.resolve(strict=True)
    if not binary.is_file():
        raise RuntimeError("Forgejo binary is not a regular file")
    state = Path(args.state_dir)
    evidence = Path(args.evidence_dir).resolve()
    if not state.is_absolute() or state.parent != Path("/tmp") or state.exists():
        raise RuntimeError("state dir must be a new direct child of /tmp")
    if evidence.exists():
        raise RuntimeError("refusing to overwrite an existing evidence directory")
    version = subprocess.run(
        [str(binary), "--version"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env={"PATH": "/usr/bin:/bin", "HOME": "/tmp"},
        timeout=10,
        check=False,
    )
    if version.returncode or FORGEJO_VERSION not in version.stdout:
        raise RuntimeError("this E2E requires the locally verified Forgejo 17.0.0 binary")
    for port in (3130, 3131):
        with socket.socket() as probe:
            if probe.connect_ex(("127.0.0.1", port)) == 0:
                raise RuntimeError(f"127.0.0.1:{port} is already in use")

    state.mkdir(mode=0o700)
    evidence.mkdir(mode=0o700, parents=True)
    forgejo_helpers.create_app_ini(state, allow_loopback_webhooks=True)
    app_ini = state / "custom/conf/app.ini"
    app_ini_text = app_ini.read_text(encoding="utf-8")
    if app_ini_text.count("HTTP_PORT = 3130") != 1:
        raise RuntimeError("Forgejo fixture config did not contain one expected HTTP port")
    app_ini.write_text(app_ini_text.replace("HTTP_PORT = 3130", "HTTP_PORT = 3131"), encoding="utf-8")
    if args.probe_read_only_git_write or args.probe_pr_comment_batch:
        enable_console_info_access_logs(state)
    receiver_bin, reconcile_bin, build_provenance = fixture.build_local_binaries(
        forgejo_helpers.process_e2e.REPO.resolve(strict=True), state, evidence
    )
    write_private_json(evidence / "build-provenance.json", build_provenance)
    server_log_path = state / "log/forgejo-server.log"
    server_log = server_log_path.open("wb")
    server: subprocess.Popen[bytes] | None = None
    receiver: subprocess.Popen[bytes] | None = None
    receiver_log = None
    stopped: dict[str, Any] | None = None
    receiver_stopped: dict[str, Any] | None = None
    api_proxy: native_replay.ReconciliationApiObservationProxy | None = None
    api_proxy_stopped: dict[str, Any] | None = None
    receiver_port: int | None = None
    tokens: list[str] = []
    stage = "start_forgejo"
    try:
        server = subprocess.Popen(
            [
                str(binary),
                "--work-path",
                str(state),
                "--config",
                str(state / "custom/conf/app.ini"),
                "web",
            ],
            cwd=state,
            env=fixture.private_command_environment(state),
            stdout=server_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        api_proxy = native_replay.ReconciliationApiObservationProxy()
        api_proxy.start()
        forgejo_helpers.wait_for_health(server)
        forgejo_helpers.assert_forgejo_listener(server, port=3131)

        stage = "provision_local_accounts"
        fixture.admin_command(
            binary,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                ADMIN,
                "--random-password",
                "--email",
                f"{ADMIN}@local.invalid",
                "--admin",
                "--must-change-password=false",
            ],
        )
        fixture.admin_command(
            binary,
            state,
            [
                "admin",
                "user",
                "create",
                "--username",
                REVIEWER,
                "--random-password",
                "--email",
                f"{REVIEWER}@local.invalid",
                "--must-change-password=false",
            ],
        )
        token = fixture.create_access_token(
            binary,
            state,
            ADMIN,
            "local-org-team-review-readback",
            "write:user,write:organization,write:repository,write:issue",
        )
        tokens.append(token)
        api = fixture.LocalForgejoApi(token)
        account, _ = api.request("GET", "/user")
        if account.get("login") != ADMIN or account.get("is_admin") is not True:
            raise RuntimeError("Forgejo did not confirm the local test administrator")

        stage = "create_organization_and_team"
        org, _ = api.request(
            "POST",
            "/orgs",
            {"username": ORGANIZATION, "visibility": "private"},
        )
        org_id = org.get("id") if isinstance(org, dict) else None
        if (
            api.last_status != 201
            or type(org_id) is not int
            or org_id < 1
            or org.get("username", org.get("name")) != ORGANIZATION
            or org.get("visibility") != "private"
        ):
            raise RuntimeError("Forgejo did not create the expected private organization")

        team, _ = api.request(
            "POST",
            f"/orgs/{ORGANIZATION}/teams",
            {
                "name": TEAM,
                "description": "Local integration-test reviewer team",
                "permission": "write",
                "includes_all_repositories": True,
                "units_map": {
                    "repo.code": "read",
                    "repo.issues": "read",
                    "repo.pulls": "write",
                },
            },
        )
        team_id = team.get("id") if isinstance(team, dict) else None
        team_units = team.get("units_map") if isinstance(team, dict) else None
        if (
            api.last_status != 201
            or type(team_id) is not int
            or team_id < 1
            or team.get("name") != TEAM
            or team.get("includes_all_repositories") is not True
            or not isinstance(team_units, dict)
            or team_units.get("repo.pulls") != "write"
        ):
            raise RuntimeError("Forgejo did not create the expected review team")
        api.request("PUT", f"/teams/{team_id}/members/{REVIEWER}")
        if api.last_status != 204:
            raise RuntimeError("Forgejo did not confirm reviewer-team membership")

        stage = "create_org_repository_and_pull_request"
        repository, _ = api.request(
            "POST",
            f"/orgs/{ORGANIZATION}/repos",
            {
                "name": REPOSITORY,
                "description": "Disposable local team-review readback fixture",
                "private": True,
                "auto_init": True,
                "default_branch": "main",
            },
        )
        if (
            api.last_status != 201
            or not isinstance(repository, dict)
            or repository.get("full_name") != f"{ORGANIZATION}/{REPOSITORY}"
            or repository.get("private") is not True
        ):
            raise RuntimeError("Forgejo did not create the expected private organization repository")

        repo_path = f"/repos/{ORGANIZATION}/{REPOSITORY}"
        api.request(
            "POST",
            f"{repo_path}/branches",
            {"new_branch_name": BRANCH, "old_branch_name": "main"},
        )
        if api.last_status != 201:
            raise RuntimeError("Forgejo did not create the team-review candidate branch")
        candidate = (
            "# Live team reviewer readback\n\n"
            "A real local Forgejo PR used to verify a non-empty organization-team reviewer list.\n"
        ).encode("utf-8")
        api.request(
            "POST",
            f"{repo_path}/contents/team-review-readback.md",
            {
                "message": "Add team reviewer readback fixture",
                "content": base64.b64encode(candidate).decode("ascii"),
                "branch": BRANCH,
            },
        )
        if api.last_status != 201:
            raise RuntimeError("Forgejo did not create the candidate file")
        created_pr, _ = api.request(
            "POST",
            f"{repo_path}/pulls",
            {
                "title": "Verify organization team reviewer readback",
                "body": "Disposable local API integration fixture.",
                "head": BRANCH,
                "base": "main",
            },
        )
        pr_number = created_pr.get("number") if isinstance(created_pr, dict) else None
        if api.last_status != 201 or type(pr_number) is not int or pr_number < 1:
            raise RuntimeError("Forgejo did not create the expected local pull request")

        stage = "start_receiver_and_create_webhook"
        webhook_secret = secrets.token_hex(32)
        secret_path = state / "webhook.secret"
        forgejo_helpers.write_private(secret_path, webhook_secret.encode("ascii"))
        journal_path = state / "data/webhook-inbox.jsonl"
        receiver_port = fixture.select_receiver_port()
        receiver_address = f"127.0.0.1:{receiver_port}"
        expected_hook_url = f"http://{receiver_address}/forgejo/webhook"
        receiver_log = (state / "log/receiver.log").open("wb")
        receiver = subprocess.Popen(
            [str(receiver_bin), receiver_address, str(secret_path), str(journal_path)],
            cwd=state,
            env=fixture.private_command_environment(state),
            stdout=receiver_log,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )
        fixture.wait_for_listener(receiver, receiver_port)
        hook, _ = api.request(
            "POST",
            f"/repos/{ORGANIZATION}/{REPOSITORY}/hooks",
            {
                "type": "gitea",
                "config": {
                    "url": expected_hook_url,
                    "content_type": "json",
                    "secret": webhook_secret,
                },
                "events": (
                    ["pull_request", "issue_comment"]
                    if args.probe_pr_comment_batch
                    else ["pull_request"]
                ),
                "active": True,
            },
        )
        hook_id = hook.get("id") if isinstance(hook, dict) else None
        if (
            api.last_status != 201
            or type(hook_id) is not int
            or hook_id < 1
            or hook.get("active") is not True
        ):
            raise RuntimeError("Forgejo did not create the expected active local PR webhook")
        hook_readback, _ = api.request(
            "GET", f"/repos/{ORGANIZATION}/{REPOSITORY}/hooks/{hook_id}"
        )
        hook_config = hook_readback.get("config") if isinstance(hook_readback, dict) else None
        if (
            api.last_status != 200
            or not isinstance(hook_readback, dict)
            or hook_readback.get("active") is not True
            or not isinstance(hook_config, dict)
            or hook_config.get("url") != expected_hook_url
        ):
            raise RuntimeError("Forgejo hook readback did not confirm the loopback receiver target")

        stage = "request_and_read_back_team_reviewer"
        api.request(
            "POST",
            f"{repo_path}/pulls/{pr_number}/requested_reviewers",
            {"team_reviewers": [TEAM]},
        )
        request_status = api.last_status
        current_pr, _ = api.request("GET", f"{repo_path}/pulls/{pr_number}")
        base = current_pr.get("base") if isinstance(current_pr, dict) else None
        base_repo = base.get("repo") if isinstance(base, dict) else None
        if (
            not isinstance(current_pr, dict)
            or current_pr.get("number") != pr_number
            or current_pr.get("state") != "open"
            or current_pr.get("merged") is not False
            or not isinstance(base_repo, dict)
            or base_repo.get("full_name") != f"{ORGANIZATION}/{REPOSITORY}"
        ):
            raise RuntimeError("Forgejo PR readback did not bind the request to the intended open PR")
        projection = validate_team_readback(
            request_status=request_status,
            pull_request=current_pr,
            expected_team_id=team_id,
            expected_team_name=TEAM,
        )
        journal_records = emitted.wait_for_journal_count(journal_path, 1)
        if len(journal_records) != 1:
            raise RuntimeError("receiver journal contains an unexpected number of PR webhooks")
        event_summary = validate_review_requested_delivery_record(
            journal_records[0],
            owner=ORGANIZATION,
            repository=REPOSITORY,
            pull_request_number=pr_number,
        )
        if event_summary["requested_team"] not in (None, {"id": team_id, "name": TEAM}):
            raise RuntimeError("Forgejo webhook team identity differs from the created review team")

        comment_event_summary = None
        comment_id = None
        receiver_restart_summary = None
        interrupted_cli_summary = None
        if args.probe_pr_comment_batch:
            stage = "create_local_pull_request_comment_delivery"
            created_comment, _ = api.request(
                "POST",
                f"{repo_path}/issues/{pr_number}/comments",
                {"body": PR_COMMENT_BODY},
            )
            if api.last_status != 201:
                raise RuntimeError("Forgejo did not create the synthetic PR comment with HTTP 201")
            comment_id = created_comment.get("id") if isinstance(created_comment, dict) else None
            if type(comment_id) is not int or comment_id < 1:
                raise RuntimeError("Forgejo comment creation response omitted its numeric ID")
            journal_records = emitted.wait_for_journal_count(journal_path, 2)
            if len(journal_records) != 2:
                raise RuntimeError("receiver journal contains an unexpected number of PR/comment webhooks")
            comment_event_summary = emitted.parse_live_pull_request_delivery_record(
                journal_records[1],
                owner=ORGANIZATION,
                repository=REPOSITORY,
                pull_request_number=pr_number,
                expected_event="issue_comment",
                expected_action="created",
                expected_comment_id=comment_id,
            )
            if comment_event_summary.get("sequence") != 2 or comment_event_summary.get("is_pull") is not True:
                raise RuntimeError("Forgejo issue_comment delivery was not bound as the second PR conversation event")
            current_pr, _ = api.request("GET", f"{repo_path}/pulls/{pr_number}")
            base = current_pr.get("base") if isinstance(current_pr, dict) else None
            head = current_pr.get("head") if isinstance(current_pr, dict) else None
            current_pr_base_sha = base.get("sha") if isinstance(base, dict) else None
            current_pr_head_sha = head.get("sha") if isinstance(head, dict) else None
            if (
                not isinstance(current_pr, dict)
                or current_pr.get("number") != pr_number
                or current_pr.get("state") != "open"
                or current_pr.get("merged") is not False
                or not isinstance(current_pr_base_sha, str)
                or not isinstance(current_pr_head_sha, str)
            ):
                raise RuntimeError("Forgejo current PR readback lost its open parent binding after comment creation")
            if validate_team_readback(
                request_status=request_status,
                pull_request=current_pr,
                expected_team_id=team_id,
                expected_team_name=TEAM,
            ) != projection:
                raise RuntimeError("Forgejo current PR readback lost the requested reviewer team after comment creation")

            if args.kill_receiver_before_reconcile:
                stage = "acknowledge_native_webhooks_then_restart_receiver"
                if receiver is None or receiver_port is None:
                    raise RuntimeError("local webhook receiver was unavailable at the requested restart boundary")
                hook_tasks = native_replay.wait_for_task_count(
                    state / "data/forgejo.db",
                    hook_id,
                    expected_count=2,
                    final_status=202,
                )
                journal_before_restart = journal_path.read_bytes()
                records_before_restart = emitted.wait_for_journal_count(journal_path, 2)
                old_receiver = receiver
                if old_receiver.poll() is not None:
                    raise RuntimeError("webhook receiver exited before the controlled SIGKILL boundary")
                old_receiver.kill()
                killed_returncode = old_receiver.wait(timeout=5)
                listeners_after_kill = fixture.listener_addresses_for_port(receiver_port)
                if receiver_log is not None:
                    receiver_log.close()
                receiver_log = (state / "log/receiver-restarted.log").open("wb")
                receiver = subprocess.Popen(
                    [str(receiver_bin), receiver_address, str(secret_path), str(journal_path)],
                    cwd=state,
                    env=fixture.private_command_environment(state),
                    stdout=receiver_log,
                    stderr=subprocess.STDOUT,
                    close_fds=True,
                )
                fixture.wait_for_listener(receiver, receiver_port)
                journal_after_restart = journal_path.read_bytes()
                records_after_restart = emitted.wait_for_journal_count(journal_path, 2)
                receiver_restart_summary = validate_pr_comment_batch_receiver_restart(
                    journal_before=journal_before_restart,
                    journal_after=journal_after_restart,
                    records_before=records_before_restart,
                    records_after=records_after_restart,
                    hook_tasks=hook_tasks,
                    hook_id=hook_id,
                    delivery_ids=[
                        event_summary["delivery_id"],
                        comment_event_summary["delivery_id"],
                    ],
                    killed_returncode=killed_returncode,
                    listeners_after_kill=listeners_after_kill,
                    restarted_listener_owned=True,
                )
                write_private_json(
                    evidence / "receiver-restart.json", receiver_restart_summary, tokens
                )

        stage = "read_only_rust_reconciliation"
        reviewer_token = fixture.create_access_token(
            binary,
            state,
            REVIEWER,
            "local-org-team-review-readonly",
            "read:user,read:repository,read:issue",
        )
        tokens.append(reviewer_token)
        reviewer_api = fixture.LocalForgejoApi(reviewer_token)
        reviewer_account, _ = reviewer_api.request("GET", "/user")
        if reviewer_account.get("login") != REVIEWER or reviewer_account.get("is_admin") is not False:
            raise RuntimeError("Forgejo did not confirm the reconciliation credential is read-only")

        stage = "probe_read_only_credential_write_refusal"
        comments_path = f"{repo_path}/issues/{pr_number}/comments"
        comments_before, _ = api.request("GET", comments_path)
        probe_marker = f"read-only-scope-probe-{secrets.token_hex(8)}"
        reviewer_api.request(
            "POST",
            comments_path,
            {"body": probe_marker},
            expected_http_error_statuses=tuple(range(400, 600)),
        )
        probe_status = reviewer_api.last_status
        comments_after, _ = api.request("GET", comments_path)
        pull_request_after_probe, _ = api.request("GET", f"{repo_path}/pulls/{pr_number}")
        fingerprint_before = _pull_request_write_fingerprint(current_pr)
        fingerprint_after = _pull_request_write_fingerprint(pull_request_after_probe)
        before_comment_ids = [
            comment.get("id") for comment in comments_before if isinstance(comment, dict)
        ] if isinstance(comments_before, list) else None
        after_comment_ids = [
            comment.get("id") for comment in comments_after if isinstance(comment, dict)
        ] if isinstance(comments_after, list) else None
        marker_present = any(
            isinstance(comment, dict) and probe_marker in str(comment.get("body", ""))
            for comment in comments_after
        ) if isinstance(comments_after, list) else None
        write_probe_observation = {
            "http_method": "POST",
            "route": f"{repo_path}/issues/{pr_number}/comments",
            "credential_scopes": ["read:user", "read:repository", "read:issue"],
            "http_status": probe_status,
            "comment_ids_before": before_comment_ids,
            "comment_ids_after": after_comment_ids,
            "probe_marker_present_after": marker_present,
            "pull_request_binding_unchanged": fingerprint_before == fingerprint_after,
        }
        write_private_json(
            evidence / "write-scope-probe.json", write_probe_observation, tokens
        )
        write_probe_summary = validate_read_only_write_scope_probe(
            http_status=probe_status,
            marker=probe_marker,
            comments_before=comments_before,
            comments_after=comments_after,
            pull_request_before=current_pr,
            pull_request_after=pull_request_after_probe,
        )

        git_write_probe_summary = None
        git_write_control_summary = None
        if args.probe_read_only_git_write:
            stage = "probe_read_only_git_receive_pack_write_refusal"
            git_probe_summaries = run_read_only_git_receive_pack_probe(
                state=state,
                evidence=evidence,
                repo_path=repo_path,
                api=api,
                reviewer_token=reviewer_token,
                writer_token=token,
                server_log_path=server_log_path,
            )
            git_write_probe_summary = git_probe_summaries["read_only"]
            git_write_control_summary = git_probe_summaries["write_enabled_control"]

        stage = "read_only_rust_reconciliation"
        journal_before = journal_path.read_bytes()
        server_log_before = server_log_path.read_bytes() if args.probe_pr_comment_batch else b""
        if args.interrupt_cli_between_parent_and_comment:
            if api_proxy is None or comment_event_summary is None or comment_id is None:
                raise RuntimeError("local API proxy or PR/comment bindings are unavailable for the interrupt probe")
            parent_get_path = f"/api/v1/repos/{ORGANIZATION}/{REPOSITORY}/pulls/{pr_number}"
            comment_get_path = f"/api/v1/repos/{ORGANIZATION}/{REPOSITORY}/issues/comments/{comment_id}"
            api_proxy.set_reader_token(reviewer_token)
            api_proxy.set_allowed_read_routes(
                {("GET", parent_get_path), ("GET", comment_get_path)}
            )

            gate_probe_start = len(api_proxy.snapshot())
            gate_probe_log_before = server_log_path.read_bytes()
            gate_probe_connection = http.client.HTTPConnection("127.0.0.1", 3130, timeout=5)
            try:
                gate_probe_connection.request(
                    "POST",
                    comment_get_path,
                    body=b"{}",
                    headers={
                        "Authorization": f"token {reviewer_token}",
                        "Content-Type": "application/json",
                        "Content-Length": "2",
                    },
                )
                gate_probe_response = gate_probe_connection.getresponse()
                gate_probe_status = gate_probe_response.status
                gate_probe_response.read(1024)
            finally:
                gate_probe_connection.close()
            gate_probe_records = api_proxy.snapshot()[gate_probe_start:]
            gate_probe_log_after = server_log_path.read_bytes()
            if not gate_probe_log_after.startswith(gate_probe_log_before):
                raise RuntimeError("Forgejo console log changed non-append-only during the proxy gate probe")
            gate_probe_log_delta = gate_probe_log_after[len(gate_probe_log_before):].decode(
                "utf-8", errors="replace"
            )
            forbidden_post_upstream_hits = [
                line for line in gate_probe_log_delta.splitlines()
                if f" completed POST {comment_get_path} " in line
            ]
            if (
                len(gate_probe_records) != 1
                or gate_probe_records[0].get("method") != "POST"
                or gate_probe_records[0].get("path") != comment_get_path
                or gate_probe_records[0].get("reader_credential_match") is not True
                or gate_probe_records[0].get("status") != 403
                or gate_probe_records[0].get("forwarded") is not False
                or gate_probe_records[0].get("barrier") != "not_allowlisted"
                or gate_probe_status != 403
                or forbidden_post_upstream_hits
            ):
                raise RuntimeError("local reconciliation API gate did not refuse the unlisted POST before Forgejo")

            barrier = native_replay.OneShotApiGetBarrier(comment_get_path, timeout=30)
            api_proxy.set_request_barrier(barrier)
            proxy_before_attempt = api_proxy.snapshot()
            server_log_before_attempt = server_log_path.read_bytes()
            cli_process = None
            killed_process = None
            try:
                cli_process = start_reconciliation_cli(
                    reconcile_bin,
                    state,
                    journal_path,
                    [event_summary["delivery_id"], comment_event_summary["delivery_id"]],
                    reviewer_token,
                )
                if not barrier.wait_until_reached(15):
                    raise RuntimeError("reconciliation CLI did not reach the comment GET barrier")
                if cli_process.poll() is not None:
                    raise RuntimeError("reconciliation CLI exited while its comment GET was held")
                killed_process = kill_reconciliation_process_group(cli_process, reviewer_token)
                if not barrier.release(forward=False):
                    raise RuntimeError("comment GET barrier could not drop the cancelled first request")
                if not barrier.wait_until_finished(5):
                    raise RuntimeError("comment GET barrier did not finish after the test released it")
            finally:
                if cli_process is not None and cli_process.poll() is None:
                    try:
                        os.killpg(cli_process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    cli_process.wait(timeout=10)
                    cli_process.communicate(timeout=5)
                if barrier.snapshot().get("reached") is True and barrier.snapshot().get("released") is not True:
                    barrier.release(forward=False)
                    barrier.wait_until_finished(5)

            journal_after_kill = journal_path.read_bytes()
            server_log_after_kill = server_log_path.read_bytes()
            if not server_log_after_kill.startswith(server_log_before_attempt):
                raise RuntimeError("Forgejo log changed non-append-only during the interrupted API read")
            interrupted_cli_summary = validate_interrupted_cli_attempt(
                proxy_records=api_proxy.snapshot()[len(proxy_before_attempt):],
                parent_get_path=parent_get_path,
                comment_get_path=comment_get_path,
                server_log_delta=server_log_after_kill[
                    len(server_log_before_attempt):
                ].decode("utf-8", errors="replace"),
                journal_before=journal_before,
                journal_after=journal_after_kill,
                barrier=barrier.snapshot(),
                killed_process=killed_process,
                forbidden_route_record=gate_probe_records[0],
                forbidden_status=gate_probe_status,
                forbidden_post_upstream_hits=forbidden_post_upstream_hits,
            )
            interrupted_cli_summary["gate_probe_log_unchanged"] = (
                not forbidden_post_upstream_hits
            )

            proxy_before_resume = api_proxy.snapshot()
            server_log_before = server_log_path.read_bytes()
            reconcile_status, reconciled = fixture.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                [event_summary["delivery_id"], comment_event_summary["delivery_id"]],
                reviewer_token,
                target_profile="org-team-review-e2e",
            )
            journal_after = journal_path.read_bytes()
            server_log_after_resume = server_log_path.read_bytes()
            if not server_log_after_resume.startswith(server_log_before):
                raise RuntimeError("Forgejo log changed non-append-only during resumed reconciliation")
            resumed_proxy_records = api_proxy.snapshot()[len(proxy_before_resume):]
            interrupted_cli_summary["resume"] = validate_interrupted_cli_resume(
                proxy_records=resumed_proxy_records,
                parent_get_path=parent_get_path,
                comment_get_path=comment_get_path,
                reconciled=reconciled,
                journal_before=journal_before,
                journal_after=journal_after,
            )
            interrupted_cli_summary["resume_proxy_records"] = resumed_proxy_records
            write_private_json(
                evidence / "interrupted-cli-resume.json", interrupted_cli_summary, tokens
            )
        else:
            reconcile_status, reconciled = fixture.run_reconcile(
                reconcile_bin,
                state,
                journal_path,
                [event_summary["delivery_id"], comment_event_summary["delivery_id"]]
                if args.probe_pr_comment_batch and comment_event_summary is not None
                else event_summary["delivery_id"],
                reviewer_token,
                target_profile="org-team-review-e2e",
            )
            journal_after = journal_path.read_bytes()
        server_log_after = server_log_path.read_bytes() if args.probe_pr_comment_batch else b""
        journal_after = journal_path.read_bytes()
        server_log_after = server_log_path.read_bytes() if args.probe_pr_comment_batch else b""
        if args.probe_pr_comment_batch and not server_log_after.startswith(server_log_before):
            raise RuntimeError("Forgejo console log changed before the current reconciliation read window")
        server_log_delta = (
            server_log_after[len(server_log_before):].decode("utf-8", errors="replace")
            if args.probe_pr_comment_batch else ""
        )
        if args.probe_pr_comment_batch:
            if comment_event_summary is None or comment_id is None:
                raise RuntimeError("Forgejo PR/comment batch setup did not retain both event bindings")
            pr_comment_batch_summary = validate_pr_comment_batch_result(
                reconciled,
                pull_delivery_id=event_summary["delivery_id"],
                comment_delivery_id=comment_event_summary["delivery_id"],
                owner=ORGANIZATION,
                repository=REPOSITORY,
                pull_request_number=pr_number,
                comment_id=comment_id,
                expected_head_sha=current_pr_head_sha,
                expected_base_sha=current_pr_base_sha,
                expected_team_id=team_id,
                expected_team_name=TEAM,
                expected_comment_body=PR_COMMENT_BODY,
            )
            pr_comment_batch_summary.update(
                validate_pr_comment_batch_api_reads(
                    server_log_delta,
                    owner=ORGANIZATION,
                    repository=REPOSITORY,
                    pull_request_number=pr_number,
                    comment_id=comment_id,
                )
            )
            representative = reconciled["results"][0]
            canonical = representative.get("canonical_state")
            resource = representative.get("resource")
        else:
            canonical = reconciled.get("canonical_state")
            resource = reconciled.get("resource")
        expected_team_projection = [{"id": team_id, "name": TEAM}]
        if (
            reconcile_status != 0
            or reconciled.get("verdict") != "current_state_read"
            or (
                reconciled.get("delivery_id") != event_summary["delivery_id"]
                if not args.probe_pr_comment_batch
                else len(reconciled.get("results", [])) != 2
            )
            or not isinstance(resource, dict)
            or resource.get("kind") != "pull_request"
            or resource.get("owner") != ORGANIZATION
            or resource.get("repository") != REPOSITORY
            or resource.get("number") != pr_number
            or not isinstance(canonical, dict)
            or canonical.get("state") != "open"
            or canonical.get("merged") is not False
            or canonical.get("requested_reviewers_teams") != expected_team_projection
            or canonical.get("base_sha") != event_summary["base_sha"]
            or canonical.get("head_sha") != event_summary["head_sha"]
            or journal_after != journal_before
        ):
            raise RuntimeError("Rust reconciliation did not read the exact current team-review PR state")
        result: dict[str, Any] = {
            "schema_version": 1,
            "verdict": "passed",
            "scope": "forgejo_live_webhook_receiver_and_rust_read_only_reconciliation_cli",
            "forgejo": {
                "version": version.stdout.strip(),
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "commit": "unknown",
            },
            "organization": {
                "id": org_id,
                "name": ORGANIZATION,
                "visibility": org.get("visibility"),
            },
            "team": {
                "id": team_id,
                "name": TEAM,
                "includes_all_repositories": True,
                "reviewer_member_added": True,
            },
            "repository": {
                "full_name": f"{ORGANIZATION}/{REPOSITORY}",
                "private": True,
            },
            "pull_request": {
                "number": pr_number,
                "state": current_pr.get("state"),
                "merged": current_pr.get("merged"),
            },
            "team_review_request": {
                "http_status": request_status,
                "readback": projection,
            },
            "read_only_write_probe": write_probe_summary,
            "read_only_git_receive_pack_probe": git_write_probe_summary,
            "write_credential_receive_pack_control": git_write_control_summary,
            "webhook": {
                "hook_id": hook_id,
                "active_loopback_target_readback": True,
                "receiver_invoked": True,
                "journal_record": event_summary,
            },
            "rust_reconciliation": {
                "invoked": True,
                "exit_status": reconcile_status,
                "verdict": reconciled.get("verdict"),
                "resource": resource,
                "canonical_state": canonical,
                "read_only_credential_scopes": ["read:user", "read:repository", "read:issue"],
                "inbox_unchanged_by_read": journal_after == journal_before,
            },
            "llm_or_agent_invoked": False,
        }
        if args.probe_pr_comment_batch:
            result["webhook"]["issue_comment_journal_record"] = comment_event_summary
            result["pr_comment_batch"] = pr_comment_batch_summary
        if args.kill_receiver_before_reconcile:
            result["receiver_restart"] = receiver_restart_summary
        if args.interrupt_cli_between_parent_and_comment:
            result["interrupted_cli_resume"] = interrupted_cli_summary
        if api_proxy is not None:
            result["api_proxy"] = {
                "listen": "127.0.0.1:3130",
                "upstream": "127.0.0.1:3131",
                "record_count": len(api_proxy.snapshot()),
                "gate_enforced": args.interrupt_cli_between_parent_and_comment,
            }
        return result
    finally:
        if receiver is not None:
            receiver_stopped = fixture.stop_process(receiver)
        if receiver_log is not None:
            receiver_log.close()
        if server is not None:
            stopped = fixture.stop_process(server)
        server_log.close()
        forgejo_listener_addresses = fixture.listener_addresses_for_port(3131)
        receiver_listener_addresses = (
            fixture.listener_addresses_for_port(receiver_port) if receiver_port is not None else []
        )
        if api_proxy is not None:
            api_proxy_stopped = api_proxy.stop()
        proxy_listener_addresses = fixture.listener_addresses_for_port(3130)
        if forgejo_listener_addresses or receiver_listener_addresses or proxy_listener_addresses:
            raise RuntimeError("Forgejo, API proxy, or webhook receiver listener remained after cleanup")
        if stopped is not None and stopped.get("stopped") is not True:
            raise RuntimeError("Forgejo process cleanup was not confirmed")
        if receiver_stopped is not None and receiver_stopped.get("stopped") is not True:
            raise RuntimeError("receiver process cleanup was not confirmed")
        if api_proxy_stopped is not None and api_proxy_stopped.get("stopped") is not True:
            raise RuntimeError("local API proxy cleanup was not confirmed")
        write_private_json(
            evidence / "cleanup.json",
            {
                "server": stopped,
                "receiver": receiver_stopped,
                "api_proxy": api_proxy_stopped,
                "listeners_on_3130_after_stop": proxy_listener_addresses,
                "forgejo_listeners_on_3131_after_stop": forgejo_listener_addresses,
                "listeners_on_receiver_port_after_stop": receiver_listener_addresses,
            },
            tokens,
        )
        if api_proxy is not None:
            write_private_json(
                evidence / "api-proxy-requests.json",
                {
                    "schema_version": 1,
                    "listen": "127.0.0.1:3130",
                    "fixed_upstream": "127.0.0.1:3131",
                    "records": api_proxy.snapshot(),
                },
                tokens,
            )
        write_private_json(
            evidence / "environment.json",
            {
                "python": sys.version.split()[0],
                "platform": sys.platform,
                "state_dir": str(state),
                "state_dir_mode": "0700",
                "stage": stage,
                "forgejo_access_log_mode": (
                    "console_info" if args.probe_read_only_git_write or args.probe_pr_comment_batch else "file_warn"
                ),
                "receiver_kill_before_reconcile": args.kill_receiver_before_reconcile,
                "interrupt_cli_between_parent_and_comment": args.interrupt_cli_between_parent_and_comment,
                "forgejo_port": 3131,
                "local_api_proxy_port": 3130,
            },
            tokens,
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forgejo-binary", required=True)
    parser.add_argument("--state-dir", required=True)
    parser.add_argument("--evidence-dir", required=True)
    parser.add_argument("--probe-read-only-git-write", action="store_true")
    parser.add_argument("--probe-pr-comment-batch", action="store_true")
    parser.add_argument("--kill-receiver-before-reconcile", action="store_true")
    parser.add_argument("--interrupt-cli-between-parent-and-comment", action="store_true")
    args = parser.parse_args()
    result = run(args)
    write_private_json(Path(args.evidence_dir) / "result.json", result)
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
